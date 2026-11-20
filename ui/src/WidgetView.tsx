import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { CalendarDays, Maximize2, Paperclip, Pause, X } from 'lucide-react';
import { useEffect, useRef } from 'react';
import { api } from './api';
import { CheckButton } from './components/CheckButton';
import { dueLabel, elapsed, useNow } from './time';
import { pathItems, taskZone, useFileDrop } from './useFileDrop';
import { useTasks } from './useTasks';

const WIDTH = 320;
const MAX_HEIGHT = 560;
/** The card's margin on every side; the window includes it so the shadow isn't clipped. */
const MARGIN = 8;

/** The always-on-top card: the tasks in progress, so none of them get forgotten. */
export function WidgetView() {
  const { tasks, error, setError, run } = useTasks();
  const now = useNow();
  const card = useRef<HTMLDivElement>(null);
  const active = (tasks ?? []).filter((t) => t.in_progress);

  // The window hugs the card, so the transparent area around it never blocks clicks on what's behind.
  useEffect(() => {
    const el = card.current;
    if (!el) return;
    const win = getCurrentWindow();
    const fit = () => void win.setSize(new LogicalSize(WIDTH, Math.min(MAX_HEIGHT, Math.ceil(el.offsetHeight) + MARGIN * 2)));
    const observer = new ResizeObserver(fit);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Drop a file you're working on onto its task; with only one task going, anywhere on the card will do.
  useFileDrop((paths, zone) => {
    const target = taskZone(zone) ?? (active.length === 1 ? active[0].id : null);
    if (target) void run(() => api.addAttachments(target, pathItems(paths)));
    else setError('Drop onto the task the files belong to.');
  });

  const attach = (id: string) =>
    void run(async () => {
      const paths = await api.pickFiles();
      if (paths.length) await api.addAttachments(id, pathItems(paths));
    });

  return (
    <div className="widget-card" ref={card}>
      <header className="widget-header" data-tauri-drag-region>
        <h1 data-tauri-drag-region>
          Tasks In Progress <span className="count">{active.length}</span>
        </h1>
        <button className="icon-btn" onClick={() => void run(api.openMain)} aria-label="Open MultiTasker" title="Open MultiTasker">
          <Maximize2 size={14} />
        </button>
        <button
          className="icon-btn"
          onClick={() => void run(() => api.setWidgetVisible(false))}
          aria-label="Hide widget"
          title="Hide widget (bring it back from the tray)"
        >
          <X size={15} />
        </button>
      </header>

      {error && <p className="widget-error">{error}</p>}

      {tasks !== null && active.length === 0 && (
        <button className="widget-empty" onClick={() => void run(api.openMain)}>
          Nothing in progress — pick a task to start
        </button>
      )}

      {active.length > 0 && (
        <ul className="widget-list">
          {active.map((t) => {
            const due = t.due_date ? dueLabel(t.due_date) : null;
            return (
              <li key={t.id} className="widget-item" data-drop={`task:${t.id}`}>
                <CheckButton done={false} title={t.title} onToggle={() => void run(() => api.update(t.id, { status: 'done' }))} />
                <div className="widget-task">
                  <span className="widget-title" title={t.title}>
                    {t.title}
                  </span>
                  <span className="card-meta">
                    {t.started_at && <span className="meta accent">{elapsed(t.started_at, now)}</span>}
                    {t.category && <span className="tag">{t.category}</span>}
                    {due && (
                      <span className={`meta${due.late ? ' late' : ''}`}>
                        <CalendarDays size={11} /> {due.text}
                      </span>
                    )}
                    {t.attachments.length > 0 && (
                      <span className="meta" title={t.attachments.map((a) => a.name).join('\n')}>
                        <Paperclip size={11} /> {t.attachments.length}
                      </span>
                    )}
                  </span>
                </div>
                <div className="widget-actions">
                  <button className="icon-btn" onClick={() => attach(t.id)} aria-label={`Attach files to "${t.title}"`} title="Attach files">
                    <Paperclip size={14} />
                  </button>
                  <button
                    className="icon-btn"
                    onClick={() => void run(() => api.setInProgress(t.id, false))}
                    aria-label={`Pause "${t.title}"`}
                    title="Pause"
                  >
                    <Pause size={14} />
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
