# Subtitle Overlay

> Transparent, always-on-top subtitles for any macOS application. Powered by local AI.

Subtitle Overlay is a macOS desktop app that captures system audio, transcribes it in real-time using Whisper (via MLX on Apple Silicon), and displays subtitles in a transparent overlay on top of any application — no integration needed.

**Status:** v0.1 — Overlay base with demo subtitles (no audio/AI yet)

---

## Features

| Feature | Status | Version |
|---|---|---|
| Transparent overlay, always-on-top | ✅ | v0.1 |
| Drag window to any position | ✅ | v0.1 |
| Demo subtitle cycling | ✅ | v0.1 |
| Settings persistence (font size, opacity, blur) | ✅ | v0.1 |
| System audio capture (ScreenCaptureKit) | ⏳ | v0.2 |
| Real-time transcription (Whisper MLX) | ⏳ | v0.2 |
| Subtitle translation (LLM) | 🔜 | v0.3 |
| Interactive dictionary (clickable words) | 🔜 | v0.4 |
| Vocabulary saving | 🔜 | v0.5 |
| Full configuration UI | 🔜 | v1.0 |

---

## Requirements

- **macOS 13+** (Ventura or later)
- **Apple Silicon** (M1, M2, M3, M4 or later)
- ~200MB free RAM (idle, v0.1)
- ~2GB for Whisper MLX models (v0.2+, downloaded on first use)

---

## Installation

### Download the latest release

Grab the latest `.dmg` or `.app` from the [Releases](https://github.com/your-username/subtitle-overlay/releases) page (once published).

### Build from source

```bash
git clone https://github.com/your-username/subtitle-overlay.git
cd subtitle-overlay
pnpm install
pnpm tauri build
```

The built app will be at:

```
src-tauri/target/release/bundle/macos/Subtitle Overlay.app
```

Or install via DMG:

```
src-tauri/target/release/bundle/dmg/Subtitle Overlay_0.1.0_aarch64.dmg
```

---

## Quick Start

1. Open **Subtitle Overlay.app**
2. A transparent overlay appears at the bottom-center of your screen
3. Demo subtitles cycle automatically every 2 seconds
4. Drag the overlay by clicking and dragging anywhere on the text area

> **Note:** v0.1 uses demo phrases. Real audio transcription comes in v0.2.

---

## Development

### Prerequisites

- [Rust](https://www.rust-lang.org/) (1.96+)
- [Node.js](https://nodejs.org/) (24+)
- [pnpm](https://pnpm.io/) (11+)
- [Xcode](https://developer.apple.com/xcode/) Command Line Tools
- macOS 13+ with Apple Silicon

### Setup

```bash
# Clone the repo
git clone https://github.com/your-username/subtitle-overlay.git
cd subtitle-overlay

# Install frontend dependencies
pnpm install

# Run in development mode
pnpm tauri dev

# Build for production
pnpm tauri build
```

### Commands

| Command | Description |
|---|---|
| `pnpm dev` | Start Vite dev server (frontend only) |
| `pnpm tauri dev` | Run app in development mode (hot-reload) |
| `pnpm tauri build` | Build production .app and .dmg |
| `pnpm tsc --noEmit` | TypeScript type checking |
| `cargo build` | Build Rust backend only |
| `cargo clippy -- -D warnings` | Rust linting |

### Project Structure

```
subtitle-overlay/
├── src/                          # Frontend (React + TypeScript)
│   ├── components/
│   │   └── SubtitleOverlay.tsx   # Overlay component
│   ├── hooks/
│   │   └── useSubtitleDemo.ts    # Demo subtitle source
│   ├── store/
│   │   ├── subtitleStore.ts      # Subtitle state (Zustand)
│   │   └── settingsStore.ts      # Settings state (persisted)
│   ├── types/
│   │   └── subtitle.ts           # SubtitleEvent + SystemEvent contracts
│   ├── styles/
│   │   └── globals.css           # TailwindCSS v4 config
│   ├── App.tsx
│   └── main.tsx
├── src-tauri/                    # Backend (Rust + Tauri v2)
│   ├── src/
│   │   ├── main.rs               # Entry point
│   │   ├── lib.rs                 # App builder
│   │   └── window.rs             # Window configuration
│   ├── capabilities/
│   │   └── default.json          # Tauri v2 permissions
│   ├── Cargo.toml
│   └── tauri.conf.json
├── ai-pipeline/                  # v0.2+: Python MLX processes
├── docs/
│   ├── subtitle-overlay-spec.md  # Full product specification (Spanish)
│   └── decisions/                # Architecture Decision Records
├── subtitle-overlay-spec.md      # Complete PRD + architecture
├── package.json
└── vite.config.ts
```

### Architecture Overview

```
Frontend (React + Zustand)  ← IPC / events →  Backend (Rust + Tauri v2)
       ↓                                              ↓
  Transparent overlay                           Audio capture + STT
  (always-on-top)                              (v0.2+ via sidecars)
```

The frontend is intentionally **dumb**: it only renders `SubtitleEvent` objects emitted by the backend. The backend orchestrates audio capture, STT, translation, and dictionary services as external sidecar processes.

For complete architecture details, see the [spec](subtitle-overlay-spec.md) (Spanish).

---

## Tech Stack

| Layer | Technology |
|---|---|
| App shell | [Tauri v2](https://v2.tauri.app/) |
| UI | [React 19](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) |
| Build | [Vite](https://vite.dev/) 6 |
| Styling | [TailwindCSS 4](https://tailwindcss.com/) (CSS-first) |
| State | [Zustand](https://zustand-demo.pmnd.rs/) 5 |
| Backend | [Rust](https://www.rust-lang.org/) |
| Audio capture | ScreenCaptureKit / CoreAudio (Rust bindings) |
| Speech-to-text | Whisper via [MLX](https://ml-explore.github.io/mlx/) |
| Translation | LLM (local MLX or remote API) |
| Persistence | SQLite (rusqlite/sqlx) — v0.5+ |
| Package manager | [pnpm](https://pnpm.io/) |

---

## Roadmap

| Version | Focus | Key Milestone |
|---|---|---|
| ✅ **v0.1** | Overlay base | Transparent, always-on-top, draggable, demo subtitles |
| 🔜 **v0.2** | Audio + Whisper | Real-time transcription from system audio |
| 🔜 **v0.3** | Translation | Dual subtitles (original + translation) |
| 🔜 **v0.4** | Dictionary | Clickable words with definitions |
| 🔜 **v0.5** | Vocabulary | Saved words with review list |
| 🔜 **v1.0** | Polish + compatibility | Verified on VLC, YouTube, Netflix, Safari, Chrome, podcasts |

---

## Development Decisions

Notable architecture decisions are recorded as ADRs in [`docs/decisions/`](docs/decisions/):

- **ADR-0001**: SubtitleOverlay reads directly from Zustand store (not prop-driven)

---

## License

MIT — see [LICENSE](LICENSE) for details.

---

*Built with [Tauri v2](https://v2.tauri.app/), [React](https://react.dev/), and [Rust](https://www.rust-lang.org/).*
