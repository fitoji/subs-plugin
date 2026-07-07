#!/usr/bin/env python3
"""
Whisper MLX sidecar — Real-time speech-to-text via Apple MLX.

Protocol: JSON Lines over stdio.

Rust → Python (stdin):
  {"type":"audio","data":"<base64>","sample_rate":16000}
  {"type":"reset"}
  {"type":"shutdown"}

Python → Rust (stdout):
  {"type":"status","state":"loading|ready|error","message":"..."}
  {"type":"transcription","text":"...","is_final":false,"timestamp":<ms>}
  {"type":"transcription","text":"...","is_final":true,"timestamp":<ms>}

Exit codes:
  0 — clean shutdown
  1 — unexpected error
  2 — model download failure

Requirements: mlx-whisper, numpy
"""

import argparse
import json
import os
import sys
import time
import base64
import traceback
from typing import Optional

import numpy as np
import tqdm  # for custom download progress bar


# ---------------------------------------------------------------------------
# CLI argument parsing
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    """Parse CLI arguments. Currently supports ``--config`` for JSON overrides."""
    parser = argparse.ArgumentParser(
        description="Whisper MLX sidecar — real-time speech-to-text via Apple MLX."
    )
    parser.add_argument(
        "--config",
        type=str,
        help="Path to a JSON config file whose keys override the hardcoded "
             "_TRANSCRIBE_KWARGS defaults. See the JSON format documented in "
             "_load_config().",
    )
    return parser.parse_args()


def load_config(path: str) -> dict:
    """Read and validate a JSON config file.

    The expected JSON format::

        {
            "temperature": [0.0, 0.2, 0.4],
            "beam_size": 5,
            "language": "auto",
            "model": "mlx-community/whisper-large-v3-turbo",
            "no_speech_threshold": 0.35,
            "compression_ratio_threshold": 2.4,
            "logprob_threshold": -0.5
        }

    Returns the parsed dict. The caller merges values via ``dict.update()``
    so missing keys preserve the hardcoded default.

    Raises ``FileNotFoundError`` if the path does not exist.
    """
    if not os.path.isfile(path):
        raise FileNotFoundError(f"Config file not found: {path}")

    with open(path, "r", encoding="utf-8") as f:
        cfg = json.load(f)

    # Convert temperature array to tuple if present
    if "temperature" in cfg and isinstance(cfg["temperature"], list):
        cfg["temperature"] = tuple(cfg["temperature"])

    return cfg


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

# ── Model config ──────────────────────────────────────────────────────────
#   tiny           → fastest, least accurate
#   base-mlx       → good balance speed/accuracy on Apple Silicon
#   small          → better quality, slightly slower
#   medium         → great quality, needs ~5 GB RAM
#   large-v3-mlx   → best quality (non-turbo), ~3 GB RAM, 3x slower
#   large-v3-turbo → best quality/speed trade-off (current)
#   large-v3-turbo-8bit → faster, less RAM, slight quality loss
#   large-v3-turbo-asr-fp16 → INCOMPATIBLE with mlx-whisper 0.4.3
WHISPER_MODEL = "mlx-community/whisper-large-v3-turbo"
SAMPLE_RATE = 16000
OVERLAP_S = 1.0  # overlap between consecutive chunks (matches Rust AudioConfig)

# Streaming state: accumulate audio and track which parts are transcribed
_CONTEXT_BUFFER: list[float] = []  # mono float samples

# The fundamental approach for streaming Whisper is:
#   1. Keep audio accumulating in a buffer (bounded at ~30 s = Whisper's native window).
#   2. On each call, transcribe the FULL buffer (so the encoder has full acoustic
#      context), but use ``clip_timestamps`` to DECODE ONLY the new portion.
#   3. Since each region of audio is decoded exactly ONCE, word timestamps are
#      stable and we never re-decode the same audio — eliminating the duplication
#      and timestamp-shift problems that plagued the earlier implementation.
#   4. ``initial_prompt`` carries the last ~200 characters of transcribed text
#      as context, compensating for ``condition_on_previous_text=False``.

