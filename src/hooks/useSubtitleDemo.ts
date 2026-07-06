import { useEffect, useRef } from 'react';
import { useSubtitleStore } from '../store/subtitleStore';
import type { SubtitleEvent } from '../types/subtitle';

const DEMO_PHRASES = [
  'Hello, and welcome to Subtitle Overlay.',
  'This is a real-time subtitle demonstration.',
  'Subtitles appear here as you watch any content.',
  'Future versions will transcribe system audio.',
  'Click on any word to look it up in the dictionary.',
  'You can save words to your vocabulary list.',
  'Subtitle Overlay works with any macOS application.',
  'Powered by Whisper MLX and local AI processing.',
];

const INTERVAL_MS = 2000;

/**
 * Demo mode hook — emits hardcoded SubtitleEvent phrases on an interval.
 * In v0.2+, this gets replaced by useSubtitleStream (Tauri events from real STT).
 * Can be toggled via the `enabled` parameter for fallback/testing mode.
 */
export function useSubtitleDemo(enabled = true): void {
  const setCurrentSubtitle = useSubtitleStore((s) => s.setCurrentSubtitle);
  const clearCurrentSubtitle = useSubtitleStore((s) => s.clearCurrentSubtitle);
  const idRef = useRef(0);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (!enabled) {
      clearCurrentSubtitle();
      return;
    }

    // Emit first phrase immediately
    const emitNext = () => {
      const phrase = DEMO_PHRASES[idRef.current % DEMO_PHRASES.length];
      idRef.current += 1;

      const event: SubtitleEvent = {
        id: idRef.current,
        text: phrase,
        isFinal: true,
        timestamp: Date.now(),
      };

      setCurrentSubtitle(event);
    };

    emitNext();
    intervalRef.current = setInterval(emitNext, INTERVAL_MS);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
      clearCurrentSubtitle();
    };
  }, [enabled, setCurrentSubtitle, clearCurrentSubtitle]);
}
