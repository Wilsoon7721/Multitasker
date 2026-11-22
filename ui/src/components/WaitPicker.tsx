import { AppWindow, FileClock, Search } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { api, type JobWhat, type JobWindow } from '../api';

/** Programs that host other programs, where the window's title says more than the program's name. */
const TERMINALS =
  /^(cmd|powershell|pwsh|windowsterminal|wt|conhost|openconsole|mintty|alacritty|wezterm-gui|bash)\.exe$/i;

/** What a waited-on window is called: a terminal by what's running in it, anything else by its program. */
const labelFor = (w: JobWindow) => {
  const named = TERMINALS.test(w.exe) && w.title.trim() ? w.title.split(' - ')[0].trim() : w.name;
  return named.length > 40 ? `${named.slice(0, 39)}…` : named;
};

/** Chooses what a task is waiting on: a window's program, or a file still being written. */
export function WaitPicker({
  onPick,
  onClose,
}: {
  onPick: (label: string, what: JobWhat) => void;
  onClose: () => void;
}) {
  const [windows, setWindows] = useState<JobWindow[] | null>(null);
  const [filter, setFilter] = useState('');
  const panel = useRef<HTMLDivElement>(null);
  // The file picker takes focus away; the panel stays until it answers.
  const choosingFile = useRef(false);

  useEffect(() => {
    void api.jobWindows().then(setWindows, () => setWindows([]));
  }, []);

  useEffect(() => {
    const onDown = (e: PointerEvent) =>
      !choosingFile.current && !panel.current?.contains(e.target as Node) && onClose();
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && (e.stopPropagation(), onClose());
    document.addEventListener('pointerdown', onDown, true);
    document.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('pointerdown', onDown, true);
      document.removeEventListener('keydown', onKey, true);
    };
  }, [onClose]);

  const pickFile = async () => {
    choosingFile.current = true;
    const path = await api.pickFile('Wait for this file to finish saving').catch(() => null);
    choosingFile.current = false;
    if (path) onPick(path.split(/[\\/]/).pop() ?? path, { kind: 'file', path });
    else onClose();
  };

  const q = filter.trim().toLowerCase();
  const shown = (windows ?? []).filter((w) => !q || `${w.name} ${w.exe} ${w.title}`.toLowerCase().includes(q));

  return (
    <div className="app-picker wait-picker" ref={panel} role="dialog" aria-label="Wait for something to finish">
      <label className="app-picker-search">
        <Search size={14} />
        <input
          autoFocus
          placeholder="Wait for a window to finish…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      </label>
      {windows === null ? (
        <p className="app-picker-note">Looking for open windows…</p>
      ) : shown.length === 0 ? (
        <p className="app-picker-note">{windows.length ? 'No open window matches.' : 'No other windows are open.'}</p>
      ) : (
        <ul>
          {shown.map((w) => (
            <li key={w.pid}>
              <button
                onClick={() => onPick(labelFor(w), { kind: 'program', pid: w.pid })}
                title="Done when it closes, or when it has worked and then gone quiet"
                aria-label={`${w.name}: ${w.title}`}
              >
                <AppWindow size={16} className="kind kind-app" />
                <span className="app-picker-text">
                  <span className="app-picker-name">{w.name}</span>
                  <span className="app-picker-title">{w.title}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <button className="wait-picker-file" onClick={() => void pickFile()}>
        <FileClock size={15} /> A file being saved or downloaded…
      </button>
    </div>
  );
}