_COMMITTED_END_S: float = 0.0  # seconds from buffer start — last decoded time

# Last ~200 characters of transcribed text, used to condition the decoder
# across chunks (compensates for ``condition_on_previous_text=False``).
_LAST_TEXT: str = ""

# Original bilingual prompt — kept separate so we can prepend it on every call.
_INITIAL_PROMPT: str = (
    "Hello, how are you? Guten Tag, wie geht es Ihnen? "
    "The weather is nice. Das Wetter ist schön."
)

_MODEL = None  # lazy-loaded whisper model
_TRANSCRIBE_KWARGS: dict = {}  # filled by load_model(), mutated per-call


# ---------------------------------------------------------------------------
# JSON Lines helpers
# ---------------------------------------------------------------------------

def send(obj: dict) -> None:
    """Write a JSON Line to stdout and flush immediately."""
    sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def read_line() -> Optional[dict]:
    """Read one JSON Line from stdin. Returns None on EOF."""
    line = sys.stdin.readline()
    if not line:
        return None
    line = line.strip()
    if not line:
        return None
    return json.loads(line)


# ---------------------------------------------------------------------------
# Audio processing
# ---------------------------------------------------------------------------

def decode_audio(data_b64: str, expected_sr: int) -> np.ndarray:
    """Decode base64 PCM i16 → numpy float32 array, normalized to [-1, 1]."""
    raw = base64.b64decode(data_b64)
    i16 = np.frombuffer(raw, dtype=np.int16).astype(np.float32)
    i16 /= 32768.0

    # resample if sample rate differs (simple linear interpolation)
    actual_sr = expected_sr  # Rust always sends 16kHz per spec
    if actual_sr != SAMPLE_RATE:
        ratio = SAMPLE_RATE / actual_sr
        target_len = int(len(i16) * ratio)
        indices = np.linspace(0, len(i16) - 1, target_len)
        i16 = np.interp(indices, np.arange(len(i16)), i16)

    return i16


def append_to_context(chunk: np.ndarray) -> None:
    """Append new samples to the context buffer."""
    global _CONTEXT_BUFFER
    _CONTEXT_BUFFER.extend(chunk.tolist())


def trim_context() -> None:
    """Keep buffer bounded at 30 s (Whisper's native window).

    Adjusts ``_COMMITTED_END_S`` so it remains a valid position within the
    (smaller) buffer.
    """
    global _CONTEXT_BUFFER, _COMMITTED_END_S
    MAX_SAMPLES = SAMPLE_RATE * 30
    if len(_CONTEXT_BUFFER) > MAX_SAMPLES:
        trim_count = len(_CONTEXT_BUFFER) - MAX_SAMPLES
        trimmed_s = trim_count / SAMPLE_RATE
        _CONTEXT_BUFFER = _CONTEXT_BUFFER[trim_count:]
        _COMMITTED_END_S = max(0.0, _COMMITTED_END_S - trimmed_s)


# ---------------------------------------------------------------------------
# Whisper transcription
# ---------------------------------------------------------------------------

class _ProgressTqdm(tqdm.tqdm):  # type: ignore[name-defined]
    """Custom tqdm that emits download percentage to the sidecar channel.

    Pass as ``tqdm_class`` to ``snapshot_download`` or ``hf_hub_download``.
    """

    def __init__(self, *args, **kwargs) -> None:
        super().__init__(*args, **kwargs)
        self._last_pct = -1

    def update(self, n: int = 1) -> bool | None:
        result = super().update(n)
        if self.total and self.total > 0:
            pct = int(self.n / self.total * 100)
            if pct != self._last_pct:
                self._last_pct = pct
                send({"type": "status", "state": "loading", "message": f"Downloading model… {pct}%"})
        return result


