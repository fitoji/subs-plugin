import { create } from 'zustand';
import type { SubtitleEvent } from '../types/subtitle';

export type PipelineStatus = 'idle' | 'listening' | 'processing' | 'error';

interface SubtitleState {
  /** Accumulated transcript — real transcriptions, newest last */
  transcript: SubtitleEvent[];
  /** Pipeline status for the status indicator */
  status: PipelineStatus;
  /** Error message shown when status is 'error' */
  errorMessage: string | null;
  /** Non-error status message (loading, progress, etc.) */
  statusMessage: string | null;
  /** True once user clicks "Empezar a transcribir" — demo mode ends forever */
  isTranscribing: boolean;

  /** Append a new transcription to the transcript */
  addToTranscript: (event: SubtitleEvent) => void;
  /** Clear the entire transcript */
  clearTranscript: () => void;
  /** Set pipeline status and optional message */
  setStatus: (status: PipelineStatus, errorMessage?: string | null, statusMessage?: string | null) => void;
  /** Switch from demo mode to transcription mode */
  startTranscribing: () => void;
  /** Reset store to defaults */
  reset: () => void;
}

const DEFAULTS: Pick<SubtitleState, 'transcript' | 'status' | 'errorMessage' | 'statusMessage' | 'isTranscribing'> = {
  transcript: [],
  status: 'idle',
  errorMessage: null,
  statusMessage: null,
  isTranscribing: false,
};

export const useSubtitleStore = create<SubtitleState>((set) => ({
  ...DEFAULTS,

  addToTranscript: (event) =>
    set((state) => ({
      transcript: [...state.transcript, event],
    })),

  clearTranscript: () =>
    set({ transcript: [] }),

  setStatus: (status, errorMessage = null, statusMessage = null) =>
    set({ status, errorMessage, statusMessage }),

  startTranscribing: () =>
    set({ isTranscribing: true }),

  reset: () =>
    set(DEFAULTS),
}));
