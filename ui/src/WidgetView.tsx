import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { useEffect, useRef } from 'react';
import { api } from './api';
import { dueLabel, elapsed, useNow } from './time';
import { useTasks } from './useTasks';

const WIDTH = 300;
const MAX_HEIGHT = 520;

/** The always-on-top card: just the tasks in progress, so none of them get forgotten. */
export function WidgetView() {
  const { tasks, error, run } = useTasks();
  const now = useNow();
  const card = useRef<HTMLDivElement>(null);
  const active = (tasks ?? []).filter((t) => t.in_progress);

  // The window hugs the card, so the transparent area around it never blocks clicks on what's behind.
  useEffect(() => {
    const el = card.current;
    if (!el) return;
    const win = getCurrentWindow();
    // The card has an 8px margin on every side; the window includes it so the shadow is not clipped.
    const fit = () => void win.setSize(new LogicalSize(WIDTH, Math.min(MAX_HEIGHT, Math.ceil(el.offsetHeight) + 16)));
    const observer = new ResizeObserver(fit);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  return (
    <div className="widget-card" ref={card}>
      <header className="widget-header" data-tauri-drag-region>
        <span data-tauri-drag-region>
          In progress <span className="count">{active.length}</span>
        </span>
        <button className="ghost small" onClick={() => void run(api.openMain)} title="Open MultiTasker">
          Open
        </button>
      </header>

      {error && <p className="widget-error">{error}</p>}

      {tasks !== null && active.length === 0 && (
        <button className="widget-empty" onClick={() => void run(api.openMain)}>
          Nothing in progress — pick a task
        </button>
      )}

      {active.length > 0 && (
        <ul className="widget-list">
          {active.map((t) => {
            const due = t.due_date ? dueLabel(t.due_date) : null;
            return (
              <li key={t.id}>
                <input
                  type="checkbox"
                  onChange={() => void run(() => api.update(t.id, { status: 'done' }))}
                  aria-label={`Mark "${t.title}" as done`}
                  title="Done"
                />
                <div className="widget-task">
                  <span className="title">{t.title}</span>
                  <span className="meta">
                    {t.started_at && <span>{elapsed(t.started_at, now)}</span>}
                    {t.category && <span className="chip">{t.category}</span>}
                    {due && <span className={due.late ? 'late' : ''}>{due.text}</span>}
                  </span>
                </div>
                <button
                  className="ghost icon"
                  onClick={() => void run(() => api.setInProgress(t.id, false))}
                  aria-label={`Pause "${t.title}"`}
                  title="Pause"
                >
                  ❚❚
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