def _download_model(repo_id: str) -> str:
    """Download the model from HuggingFace with progress reporting.

    Returns the local cache path. If already cached, returns immediately
    without emitting progress events.
    """
    from huggingface_hub import snapshot_download, constants
    from huggingface_hub.utils import HfHubHTTPError
    import os, pathlib

    # Check if already cached — skip progress bar if so
    cache_dir = pathlib.Path(constants.HF_HUB_CACHE)
    # repo_id "org/name" → "models--org--name"
    cache_name = "models--" + repo_id.replace("/", "--")
    snapshot_dir = cache_dir / cache_name / "snapshots"
    if snapshot_dir.is_dir() and any(snapshot_dir.iterdir()):
        # Cached — download silently, no progress events
        return snapshot_download(repo_id=repo_id)

    send({"type": "status", "state": "loading", "message": "Downloading model…"})
    try:
        return snapshot_download(repo_id=repo_id, tqdm_class=_ProgressTqdm)
    except HfHubHTTPError as exc:
        send({"type": "status", "state": "error", "message": f"Model download failed: {exc}"})
        sys.exit(2)


def load_model(config_overrides: dict | None = None) -> None:
    """Load the Whisper model (lazy, called on first audio).

    Args:
        config_overrides: Optional dict of values from a ``--config`` JSON file.
                          Each key is merged into ``_TRANSCRIBE_KWARGS`` via
                          ``.update()`` so missing keys preserve hardcoded defaults.
    """
    global _MODEL, _TRANSCRIBE_KWARGS
    if _MODEL is not None:
        return

    send({"type": "status", "state": "loading", "message": f"Loading Whisper {WHISPER_MODEL} model…"})
    try:
        # Step 1: pre-download the model with progress tracking
        _download_model(WHISPER_MODEL)

        # Step 2: import and configure
        import mlx_whisper

        # Transcription parameters tuned for maximum fidelity (EN + DE).
        # ``initial_prompt`` and ``clip_timestamps`` are mutated per-call
        # in ``transcribe_buffer()`` — see that function for the streaming
        # architecture.
        _TRANSCRIBE_KWARGS = {
            "path_or_hf_repo": WHISPER_MODEL,
            # bilingual initial_prompt primes the tokenizer for correct
            # capitalization and vocabulary in English and German
            "initial_prompt": (
                "Hello, how are you? Guten Tag, wie geht es Ihnen? "
                "The weather is nice. Das Wetter ist schön."
            ),
            # 3-step fallback: greedy first, warmer on low confidence
            "temperature": (0.0, 0.2, 0.4),
            "condition_on_previous_text": False,       # Avoid repetition loops in streaming mode
            "no_speech_threshold": 0.35,               # More aggressive at filtering silence noise
            # 2.4 is the Whisper default — 2.0 was too strict for
            # German compound words (e.g. Donaudampfschifffahrt)
            "compression_ratio_threshold": 2.4,
            "logprob_threshold": -0.5,                 # Reject low-probability (noisy) segments
            # Kills the "text during silence" hallucination bug
            "hallucination_silence_threshold": 2.0,
            # Needed for Phase 2 dedup — will filter by word start times
            "word_timestamps": True,
        }

        # Apply ``--config`` overrides on top of hardcoded defaults so that
        # missing JSON keys preserve the default values.
        if config_overrides:
            _TRANSCRIBE_KWARGS.update(config_overrides)

        _MODEL = lambda audio: mlx_whisper.transcribe(
            np.array(audio, dtype=np.float32),
            **_TRANSCRIBE_KWARGS,
        )
        send({"type": "status", "state": "ready"})
    except Exception as exc:
        err_msg = str(exc)
        if "download" in err_msg.lower() or "connection" in err_msg.lower():
            send({"type": "status", "state": "error", "message": f"Model download failed: {err_msg}"})
            sys.exit(2)
        raise


