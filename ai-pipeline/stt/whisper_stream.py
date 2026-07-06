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

import json
import sys
import time
import base64
import traceback
from typing import Optional

import numpy as np


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

# ── Model config ──────────────────────────────────────────────────────────
#   tiny   → fastest, least accurate
#   base   → good balance speed/accuracy on Apple Silicon
#   small  → better quality, slightly slower
#   medium → great quality, needs ~5 GB RAM
#   large  → best quality, needs ~10 GB RAM
WHISPER_MODEL = "mlx-community/whisper-base"
CHUNK_DURATION_S = 2.0  # expected audio chunk duration
SAMPLE_RATE = 16000
OVERLAP_S = 0.5  # overlap between consecutive chunks

# Streaming state: accumulate audio and track which parts are transcribed
_CONTEXT_BUFFER: list[float] = []  # mono float samples
_COMMITTED_SAMPLES = 0  # number of samples already committed as final text
_MODEL = None  # lazy-loaded whisper model


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
    """Drop committed samples from the context buffer to avoid unbounded growth."""
    global _CONTEXT_BUFFER, _COMMITTED_SAMPLES
    if _COMMITTED_SAMPLES > 0:
        if _COMMITTED_SAMPLES >= len(_CONTEXT_BUFFER):
            _CONTEXT_BUFFER = []
        else:
            _CONTEXT_BUFFER = _CONTEXT_BUFFER[_COMMITTED_SAMPLES:]
        _COMMITTED_SAMPLES = 0


# ---------------------------------------------------------------------------
# Whisper transcription
# ---------------------------------------------------------------------------

def _download_model(repo_id: str) -> str:
    """Download the model from HuggingFace with progress reporting.

    Returns the local cache path. If already cached, returns immediately.
    """
    from huggingface_hub import snapshot_download
    from huggingface_hub.utils import HfHubHTTPError

    last_pct = -1

    def _on_progress(current: int, total: int, status: str) -> None:
        nonlocal last_pct
        if total > 0:
            pct = int(current / total * 100)
            if pct != last_pct:  # avoid flooding stdout
                last_pct = pct
                send({"type": "status", "state": "loading", "message": f"Downloading model… {pct}%"})

    send({"type": "status", "state": "loading", "message": "Downloading model…"})
    try:
        return snapshot_download(repo_id=repo_id, callback=_on_progress)
    except HfHubHTTPError as exc:
        send({"type": "status", "state": "error", "message": f"Model download failed: {exc}"})
        sys.exit(2)


def load_model() -> None:
    """Load the Whisper model (lazy, called on first audio)."""
    global _MODEL
    if _MODEL is not None:
        return

    send({"type": "status", "state": "loading", "message": f"Loading Whisper {WHISPER_MODEL} model…"})
    try:
        # Step 1: pre-download the model with progress tracking
        _download_model(WHISPER_MODEL)

        # Step 2: import and configure
        import mlx_whisper

        # Transcription parameters: no forced language → Whisper auto-detects
        # (English, Spanish, German, French, etc. — works for any language
        #  the multilingual model supports).
        _TRANSCRIBE_KWARGS = {
            "path_or_hf_repo": WHISPER_MODEL,
            "temperature": 0.0,                        # Deterministic — fewer hallucinations
            "condition_on_previous_text": False,       # Avoid repetition loops in streaming mode
            "no_speech_threshold": 0.35,               # More aggressive at filtering silence noise
            "compression_ratio_threshold": 2.0,        # Slightly stricter on repetitive text
            "logprob_threshold": -0.5,                 # Reject low-probability (noisy) segments
        }

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
    """Transcribe the uncommitted portion of the context buffer."""
    global _COMMITTED_SAMPLES

    if len(_CONTEXT_BUFFER) < SAMPLE_RATE * 0.5:  # need at least 0.5s
        return

    audio_array = np.array(_CONTEXT_BUFFER, dtype=np.float32)
    result = _MODEL(audio_array)  # already set by load_model

    # Combine all segment texts
    full_text = " ".join(seg["text"].strip() for seg in result.get("segments", []) if seg.get("text", "").strip())

    if not full_text:
        return

    # Send partial if we have more context than committed
    samples_covered = int(len(audio_array))
    _COMMITTED_SAMPLES = samples_covered

    # We send is_final: true since we transcribe the whole buffer each time
    # In v0.2 this is sufficient. Future versions can implement proper
    # word-level streaming with partials.
    send({
        "type": "transcription",
        "text": full_text,
        "is_final": True,
        "timestamp": int(time.time() * 1000),
    })

    trim_context()


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------

def main() -> None:
    send({"type": "status", "state": "loading", "message": "Initializing Whisper MLX sidecar…"})
    load_model()

    while True:
        msg = read_line()
        if msg is None:
            break  # EOF — stdin closed

        msg_type = msg.get("type")

        if msg_type == "shutdown":
            break

        elif msg_type == "reset":
            global _CONTEXT_BUFFER, _COMMITTED_SAMPLES
            _CONTEXT_BUFFER = []
            _COMMITTED_SAMPLES = 0
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
