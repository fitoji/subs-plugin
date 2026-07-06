import { create } from 'zustand';
import { persist } from 'zustand/middleware';

interface SettingsState {
  /** Font size in pixels */
  fontSize: number;
  /** Overlay background opacity (0-1) */
  opacity: number;
  /** Backdrop blur radius in pixels */
  backgroundBlur: number;
  /** Maximum width of the overlay in pixels */
  maxWidth: number;

  /** Update a single setting */
  setSetting: <K extends keyof Omit<SettingsState, 'setSetting' | 'reset'>>(
    key: K,
    value: SettingsState[K],
  ) => void;
  /** Reset all settings to defaults */
  reset: () => void;
}

const DEFAULTS: Omit<SettingsState, 'setSetting' | 'reset'> = {
  fontSize: 28,
  opacity: 0.85,
  backgroundBlur: 16,
  maxWidth: 600,
};

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      ...DEFAULTS,

      setSetting: (key, value) =>
        set({ [key]: value }),

      reset: () =>
        set(DEFAULTS),
    }),
    {
      name: 'subtitle-overlay-settings',
    },
  ),
);
