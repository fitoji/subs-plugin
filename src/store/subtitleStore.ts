import { create } from 'zustand';
import type { SubtitleEvent } from '../types/subtitle';

interface SubtitleState {
  /** The current subtitle to display */
  currentSubtitle: SubtitleEvent | null;
  /** Queue of recent subtitles for history context */
  recentSubtitles: SubtitleEvent[];
  /** Maximum entries in recentSubtitles */
  maxHistory: number;

  /** Set the current subtitle (from source) */
  setCurrentSubtitle: (event: SubtitleEvent) => void;
  /** Clear the current subtitle */
  clearCurrentSubtitle: () => void;
  /** Reset store to defaults */
  reset: () => void;
}

const DEFAULTS: Pick<SubtitleState, 'currentSubtitle' | 'recentSubtitles'> = {
  currentSubtitle: null,
  recentSubtitles: [],
};

export const useSubtitleStore = create<SubtitleState>((set) => ({
  ...DEFAULTS,
  maxHistory: 10,

  setCurrentSubtitle: (event) =>
    set((state) => ({
      currentSubtitle: event,
      recentSubtitles: [event, ...state.recentSubtitles].slice(0, state.maxHistory),
    })),

  clearCurrentSubtitle: () =>
    set({ currentSubtitle: null }),

  reset: () =>
    set(DEFAULTS),
}));
