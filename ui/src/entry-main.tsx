import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { loadPreviewBackend } from './dev/preview';
import './styles.css';

await loadPreviewBackend();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
