import { AppWindow, Search } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { api, type RunningApp } from '../api';

/** Lists programs with a window open so one can be linked to a task. */
export function AppPicker({ onPick, onClose }: { onPick: (app: RunningApp) => void; onClose: () => void }) {
  const [apps, setApps] = useState<RunningApp[] | null>(null);
  const [filter, setFilter] = useState('');
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void api.runningApps().then(setApps, () => setApps([]));
  }, []);

  useEffect(() => {
    const onDown = (e: PointerEvent) => !panel.current?.contains(e.target as Node) && onClose();
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && (e.stopPropagation(), onClose());
    document.addEventListener('pointerdown', onDown, true);
    document.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('pointerdown', onDown, true);
      document.removeEventListener('keydown', onKey, true);
    };
  }, [onClose]);

  const q = filter.trim().toLowerCase();
  const shown = (apps ?? []).filter((a) => !q || `${a.name} ${a.exe} ${a.title}`.toLowerCase().includes(q));

  return (
    <div className="app-picker" ref={panel} role="dialog" aria-label="Link an app">
      <label className="app-picker-search">
        <Search size={14} />
        <input autoFocus placeholder="Link an open app…" value={filter} onChange={(e) => setFilter(e.target.value)} />
      </label>
      {apps === null ? (
        <p className="app-picker-note">Looking for open apps…</p>
      ) : shown.length === 0 ? (
        <p className="app-picker-note">{apps.length ? 'No open app matches.' : 'No other apps have a window open.'}</p>
      ) : (
        <ul>
          {shown.map((a) => (
            <li key={a.exe_path}>
              <button onClick={() => onPick(a)} title={a.exe_path} aria-label={`${a.name}: ${a.title}`}>
                <AppWindow size={16} className="kind kind-app" />
                <span className="app-picker-text">
                  <span className="app-picker-name">{a.name}</span>
                  <span className="app-picker-title">{a.title}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
