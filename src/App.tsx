import { SubtitleOverlay } from './components/SubtitleOverlay';
import { useSubtitleStream } from './hooks/useSubtitleStream';

function App() {
  // Listen for real STT events from the Rust backend (always mounted).
  // Only fires when the pipeline is active sending events.
  useSubtitleStream();

  return <SubtitleOverlay />;
}

export default App;
