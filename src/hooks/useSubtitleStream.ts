import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useSubtitleStore, type PipelineStatus } from '../store/subtitleStore';
import type { SubtitleEvent } from '../types/subtitle';

/**
 * Payload shape received from the Rust backend's `"system-event"` channel.
 */
interface SystemEventPayload {
  type: 'stt_status' | 'audio_status' | 'translator_status';
  status: string;
  message?: string;
}

/**
 * Maps Rust backend status strings to the frontend PipelineStatus type.
 */
function mapStatus(raw: string): PipelineStatus {
  switch (raw) {
    case 'listening':
    case 'active':
      return 'listening';
    case 'loading':
    case 'processing':
      return 'processing';
    case 'error':
    case 'fatal':
      return 'error';
    default:
      return 'idle';
  }
}

/**
 * Hook that listens for Tauri IPC events from the Rust backend
 * and feeds them into the Zustand subtitle store.
 *
 * Call once at the app root when real audio capture is active.
 * Cleans up listeners on unmount.
 */
export function useSubtitleStream() {
  const addToTranscript = useSubtitleStore((s) => s.addToTranscript);
  const setStatus = useSubtitleStore((s) => s.setStatus);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];

    async function setup() {
      // Listen for subtitle events from the Whisper pipeline
      const unlistenSub = await listen<SubtitleEvent>('subtitle-event', (event) => {
        if (cancelled) return;
        addToTranscript(event.payload);
      });
      unlisteners.push(unlistenSub);

      // Listen for system status events
      const unlistenSys = await listen<SystemEventPayload>('system-event', (event) => {
        if (cancelled) return;
        const { type: _type, status, message } = event.payload;
        const mapped = mapStatus(status);
        setStatus(mapped, mapped === 'error' ? message ?? null : null);

        if (message) {
          console.debug(`[pipeline] ${_type}: ${status} — ${message}`);
        }
      });
      unlisteners.push(unlistenSys);
    }

    setup();

    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [addToTranscript, setStatus]);
}
