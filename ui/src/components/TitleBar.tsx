import { getCurrentWindow } from '@tauri-apps/api/window';
import { Copy, Minus, PictureInPicture2, Square, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, onWidgetVisibility } from '../api';

/** The main window's own title bar (the native one is turned off), with the widget toggle and window controls. */
export function TitleBar({ onError }: { onError: (e: string) => void }) {
  const win = getCurrentWindow();
  const [maximized, setMaximized] = useState(false);
  const [widgetVisible, setWidgetVisible] = useState<boolean | null>(null);

  useEffect(() => {
    let stop: (() => void) | undefined;
    const sync = () => void win.isMaximized().then(setMaximized, () => {});
    sync();
    void win.onResized(sync).then((u) => (stop = u));
    return () => stop?.();
  }, [win]);

  useEffect(() => {
    void api.widgetVisible().then(setWidgetVisible, () => {});
    return onWidgetVisibility(setWidgetVisible);
  }, []);

  const toggleWidget = async () => {
    try {
      setWidgetVisible(await api.setWidgetVisible(!widgetVisible));
    } catch (e) {
      onError(String(e));
    }
  };

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="titlebar-brand" data-tauri-drag-region>
        <img src="/icon.svg" alt="" width={16} height={16} />
        <span data-tauri-drag-region>MultiTasker</span>
      </div>

      <div className="titlebar-actions">
        <button
          className={`pill${widgetVisible ? ' on' : ''}`}
          onClick={() => void toggleWidget()}
          aria-pressed={!!widgetVisible}
          title="The always-on-top list of tasks in progress"
        >
          <PictureInPicture2 size={14} />
          {widgetVisible ? 'Hide widget' : 'Show widget'}
        </button>

        <div className="window-controls">
          <button onClick={() => void win.minimize()} aria-label="Minimize">
            <Minus size={15} />
          </button>
          <button onClick={() => void win.toggleMaximize()} aria-label={maximized ? 'Restore' : 'Maximize'}>
            {maximized ? <Copy size={13} /> : <Square size={12} />}
          </button>
          <button className="close" onClick={() => void win.close()} aria-label="Close to tray">
            <X size={16} />
          </button>
        </div>
      </div>
    </header>
  );
}
