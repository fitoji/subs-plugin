import { useSubtitleStore } from '../store/subtitleStore';
import { useSettingsStore } from '../store/settingsStore';

/**
 * Transparent overlay that renders the current subtitle centered on screen.
 * Uses Tailwind classes and CSS variables from settingsStore for styling.
 * The root div uses data-tauri-drag-region for window dragging.
 */
export function SubtitleOverlay() {
  const currentSubtitle = useSubtitleStore((s) => s.currentSubtitle);
  const { fontSize, opacity, backgroundBlur, maxWidth } = useSettingsStore();

  if (!currentSubtitle) {
    return (
      <div
        data-tauri-drag-region
        className="flex h-screen w-screen items-center justify-center"
      >
        <div className="rounded-xl px-6 py-3 text-center text-white/40 text-lg">
          Waiting for audio...
        </div>
      </div>
    );
  }

  return (
    <div
      data-tauri-drag-region
      className="flex h-screen w-screen items-end justify-center pb-16"
    >
      <div
        className="rounded-xl px-6 py-4 text-center transition-all duration-300"
        style={{
          fontSize: `${fontSize}px`,
          maxWidth: `${maxWidth}px`,
          backgroundColor: `rgba(0, 0, 0, ${opacity})`,
          backdropFilter: `blur(${backgroundBlur}px)`,
          WebkitBackdropFilter: `blur(${backgroundBlur}px)`,
          textShadow: '0 1px 3px rgba(0,0,0,0.8), 0 0 2px rgba(0,0,0,0.5)',
          lineHeight: 1.4,
        }}
      >
        <p className="m-0 text-white select-none">
          {currentSubtitle.text}
        </p>
      </div>
    </div>
  );
}
