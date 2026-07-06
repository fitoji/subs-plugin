import { SubtitleOverlay } from './components/SubtitleOverlay';
import { useSubtitleDemo } from './hooks/useSubtitleDemo';

function App() {
  // v0.1: Demo mode enabled by default.
  // In v0.2+, this will be toggled based on real audio source availability.
  useSubtitleDemo(true);

  return <SubtitleOverlay />;
}

export default App;
