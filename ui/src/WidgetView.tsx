import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { AppWindow, CalendarDays, ChevronDown, Maximize2, Paperclip, Pause, Play, X } from 'lucide-react';
import { Fragment, useEffect, useRef, useState } from 'react';
import { api } from './api';
import { AppPicker } from './components/AppPicker';
import { CheckButton } from './components/CheckButton';
import { WidgetTodo } from './components/WidgetTodo';
import { dueInfo, elapsed, useNow } from './time';
import { pathItems, taskZone, useFileDrop } from './useFileDrop';
import { useActivity } from './useActivity';
import { useTasks } from './useTasks';

const WIDTH = 320;
const MAX_HEIGHT = 560;
/** The card's margin on every side; the window includes it so the shadow isn't clipped. */
const MARGIN = 8;
const EXPANDED_KEY = 'widget.expanded';

/** Whether the to-do list was left open; a per-computer convenience, so local storage is enough. */
function loadExpanded(): boolean {
  try {
    return localStorage.getItem(EXPANDED_KEY) === '1';
  } catch {
    return false;
  }
}

/** The always-on-top card: the tasks in progress, so none of them get forgotten. */
export function WidgetView() {
  const { tasks, error, setError, run } = useTasks();
  const now = useNow();
  const card = useRef<HTMLDivElement>(null);
  const active = (tasks ?? []).filter((t) => t.in_progress);
  const todo = (tasks ?? []).filter((t) => t.status === 'todo' && !t.in_progress);
  const [expanded, setExpanded] = useState(loadExpanded);
  const [dragState, setDragState] = useState<'idle' | 'dragging' | 'over'>('idle');
  const activeZone = useRef<HTMLElement>(null);
  const [pickingFor, setPickingFor] = useState<string | null>(null);
  const { snapshot, activeNow } = useActivity();
  const suggestion = snapshot.suggestion;

  const toggleExpanded = () =>
    setExpanded((cur) => {
      try {
        localStorage.setItem(EXPANDED_KEY, cur ? '0' : '1');
      } catch {
        // Storage unavailable: the choice just won't be remembered.
      }
      return !cur;
    });

  // The window hugs the card, so the transparent area around it never blocks clicks on what's behind.
  useEffect(() => {
    const el = card.current;
    if (!el) return;
    const win = getCurrentWindow();
    const fit = () =>
      void win.setSize(new LogicalSize(WIDTH, Math.min(MAX_HEIGHT, Math.ceil(el.offsetHeight) + MARGIN * 2)));
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
        <button
          className={`icon-btn expand${expanded ? ' on' : ''}`}
          onClick={toggleExpanded}
          aria-expanded={expanded}
          aria-label={expanded ? 'Hide to-do list' : 'Show to-do list'}
          title={expanded ? 'Hide to-do list' : 'Show to-do list'}
        >
          <ChevronDown size={16} />
        </button>
        <button
          className="icon-btn"
          onClick={() => void run(api.openMain)}
          aria-label="Open MultiTasker"
          title="Open MultiTasker"
        >
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

      {suggestion && (
        <div className="suggestion" role="status">
          <span className="live-dot" aria-hidden />
          <div className="suggestion-text">
            <span className="suggestion-title" title={suggestion.title}>
              Working on “{suggestion.title}”?
            </span>
            <span className="suggestion-reason">{suggestion.reason}</span>
          </div>
          <button className="primary small" onClick={() => void run(() => api.setInProgress(suggestion.task_id, true))}>
            <Play size={12} /> Start
          </button>
          <button
            className="icon-btn"
            onClick={() => void run(() => api.dismissSuggestion(suggestion.task_id))}
            aria-label="Not now"
            title="Not now (quiet for 30 minutes)"
          >
            <X size={14} />
          </button>
        </div>
      )}

      <section ref={activeZone} className={`widget-active drag-${dragState}`}>
        {tasks !== null && active.length === 0 && dragState === 'idle' && (
          <button className="widget-empty" onClick={expanded ? undefined : toggleExpanded} disabled={expanded}>
            {expanded
              ? 'Nothing in progress — drag a task up to start it'
              : 'Nothing in progress — pick a task to start'}
          </button>
        )}

        {dragState !== 'idle' && <p className="drop-hint">Drop here to start</p>}

        {active.length > 0 && (
          <ul className="widget-list">
            {active.map((t) => {
              const due = dueInfo(t, now);
              const live = activeNow(t.id, now);
              return (
                <Fragment key={t.id}>
                  <li className={`widget-item${live ? ' live' : ''}`} data-drop={`task:${t.id}`}>
                    <CheckButton
                      done={false}
                      title={t.title}
                      onToggle={() => void run(() => api.update(t.id, { status: 'done' }))}
                    />
                    <div className="widget-task">
                      <span className="widget-title" title={live ? `${t.title} — ${live.reason}` : t.title}>
                        {live && <span className="live-dot" aria-label={`Active now: ${live.reason}`} />}
                        {t.title}
                      </span>
                      <span className="card-meta">
                        {t.started_at && <span className="meta accent">{elapsed(t.started_at, now)}</span>}
                        {t.category && <span className="tag">{t.category}</span>}
                        {due && (
                          <span className={`meta${due.late ? ' late' : due.soon ? ' soon' : ''}`}>
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
                      <button
                        className="icon-btn"
                        onClick={() => attach(t.id)}
                        aria-label={`Attach files to "${t.title}"`}
                        title="Attach files"
                      >
                        <Paperclip size={14} />
                      </button>
                      <button
                        className={`icon-btn${pickingFor === t.id ? ' on' : ''}`}
                        onClick={() => setPickingFor((cur) => (cur === t.id ? null : t.id))}
                        aria-label={`Link an app to "${t.title}"`}
                        title="Link an app — its windows count as working on this"
                      >
                        <AppWindow size={14} />
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
                  {pickingFor === t.id && (
                    <li className="widget-picker">
                      <AppPicker
                        onClose={() => setPickingFor(null)}
                        onPick={(app) => {
                          setPickingFor(null);
                          void run(() =>
                            api.addAttachments(t.id, [{ type: 'app', path: app.exe_path, name: app.name }]),
                          );
                        }}
                      />
                    </li>
                  )}
                </Fragment>
              );
            })}
          </ul>
        )}
      </section>

      {expanded && tasks !== null && (
        <WidgetTodo
          tasks={todo}
          run={run}
          target={activeZone}
          onDragChange={setDragState}
          onCollapse={toggleExpanded}
        />
      )}
    </div>
  );
}
