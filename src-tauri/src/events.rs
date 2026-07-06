//! Tauri IPC event emission helpers.
//!
//! Provides typed helpers for emitting the two event channels used by
//! the subtitle pipeline:
//!
//! - `"subtitle-event"` — carries `SubtitleEventPayload` to the React overlay
//! - `"system-event"`   — carries `SystemEventPayload` for status indicators

use serde::Serialize;
use tauri::{Emitter, Wry};

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

/// Payload for the `"subtitle-event"` channel.
#[derive(Debug, Clone, Serialize)]
pub struct SubtitleEventPayload {
    pub id: u32,
    pub text: String,
    #[serde(rename = "isFinal")]
    pub is_final: bool,
    pub timestamp: u64,
}

/// Payload for the `"system-event"` channel.
#[derive(Debug, Clone, Serialize)]
pub struct SystemEventPayload {
    #[serde(rename = "type")]
    pub event_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// Emit helpers
// ---------------------------------------------------------------------------

/// Emit a `SubtitleEventPayload` on the `"subtitle-event"` channel.
pub fn emit_subtitle_event(app: &impl Emitter<Wry>, payload: SubtitleEventPayload) {
    let _ = app.emit("subtitle-event", payload);
}

/// Emit a `SystemEventPayload` on the `"system-event"` channel.
pub fn emit_system_event(app: &impl Emitter<Wry>, payload: SystemEventPayload) {
    let _ = app.emit("system-event", payload);
}

/// Convenience: emit an audio status change.
pub fn emit_audio_status(app: &impl Emitter<Wry>, status: &str, message: Option<&str>) {
    emit_system_event(
        app,
        SystemEventPayload {
            event_type: "audio_status".into(),
            status: status.into(),
            message: message.map(String::from),
        },
    );
}

/// Convenience: emit an STT status change.
pub fn emit_stt_status(app: &impl Emitter<Wry>, status: &str, message: Option<&str>) {
    emit_system_event(
        app,
        SystemEventPayload {
            event_type: "stt_status".into(),
            status: status.into(),
            message: message.map(String::from),
        },
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_serialization() {
        let sub = SubtitleEventPayload {
            id: 1,
            text: "hello".into(),
            is_final: true,
            timestamp: 1_000_000,
        };
        let json = serde_json::to_string(&sub).unwrap();
        assert!(json.contains("\"isFinal\":true"));
        assert!(json.contains("\"text\":\"hello\""));
    }

    #[test]
    fn test_system_event_serialization() {
        let evt = SystemEventPayload {
            event_type: "audio_status".into(),
            status: "error".into(),
            message: Some("Permission denied".into()),
        };
        let json = serde_json::to_string(&evt).unwrap();
        assert!(json.contains("\"message\":\"Permission denied\""));
        assert!(json.contains("\"type\":\"audio_status\""));
    }

    #[test]
    fn test_system_event_no_message() {
        let evt = SystemEventPayload {
            event_type: "stt_status".into(),
            status: "ready".into(),
            message: None,
        };
        let json = serde_json::to_string(&evt).unwrap();
        assert!(!json.contains("message"));
    }
}
