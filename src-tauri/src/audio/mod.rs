//! System audio capture module.
//!
//! Uses `screencapturekit-rs` (macOS 13+) to capture system audio output.
//! Captured audio (48 kHz stereo float) is resampled to 16 kHz mono PCM,
//! accumulated in a sliding ring buffer, gated by silence detection (RMS VAD),
//! and flushed as 2 s i16 PCM chunks with 0.5 s overlap.
//!
//! ## Architecture
//!
//! ```text
//! SCStream (callback)  →  extract + channel → process task
//!                            ↓                       ↓
//!                     CMSampleBuffer          handle_audio_buffer()
//!                     audio_buffer_list()         ↓
//!                     extract f32 frames    resample_and_mix()
//!                            ↓                  ↓
//!                      try_send(tx)        RingBuffer → VAD → PCM
//! ```
//!
//! ## Thread safety
//!
//! The SCStream audio callback runs on an unknown dispatch queue. Raw audio
//! frames are extracted there (safe, read-only) and sent through a tokio
//! channel to an async task that owns the `AudioCapture` processing logic.

use std::collections::VecDeque;

use screencapturekit::cm::CMSampleBufferExt;
use screencapturekit::prelude::*;
use screencapturekit::stream::configuration::{AudioChannelCount, AudioSampleRate};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the audio capture pipeline.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Target output sample rate (Hz). Default: 16 000.
    pub sample_rate: u32,
    /// Window duration in milliseconds. Default: 2000.
    pub chunk_ms: u32,
    /// Overlap between consecutive windows in milliseconds. Default: 500.
    pub overlap_ms: u32,
    /// RMS silence threshold (0.0 – 1.0). Default: 0.02.
    pub silence_threshold: f32,
    /// Input sample rate from ScreenCaptureKit (always 48 000 on macOS).
    pub input_sample_rate: u32,
    /// Number of input channels (always 2 for stereo).
    pub input_channels: u16,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            // 5 s chunks (vs 2 s) give Whisper more phonetic context —
            // trade-off is ~3 s additional latency per the fidelity plan
            chunk_ms: 5000,
            overlap_ms: 1000,
            silence_threshold: 0.02,
            input_sample_rate: 48_000,
            input_channels: 2,
        }
    }
}

impl AudioConfig {
    /// Number of output samples per chunk window.
    pub fn chunk_samples(&self) -> usize {
        (self.sample_rate as u64 * self.chunk_ms as u64 / 1000) as usize
    }

    /// Number of output samples for the overlap portion.
    pub fn overlap_samples(&self) -> usize {
        (self.sample_rate as u64 * self.overlap_ms as u64 / 1000) as usize
    }

    /// Step size (advance) between consecutive chunks.
    pub fn step_samples(&self) -> usize {
        self.chunk_samples().saturating_sub(self.overlap_samples())
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("ScreenCaptureKit permission denied — user denied screen recording")]
    PermissionDenied,

    #[error("No audio output device found")]
    NoDevice,

    #[error("ScreenCaptureKit stream error: {0}")]
    Capture(String),

    #[error("Ring buffer underflow — not enough samples for a full chunk")]
    BufferUnderflow,
}

// ---------------------------------------------------------------------------
// Resampling and channel mixing
// ---------------------------------------------------------------------------

/// Resample from 48 kHz stereo float to 16 kHz mono float.
///
/// `input` is an interleaved `[L, R, L, R, …]` f32 buffer at `input_rate` Hz.
/// Returns a single-channel f32 buffer at `target_rate` Hz, normalized [-1, 1].
pub fn resample_and_mix(
    input: &[f32],
    input_rate: u32,
    target_rate: u32,
    channels: u16,
) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }

    // Step 1: mix down to mono (average channels)
    let frames = input.len() / channels as usize;
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        let start = frame * channels as usize;
        let sum: f32 = input[start..start + channels as usize].iter().sum();
        mono.push(sum / channels as f32);
    }

    // Step 2: resample if needed (linear interpolation)
    if input_rate == target_rate {
        return mono;
    }

    let ratio = target_rate as f64 / input_rate as f64;
    let target_len = (mono.len() as f64 * ratio).ceil() as usize;
    let mut out = Vec::with_capacity(target_len);

    for i in 0..target_len {
        let src_idx = i as f64 / ratio;
        let lo = src_idx.floor() as usize;
        let hi = (lo + 1).min(mono.len() - 1);
        let frac = src_idx - src_idx.floor();
        let sample = mono[lo] * (1.0 - frac as f32) + mono[hi] * frac as f32;
        out.push(sample);
    }

    out
}