def transcribe_buffer() -> None:
    """Transcribe only the NEW portion of the buffer using ``clip_timestamps``.

    The full buffer is passed to the encoder (full acoustic context), but the
    decoder starts at ``_COMMITTED_END_S`` and skips already-processed audio.
    Each audio region is decoded exactly ONCE, so word timestamps are stable.

    ``_LAST_TEXT`` is fed as ``initial_prompt`` to give the model cross-chunk
    linguistic context (compensates for ``condition_on_previous_text=False``).
    """
    global _COMMITTED_END_S, _LAST_TEXT, _TRANSCRIBE_KWARGS

    if len(_CONTEXT_BUFFER) < int(SAMPLE_RATE * 0.5):  # need at least 0.5 s
        return

    buffer_duration = len(_CONTEXT_BUFFER) / SAMPLE_RATE

    # Nothing new to decode yet
    if _COMMITTED_END_S >= buffer_duration - 0.15:
        return

    audio_array = np.array(_CONTEXT_BUFFER, dtype=np.float32)

    # Tell Whisper to decode only the unprocessed region
    _TRANSCRIBE_KWARGS["clip_timestamps"] = [_COMMITTED_END_S, buffer_duration + 1.0]

    # Give the model linguistic context from the last transcription
    if _LAST_TEXT:
        _TRANSCRIBE_KWARGS["initial_prompt"] = f"{_INITIAL_PROMPT}\n{_LAST_TEXT}"
    else:
        _TRANSCRIBE_KWARGS["initial_prompt"] = _INITIAL_PROMPT

    result = _MODEL(audio_array)

    # Collect new words (all in the decoded region are genuinely new)
    words: list[dict[str, float | str]] = []
    for seg in result.get("segments", []):
        for w in seg.get("words", []):
            text: str = w.get("word", "").strip()
            if text:
                words.append({
                    "word": text,
                    "start": w.get("start", 0.0),
                    "end": w.get("end", 0.0),
                })

    if not words:
        # No speech detected in this region — advance past it
        _COMMITTED_END_S = buffer_duration
        return

    # Advance the committed end
    new_end = max(w["end"] for w in words)  # type: ignore[type-var]
    if new_end > _COMMITTED_END_S:
        _COMMITTED_END_S = new_end

    new_text = " ".join(str(w["word"]) for w in words)

    # Keep last ~200 chars for context conditioning
    _LAST_TEXT = new_text[-200:]

    send({
        "type": "transcription",
        "text": new_text,
        "is_final": True,
        "timestamp": int(time.time() * 1000),
    })

    trim_context()


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------

def main() -> None:
    args = parse_args()

    # Load optional config overrides from ``--config`` file
    config_overrides: dict | None = None
    if args.config:
        config_overrides = load_config(args.config)

    send({"type": "status", "state": "loading", "message": "Initializing Whisper MLX sidecar…"})
    load_model(config_overrides)

    while True:
        msg = read_line()
        if msg is None:
            break  # EOF — stdin closed

        msg_type = msg.get("type")

        if msg_type == "shutdown":
            break

        elif msg_type == "reset":
            global _CONTEXT_BUFFER, _COMMITTED_END_S, _LAST_TEXT
            _CONTEXT_BUFFER = []
            _COMMITTED_END_S = 0.0
            _LAST_TEXT = ""
            send({"type": "status", "state": "ready", "message": "Context reset"})

        elif msg_type == "audio":
            data_b64 = msg["data"]
            sr = msg.get("sample_rate", 16000)

            chunk = decode_audio(data_b64, sr)
            append_to_context(chunk)
            transcribe_buffer()

        else:
            # Unknown message type — ignore
            pass

    send({"type": "status", "state": "ready", "message": "Shutting down"})


if __name__ == "__main__":
    try:
        main()
        sys.exit(0)
    except Exception:
        send({"type": "status", "state": "error", "message": traceback.format_exc()})
        sys.exit(1)
