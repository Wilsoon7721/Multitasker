import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { SwitcherView } from './SwitcherView';
import { loadPreviewBackend } from './dev/preview';
import './styles.css';

await loadPreviewBackend();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <SwitcherView />
  </StrictMode>,
);