// ---------------------------------------------------------------------------
// Silence detection (VAD)
// ---------------------------------------------------------------------------

/// Returns `true` if the RMS energy of `samples` is below `threshold`.
///
/// `samples` should be float values in [-1, 1].
pub fn detect_silence(samples: &[f32], threshold: f32) -> bool {
    if samples.is_empty() {
        return true;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    let rms = (sum_sq / samples.len() as f32).sqrt();
    rms < threshold
}

// ---------------------------------------------------------------------------
// Ring buffer with overlap
// ---------------------------------------------------------------------------

/// A sliding ring buffer that accumulates mono float samples and produces
/// fixed-size chunks with configurable overlap.
pub struct RingBuffer {
    /// Internal sample buffer (mono f32).
    buf: VecDeque<f32>,
    /// Configuration (sample rate, window size, overlap).
    config: AudioConfig,
}

impl RingBuffer {
    pub fn new(config: AudioConfig) -> Self {
        Self {
            buf: VecDeque::with_capacity(config.chunk_samples() * 2),
            config,
        }
    }

    /// Push new samples into the buffer.
    pub fn push(&mut self, samples: &[f32]) {
        self.buf.extend(samples.iter().copied());
    }

    /// Total samples currently in the buffer.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Drain samples up to `count` from the front (advance the window).
    pub fn drain_front(&mut self, count: usize) {
        let n = count.min(self.buf.len());
        self.buf.drain(..n);
    }

    /// Get a contiguous window of `self.config.chunk_samples()` from the front.
    /// Returns `None` if there aren't enough samples yet.
    pub fn peek_chunk(&self) -> Option<Vec<f32>> {
        if self.buf.len() < self.config.chunk_samples() {
            return None;
        }
        let chunk: Vec<f32> = self.buf.range(..self.config.chunk_samples()).copied().collect();
        Some(chunk)
    }

    /// Advance the buffer by `step_samples()` (chunk - overlap).
    pub fn advance(&mut self) {
        let step = self.config.step_samples();
        self.drain_front(step);
    }

    /// Clear all buffered samples.
    pub fn reset(&mut self) {
        self.buf.clear();
    }
}

// ---------------------------------------------------------------------------
// PCM encoding helper
// ---------------------------------------------------------------------------

/// Convert mono f32 samples [-1, 1] to little-endian i16 PCM bytes.
pub fn f32_to_i16_pcm(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let i16_sample = (clamped * 32767.0) as i16;
        out.extend_from_slice(&i16_sample.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Audio frame extraction from CMSampleBuffer
// ---------------------------------------------------------------------------

/// Extract interleaved f32 audio frames from a `CMSampleBuffer`.
///
/// Handles both interleaved (1 buffer × N channels) and non-interleaved
/// (N buffers × 1 channel each) `AudioBufferList` layouts. Returns `None`
/// when the buffer has no audio data or the format is unexpected.
///
/// The output is always **interleaved** `[L, R, L, R, …]` f32, matching
/// what `resample_and_mix()` expects.
pub fn extract_audio_frames_from_sample(sample: &CMSampleBuffer) -> Option<Vec<f32>> {
    let audio_list = sample.audio_buffer_list()?;
    let num_buffers = audio_list.num_buffers();

    if num_buffers == 0 {
        return None;
    }

    // --- Case 1: single interleaved buffer ---
    if num_buffers == 1 {
        let buf = audio_list.get(0)?;
        let data = buf.data();
        let _channels = buf.number_channels.max(1) as usize;

        if data.is_empty() || data.len() < 4 {
            return None;
        }

        // SAFETY: ScreenCaptureKit delivers 32-bit float PCM data.
        let samples: &[f32] = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const f32, data.len() / 4)
        };

        // If the buffer claims 2 channels but the data is actually
        // non-interleaved single-channel, we'd get half the expected
        // frames. We trust the AudioBuffer's channel count here.
        return Some(samples.to_vec());
    }

    // --- Case 2: non-interleaved (one buffer per channel) ---
    // Collect each channel's data as byte slices.
    let mut channel_data: Vec<&[u8]> = Vec::with_capacity(num_buffers);
    for i in 0..num_buffers {
        let buf = audio_list.get(i)?;
        channel_data.push(buf.data());
    }

    let frame_count = channel_data[0].len() / 4; // 4 bytes per f32
    let mut out = Vec::with_capacity(frame_count * num_buffers);

    for frame in 0..frame_count {
        for ch in 0..num_buffers {
            let ch_bytes = channel_data[ch];
            let offset = frame * 4;
            if offset + 4 <= ch_bytes.len() {
                // SAFETY: raw f32 bytes from CoreMedia audio buffer.
                let sample_val = unsafe { std::ptr::read(ch_bytes.as_ptr().add(offset) as *const f32) };
                out.push(sample_val);
            }
        }
    }

    Some(out)
}

// ---------------------------------------------------------------------------
// SCStream creation
// ---------------------------------------------------------------------------

/// Create and start an `SCStream` that captures system audio on the first
/// available display.
///
/// Audio frames are extracted in the callback and sent through `audio_tx` as
/// interleaved f32 `[L, R, L, R, …]` at 48 kHz.
///
/// This function **blocks** (uses synchronous FFI) — call it from
/// `spawn_blocking` or a dedicated thread.
pub fn create_audio_stream(
    audio_tx: tokio::sync::mpsc::Sender<Vec<f32>>,
) -> Result<SCStream, AudioError> {
    // 1. Get shareable content
    let content = SCShareableContent::get().map_err(|e| AudioError::Capture(e.to_string()))?;

    let display = content
        .displays()
        .first()
        .cloned()
        .ok_or(AudioError::NoDevice)?;

    // 2. Content filter — capture entire first display
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build();

    // 3. Stream configuration — enable audio capture
    let config = SCStreamConfiguration::new()
        .with_captures_audio(true)
        .with_sample_rate(AudioSampleRate::Rate48000)
        .with_channel_count(AudioChannelCount::Stereo)
        .with_excludes_current_process_audio(true);

    // 4. Error handler (logs stream errors)
    let error_handler = ErrorHandler::new(|error| {
        eprintln!("[audio] SCStream error: {error}");
    });

    // 5. Create stream with delegate for error handling
    let mut stream = SCStream::new_with_delegate(&filter, &config, error_handler);

    // 6. Audio output handler — extract frames and send through channel
    stream.add_output_handler(
        move |sample: CMSampleBuffer, of_type: SCStreamOutputType| {
            if of_type == SCStreamOutputType::Audio {
                if let Some(frames) = extract_audio_frames_from_sample(&sample) {
                    // Use try_send — never block the SC dispatch queue.
                    if audio_tx.try_send(frames).is_err() {
                        // Channel full or closed — drop this frame.
                    }
                }
            }
        },
        SCStreamOutputType::Audio,
    );

    // 7. Start capture
    stream.start_capture().map_err(|e| AudioError::Capture(e.to_string()))?;

    Ok(stream)
}

// ---------------------------------------------------------------------------
// AudioCapture — high-level API
// ---------------------------------------------------------------------------

/// High-level audio capture orchestrator.
///
/// Manages the audio pipeline from ScreenCaptureKit callback to PCM chunk
/// emission. The actual `SCStream` creation is handled by platform-specific
/// code (see `start()` and the `handle_audio_buffer()` callback path).
pub struct AudioCapture {
    config: AudioConfig,
    ring: RingBuffer,
    /// Whether capture is active.
    active: bool,
    /// Running status (for status reporting).
    pub audio_status: AudioStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AudioStatus {
    Idle,
    Active,
    Silence,
    Error(String),
}

impl AudioCapture {
    /// Create a new capture pipeline with the given config.
    pub fn new(config: AudioConfig) -> Self {
        let ring = RingBuffer::new(config.clone());
        Self {
            audio_status: AudioStatus::Idle,
            active: false,
            config,
            ring,
        }
    }

    pub fn config(&self) -> &AudioConfig {
        &self.config
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    // ----- Lifecycle -------------------------------------------------------

    /// Mark capture as active.
    ///
    /// Does NOT create the SCStream — that is done externally via
    /// [`create_audio_stream()`]. This simply sets `active = true` so that
    /// [`handle_audio_buffer()`] accepts incoming frames.
    pub fn start(&mut self) -> Result<(), AudioError> {
        if self.active {
            return Ok(());
        }
        self.active = true;
        self.audio_status = AudioStatus::Active;
        Ok(())
    }

    /// Stop capture and reset the ring buffer.
    pub fn stop(&mut self) -> Result<(), AudioError> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        self.ring.reset();
        self.audio_status = AudioStatus::Idle;
        Ok(())
    }

    // ----- Audio data processing -------------------------------------------

    /// Process raw audio data from ScreenCaptureKit.
    ///
    /// `input` is interleaved `[L, R, L, R, …]` f32 samples at 48 kHz.
    /// This method resamples, mixes to mono, accumulates the ring buffer,
    /// runs VAD, and returns a PCM chunk if a full window is ready.
    ///
    /// Returns `Some(Vec<u8>)` — 2 s of 16 kHz mono i16 PCM little-endian —
    /// when a full chunk is available and not silent.
    pub fn handle_audio_buffer(&mut self, input: &[f32]) -> Option<Vec<u8>> {
        if !self.active || input.is_empty() {
            return None;
        }

        // Resample 48k stereo → 16k mono
        let mono = resample_and_mix(
            input,
            self.config.input_sample_rate,
            self.config.sample_rate,
            self.config.input_channels,
        );

        // Push to ring buffer
        self.ring.push(&mono);

        // Check if we have a full chunk
        let chunk = self.ring.peek_chunk()?;

        // Silence gate
        if detect_silence(&chunk, self.config.silence_threshold) {
            self.audio_status = AudioStatus::Silence;
            self.ring.advance(); // still advance to keep buffer fresh
            return None;
        }

        // Produce PCM output
        self.audio_status = AudioStatus::Active;
        let pcm = f32_to_i16_pcm(&chunk);
        self.ring.advance();
        Some(pcm)
    }

    /// Reset state (e.g., after a stream error).
    pub fn reset(&mut self) {
        self.ring.reset();
        self.audio_status = AudioStatus::Idle;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AudioConfig {
        AudioConfig {
            sample_rate: 16_000,
            chunk_ms: 5000,
            overlap_ms: 1000,
            ..Default::default()
        }
    }

    #[test]
    fn test_config_samples() {
        let cfg = test_config();
        assert_eq!(cfg.chunk_samples(), 32000); // 2 s × 16 kHz
        assert_eq!(cfg.overlap_samples(), 8000); // 0.5 s × 16 kHz
        assert_eq!(cfg.step_samples(), 24000);  // 32000 - 8000
    }

    #[test]
    fn test_resample_stereo_to_mono() {
        // 48 kHz stereo → 16 kHz mono
        let input_rate = 48_000;
        let target_rate = 16_000;
        let channels = 2;

        // 1 frame = 2 samples (L + R)
        let frames = 480; // 10 ms at 48 kHz
        let mut input = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            input.push(0.5); // L
            input.push(0.3); // R
        }

        let result = resample_and_mix(&input, input_rate, target_rate, channels);
        // 10 ms at 16 kHz = 160 samples
        assert!(!result.is_empty());
        // Values should be averaged: (0.5 + 0.3) / 2 = 0.4
        assert!((result[0] - 0.4).abs() < 0.01);
    }

    #[test]
    fn test_detect_silence() {
        assert!(detect_silence(&[0.0, 0.0, 0.0], 0.02));
        assert!(!detect_silence(&[0.5, -0.3, 0.8], 0.02));
        assert!(detect_silence(&[], 0.02));
    }

    #[test]
    fn test_ring_buffer_push_and_chunk() {
        let mut ring = RingBuffer::new(test_config());
        assert!(ring.is_empty());

        // Push 1 second of silence
        let one_sec: Vec<f32> = vec![0.0; 16000];
        ring.push(&one_sec);
        assert_eq!(ring.len(), 16000);
        assert!(ring.peek_chunk().is_none()); // need 32000

        // Push another second
        ring.push(&one_sec);
        assert_eq!(ring.len(), 32000);
        assert!(ring.peek_chunk().is_some());
    }

    #[test]
    fn test_ring_buffer_overlap_advance() {
        let mut ring = RingBuffer::new(test_config());

        // Fill buffer with identifiable samples
        let samples: Vec<f32> = (0..48000).map(|i| i as f32 / 48000.0).collect();
        ring.push(&samples);

        let chunk1 = ring.peek_chunk().unwrap();
        assert_eq!(chunk1.len(), 32000);
        assert!((chunk1[0] - 0.0).abs() < 0.001);
        assert!((chunk1[100] - 100.0 / 48000.0).abs() < 0.001);

        ring.advance(); // step = 24000
        assert_eq!(ring.len(), 48000 - 24000);

        // After advance, the remaining data starts at sample index 24000
        // Since we consumed 24000 and have 24000 left, next peek should need
        // 32000, but we only have 24000, so it should return None
        assert!(ring.peek_chunk().is_none());
    }

    #[test]
    fn test_f32_to_i16_pcm() {
        let input = vec![0.0, 0.5, -0.5, 1.0, -1.0];
        let bytes = f32_to_i16_pcm(&input);
        assert_eq!(bytes.len(), input.len() * 2);
        // 0.0 → 0
        assert_eq!(bytes[0], 0);
        assert_eq!(bytes[1], 0);
        // 1.0 → 32767
        let last = i16::from_le_bytes([bytes[bytes.len() - 2], bytes[bytes.len() - 1]]);
        assert_eq!(last, -32768); // -1.0 → -32768
    }

    #[test]
    fn test_audio_capture_handle_buffer() {
        let mut cap = AudioCapture::new(test_config());
        assert!(!cap.is_active());

        // Simulate a stereo buffer at 48 kHz (10 ms = 480 frames × 2 channels)
        let mut buf = Vec::with_capacity(960);
        for _ in 0..480 {
            buf.push(0.1); // L
            buf.push(0.1); // R
        }

        // Not active → should return None
        assert!(cap.handle_audio_buffer(&buf).is_none());

        // Activate
        cap.start().unwrap();
        assert!(cap.is_active());

        // Need enough data for a full chunk (32000 samples mono → 64000 PCM bytes)
        // Push 3 seconds of non-silent audio at 48 kHz stereo = 144k samples
        let three_sec_frames = 48_000 * 3; // 144k frames
        let mut big_buf = Vec::with_capacity(three_sec_frames * 2);
        for _ in 0..three_sec_frames {
            big_buf.push(0.1);
            big_buf.push(0.1);
        }
        let result = cap.handle_audio_buffer(&big_buf);
        assert!(result.is_some());
        let pcm = result.unwrap();
        assert_eq!(pcm.len(), 32000 * 2); // 32000 samples × 2 bytes = 64000
    }

    #[test]
    fn test_audio_capture_silence_returns_none() {
        let mut cap = AudioCapture::new(test_config());
        cap.start().unwrap();

        // Push silent audio
        let three_sec_frames = 48_000 * 3;
        let mut buf = vec![0.0f32; three_sec_frames * 2]; // all zeros
        let result = cap.handle_audio_buffer(&buf);
        assert!(result.is_none()); // silence → no chunk
        assert_eq!(cap.audio_status, AudioStatus::Silence);
    }
}
