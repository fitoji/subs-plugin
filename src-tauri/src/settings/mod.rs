//! Whisper transcription settings — persisted as TOML on disk.
//!
//! Atomic write (`.tmp` → rename), per-field `#[serde(default)]` for
//! corrupt-file recovery, and a `load_or_default()` convenience for
//! startup paths.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Struct
// ---------------------------------------------------------------------------

/// All tunable Whisper transcription parameters.
///
/// Every field carries `#[serde(default = "…")]` so a hand-edited (or
/// corrupted) TOML file still loads — missing fields fall back to
/// the same defaults used by `Default::default()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperSettings {
    /// (initial, midpoint, final) temperature values for Whisper sampling.
    #[serde(default = "default_temperature")]
    pub temperature: (f64, f64, f64),
    /// Beam size for beam-search decoding.
    #[serde(default = "default_beam_size")]
    pub beam_size: u32,
    /// Source language code (`"auto"` for auto-detect, or ISO 639-1).
    #[serde(default = "default_language")]
    pub language: String,
    /// MLX model identifier (Hugging Face repo or local path).
    #[serde(default = "default_model")]
    pub model: String,
    /// Threshold below which a segment is considered silence/no-speech.
    #[serde(default = "default_no_speech_threshold")]
    pub no_speech_threshold: f64,
    /// Compression ratio threshold for token-level filtering.
    #[serde(default = "default_compression_ratio_threshold")]
    pub compression_ratio_threshold: f64,
    /// Log-probability threshold below which a token is excluded.
    #[serde(default = "default_logprob_threshold")]
    pub logprob_threshold: f64,
}

// ---------------------------------------------------------------------------
// Defaults (mirror current Python hardcoded values in whisper_stream.py)
// ---------------------------------------------------------------------------

fn default_temperature() -> (f64, f64, f64) {
    (0.0, 0.2, 0.4)
}

fn default_beam_size() -> u32 {
    5
}

fn default_language() -> String {
    "auto".to_owned()
}

fn default_model() -> String {
    "mlx-community/whisper-large-v3-turbo".to_owned()
}

fn default_no_speech_threshold() -> f64 {
    0.35
}

fn default_compression_ratio_threshold() -> f64 {
    2.4
}

fn default_logprob_threshold() -> f64 {
    -0.5
}

impl Default for WhisperSettings {
    fn default() -> Self {
        Self {
            temperature: default_temperature(),
            beam_size: default_beam_size(),
            language: default_language(),
            model: default_model(),
            no_speech_threshold: default_no_speech_threshold(),
            compression_ratio_threshold: default_compression_ratio_threshold(),
            logprob_threshold: default_logprob_threshold(),
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

/// Read settings from a TOML file at `path`.
///
/// If the file cannot be read or deserialized, the error is returned as a
/// `String`. Corrupt files are **not** silently replaced — the caller
/// decides whether to fall back via [`load_or_default`].
pub fn load(path: &Path) -> Result<WhisperSettings, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read settings file: {e}"))?;

    toml::from_str(&content).map_err(|e| format!("Failed to parse settings TOML: {e}"))
}

/// Write `settings` to `path` **atomically** (write to `.tmp`, then rename).
///
/// This avoids partial writes if the process crashes mid-write. The
/// parent directory must already exist.
pub fn save(path: &Path, settings: &WhisperSettings) -> Result<(), String> {
    let toml_str = toml::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;

    // Write to a sibling `.tmp` file first so a crash mid-write does not
    // corrupt the real file.
    let tmp_path = path.with_extension("tmp");

    let mut tmp =
        fs::File::create(&tmp_path).map_err(|e| format!("Failed to create temp file: {e}"))?;

    tmp.write_all(toml_str.as_bytes())
        .map_err(|e| format!("Failed to write temp file: {e}"))?;

    // Sync metadata + data to disk before the rename.
    tmp.sync_all()
        .map_err(|e| format!("Failed to sync temp file: {e}"))?;

    // Atomic on the same filesystem.
    fs::rename(&tmp_path, path).map_err(|e| format!("Failed to rename temp file: {e}"))?;

    Ok(())
}

/// Load settings from `path`, or return [`Default::default()`] on any error
/// (missing file, corrupt content, I/O glitch).
///
/// This is the safe startup path — the app never fails to boot because of a
/// bad `settings.user`.
pub fn load_or_default(path: &Path) -> WhisperSettings {
    load(path).unwrap_or_else(|_| WhisperSettings::default())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn defaults_are_valid() {
        let s = WhisperSettings::default();
        assert_eq!(s.temperature, (0.0, 0.2, 0.4));
        assert_eq!(s.beam_size, 5);
        assert_eq!(s.language, "auto");
        assert_eq!(s.model, "mlx-community/whisper-large-v3-turbo");
        assert!((s.no_speech_threshold - 0.35).abs() < 1e-10);
        assert!((s.compression_ratio_threshold - 2.4).abs() < 1e-10);
        assert!((s.logprob_threshold - (-0.5)).abs() < 1e-10);
    }

    #[test]
    fn roundtrip_toml() {
        let original = WhisperSettings {
            temperature: (0.2, 0.4, 0.6),
            beam_size: 7,
            language: "es".into(),
            model: "mlx-community/whisper-medium".into(),
            no_speech_threshold: 0.5,
            compression_ratio_threshold: 2.0,
            logprob_threshold: -1.0,
        };

        let dir = std::env::temp_dir().join("subtitle-overlay-test-roundtrip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.user");

        save(&path, &original).unwrap();
        let loaded = load(&path).unwrap();

        assert_eq!(loaded.temperature, original.temperature);
        assert_eq!(loaded.beam_size, original.beam_size);
        assert_eq!(loaded.language, original.language);
        assert_eq!(loaded.model, original.model);
        assert!((loaded.no_speech_threshold - original.no_speech_threshold).abs() < 1e-10);
        assert!(
            (loaded.compression_ratio_threshold - original.compression_ratio_threshold).abs()
                < 1e-10
        );
        assert!((loaded.logprob_threshold - original.logprob_threshold).abs() < 1e-10);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_file_loads_none() {
        let dir = std::env::temp_dir().join("subtitle-overlay-test-corrupt");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.user");

        // Write garbage
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(b"not valid toml {{{").unwrap();
        drop(f);

        // load() should fail
        assert!(load(&path).is_err());

        // load_or_default() should fall back
        let s = load_or_default(&path);
        assert_eq!(s.beam_size, 5);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_loads_default() {
        let path = Path::new("/tmp/subtitle-overlay-test-missing/nonexistent.user");
        let s = load_or_default(path);
        assert_eq!(s.language, "auto");
    }

    #[test]
    fn atomic_write_survives_interruption() {
        // Simulate: write a valid file, then check the temp file is gone
        // after a successful save.
        let dir = std::env::temp_dir().join("subtitle-overlay-test-atomic");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.user");
        let tmp_path = path.with_extension("tmp");

        let s = WhisperSettings::default();
        save(&path, &s).unwrap();

        // The .tmp file must not linger after a successful save.
        assert!(
            !tmp_path.exists(),
            "tmp file should be cleaned up after rename"
        );

        // The real file must be valid TOML.
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.language, "auto");

        fs::remove_dir_all(&dir).unwrap();
    }
}
