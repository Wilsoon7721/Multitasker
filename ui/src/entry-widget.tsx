import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { WidgetView } from './WidgetView';
import { loadPreviewBackend } from './dev/preview';
import './styles.css';
import { quietBrowserMenu } from './components/ContextMenu';

quietBrowserMenu();

await loadPreviewBackend();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <WidgetView />
  </StrictMode>,
);
