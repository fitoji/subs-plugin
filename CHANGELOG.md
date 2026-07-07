# Changelog

All notable changes to Subtitle Overlay are documented here.

---

## [0.2.2] — 2026-07-07

### Added
- **Native macOS menu** — Whisper settings accessible via app menu bar (Model, Language, Temperature, Beam Size, thresholds)
- **Settings persistence** — parameters saved as TOML to `~/Library/Application Support/subtitle-overlay/settings.user`
- **Reload Whisper** — kills and re-spawns the Python sidecar with current settings; audio capture (SCStream) stays alive
- **`--config` CLI arg** — Python sidecar accepts `--config <json>` to override transcription parameters at startup
- **Reloading status UI** — overlay shows "Reloading Whisper…" centered during sidecar restart
- **Reset to Defaults** — restores all Whisper parameters to their safe defaults

### Changed
- **Sidecar re-spawn** — `spawn_sidecar()` now accepts optional `config_path: Option<&Path>` for `--config` argument
- **Frontend status** — added `'reloading'` to the `stt_status` union type

## [0.2.1] — 2026-07-06

### Added
- **Bilingual initial_prompt** — primes Whisper tokenizer for correct capitalization and vocabulary in English and German
- **Language auto-detect** — no longer forced to Spanish; works for EN, DE, ES, FR, etc.
- **Model download progress bar** — shows percentage in the status bar during first-time model download via custom tqdm class
- **`hallucination_silence_threshold`** — suppresses the "text during silence" Whisper bug (2.0 s threshold)
- **`word_timestamps`** — enables word-level timestamps in Whisper output
- **Temperature fallback** — `(0.0, 0.2, 0.4)` fallback chain for robust transcription on low-confidence audio
- **5 s audio chunks** — increased from 2 s for better Whisper phonetic context (trade-off: ~3 s additional latency)
- **DC-removal + 80 Hz high-pass filter** — removes DC offset and low-frequency rumble (HVAC, traffic, handling noise) before VAD
- **`clip_timestamps` streaming** — each region of audio decoded exactly ONCE instead of re-transcribing the entire buffer; eliminates word duplication and timestamp-shift bugs
- **Cross-chunk context** — last ~200 characters of transcript fed as `initial_prompt` to compensate for `condition_on_previous_text=False`
- **Silence region skipping** — `_COMMITTED_END_S` advances past audio regions where Whisper produces no words, preventing re-processing
- **Dark background during model loading** — no more white screen while model downloads
- **Centered loading message** — shows during model download/loading
- **`statusMessage` in Zustand store** — displays non-error status messages (loading, progress) in the status bar
- **Copy button** — clipboard SVG icon for copying transcript
- **Text selection** — transcript text is selectable with mouse
- **Resizable window** — overlay window supports resize

### Changed
- **Model:** `whisper-base` → `whisper-large-v3-turbo` (best quality/speed trade-off on Apple Silicon)
- **Compression ratio threshold:** `2.0` → `2.4` (default, better for German compound words)
- **Logprob threshold:** `-1.0` → `-0.5` (stricter rejection of noisy segments)
- **No-speech threshold:** `0.6` → `0.35` (more aggressive silence filtering)
- **Chunk/overlap:** `2 s / 0.5 s` → `5 s / 1 s` (Rust `AudioConfig`)

### Fixed
- **CRITICAL:** Word-timestamp-based dedup was broken — timestamps shift by 2+ seconds when re-transcribing the same audio in a longer buffer. Replaced with `clip_timestamps` decoder-only-once approach.
- **CRITICAL:** Model repo `mlx-community/whisper-base` doesn't exist → renamed to `mlx-community/whisper-base-mlx`, then upgraded to `whisper-large-v3-turbo`
- **CRITICAL:** `mlx-community/whisper-large-v3-turbo-asr-fp16` incompatible with `mlx_whisper` 0.4.3 (config has `activation_dropout` field)
- White screen during model loading (background now always applied in chat mode)
- Sidecar crash on missing `mlx-whisper` in venv
- `isTranscribing` gate prevents status text from showing after transcription starts
- Overlap/filter dedup using `_COMMITTED_SAMPLES` now correctly replaced by `_COMMITTED_END_S`

### Documentation
- `docs/whisper-fidelity-research.md` — comprehensive analysis of 10 fidelity gaps with impact estimates
- `docs/plan-whisper-fidelity-v0.3.md` — 4-phase implementation plan with risk assessment and acceptance tests
- `docs/decisions/0002-*` — trade-off latencia vs. fidelidad
