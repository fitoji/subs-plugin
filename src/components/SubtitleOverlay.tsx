import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useSubtitleStore } from '../store/subtitleStore';
import { useSettingsStore } from '../store/settingsStore';

// ---------------------------------------------------------------------------
// Demo phrases
// ---------------------------------------------------------------------------

const DEMO_PHRASES = [
  'Hello, and welcome to Subtitle Overlay.',
  'This is a real-time subtitle demonstration.',
  'Subtitles appear here as you watch any content.',
  'Click the button below to start transcribing.',
  'Powered by Whisper MLX and local AI processing.',
];

const DEMO_INTERVAL_MS = 2500;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function startCapture() {
  const store = useSubtitleStore.getState();
  try {
    await invoke('start_capture');
  } catch (e) {
    const msg = typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
    console.error('[overlay] start_capture failed:', msg);
    store.setStatus('error', msg);
  }
}

async function stopCapture() {
  try {
    await invoke('stop_capture');
  } catch (e) {
    const msg = typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
    console.error('[overlay] stop_capture failed:', msg);
  }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function SubtitleOverlay() {
  const transcript = useSubtitleStore((s) => s.transcript);
  const status = useSubtitleStore((s) => s.status);
  const errorMessage = useSubtitleStore((s) => s.errorMessage);
  const isTranscribing = useSubtitleStore((s) => s.isTranscribing);
  const startTranscribing = useSubtitleStore((s) => s.startTranscribing);
  const { fontSize, opacity, backgroundBlur, maxWidth } = useSettingsStore();
  const scrollRef = useRef<HTMLDivElement>(null);

  // ---------- Demo: cycling phrase ----------
  const [demoIndex, setDemoIndex] = useState(0);

  useEffect(() => {
    if (isTranscribing) return;
    const id = setInterval(() => {
      setDemoIndex((i) => (i + 1) % DEMO_PHRASES.length);
    }, DEMO_INTERVAL_MS);
    return () => clearInterval(id);
  }, [isTranscribing]);

  // ---------- Auto-scroll in chat mode ----------
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [transcript.length, transcript[transcript.length - 1]?.text]);

  // ===================================================================
  // MODE 1 — Demo / Welcome
  // ===================================================================

  if (!isTranscribing) {
    return (
      <div
        data-tauri-drag-region
        className="flex h-screen w-screen flex-col items-center justify-center"
      >
        {/* Demo subtitle pill — centered at bottom area */}
        <div
          className="mb-8 rounded-xl px-6 py-4 text-center transition-all duration-500"
          style={{
            fontSize: `${fontSize}px`,
            maxWidth: `min(80vw, ${maxWidth}px)`,
            backgroundColor: `rgba(0, 0, 0, ${opacity})`,
            backdropFilter: `blur(${backgroundBlur}px)`,
            WebkitBackdropFilter: `blur(${backgroundBlur}px)`,
            textShadow: '0 1px 3px rgba(0,0,0,0.8), 0 0 2px rgba(0,0,0,0.5)',
            lineHeight: 1.4,
          }}
        >
          <p className="m-0 select-none text-white">
            {DEMO_PHRASES[demoIndex]}
          </p>
        </div>

        {/* Call-to-action button */}
        <button
          onClick={async (e) => {
            e.stopPropagation();
            startTranscribing();
            await startCapture();
          }}
          className="cursor-pointer rounded-xl border border-white/20 bg-white/10 px-8 py-3 text-lg text-white/80 backdrop-blur-sm transition-colors hover:bg-white/20 hover:text-white active:bg-white/30"
        >
          Empezar a transcribir
        </button>
      </div>
    );
  }

  // ===================================================================
  // MODE 2 — Chat transcript
  // ===================================================================

  const hasTranscript = transcript.length > 0;

  return (
    <div
      data-tauri-drag-region
      className="flex h-screen w-screen cursor-pointer flex-col"
      onClick={() => {
        if (status === 'listening' || status === 'processing') {
          stopCapture();
        } else {
          startCapture();
        }
      }}
    >
      {/* Scrollable transcript */}
      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto px-4 py-4"
        style={{
          scrollBehavior: 'smooth',
          backgroundColor: hasTranscript ? `rgba(0, 0, 0, ${opacity})` : undefined,
          backdropFilter: hasTranscript ? `blur(${backgroundBlur}px)` : undefined,
          WebkitBackdropFilter: hasTranscript ? `blur(${backgroundBlur}px)` : undefined,
        }}
      >
        {hasTranscript && (
          <div
            className="mx-auto"
            style={{ maxWidth: `min(90vw, ${maxWidth}px)` }}
          >
            {transcript.map((entry) => (
              <p
                key={entry.id}
                className="m-0 select-none leading-relaxed text-white"
                style={{
                  fontSize: `${fontSize}px`,
                  textShadow: '0 1px 3px rgba(0,0,0,0.9), 0 0 2px rgba(0,0,0,0.6)',
                  lineHeight: 1.5,
                }}
              >
                {entry.text}
              </p>
            ))}
          </div>
        )}
      </div>

      {/* Status bar — only transient states */}
      <div className="flex items-center justify-center gap-2 px-4 pb-3 pt-1">
        {status === 'listening' && (
          <span className="text-xs text-white/40">Recording…</span>
        )}
        {status === 'processing' && (
          <span className="text-xs text-white/60">Processing…</span>
        )}
        {status === 'error' && (
          <span className="text-xs text-red-400">
            {errorMessage ?? 'Error'}
          </span>
        )}
      </div>
    </div>
  );
}
