import {
  AppWindow,
  ArrowRightLeft,
  CalendarDays,
  ChevronDown,
  Coffee,
  Maximize2,
  Paperclip,
  Pause,
  Play,
  Timer,
  Undo2,
  X,
} from 'lucide-react';
import { Fragment, useEffect, useMemo, useRef, useState } from 'react';
import { api, onWidgetAppearance } from './api';
import { AppPicker } from './components/AppPicker';
import { CheckButton } from './components/CheckButton';
import { DueNudge, nudgeFor } from './components/DueNudge';
import { LeftOffPrompt, LinkOfferNote, PickUpBar } from './components/PickUp';
import { WidgetTodo } from './components/WidgetTodo';
import { dueInfo, localDate, localTime, useNow, workedTime } from './time';
import { pathItems, taskZone, useFileDrop } from './useFileDrop';
import { useActivity } from './useActivity';
import { hours, spentBetween, startOfDay, useSessions } from './useTimeSpent';
import { useTasks } from './useTasks';
import { useWidgetSize, type Edge } from './useWidgetSize';

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
  const paused = snapshot.paused;
  // Only one nudge at a time, and none while a suggestion already asks for attention.
  const [, nudgesPutOff] = useState(0);
  const nudge = suggestion ? null : nudgeFor(tasks ?? [], now);
  const dayStart = startOfDay(0, new Date(now));
  const todaySessions = useSessions(dayStart, startOfDay(1, dayStart));
  const today = useMemo(
    () => spentBetween(todaySessions, tasks ?? [], dayStart, startOfDay(1, dayStart), now).total,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [todaySessions, tasks, dayStart.getTime(), now],
  );
  const switched = snapshot.switched;
  // A task just paused here, to ask where it was left, and one just picked up, to help get back into it.
  const [leftOffFor, setLeftOffFor] = useState<{ id: string; title: string } | null>(null);
  const [picking, setPicking] = useState<{ id: string; noteOnly: boolean } | null>(null);
  const setPickUpId = (id: string | null, noteOnly = false) => setPicking(id ? { id, noteOnly } : null);
  const pickUp = picking ? ((tasks ?? []).find((t) => t.id === picking.id && t.in_progress) ?? null) : null;
  const [deviceId, setDeviceId] = useState<string | null>(null);

  useEffect(
    () =>
      void api.thisDevice().then(
        (d) => setDeviceId(d.id),
        () => {},
      ),
    [],
  );

  // However a task was started here (widget, main window, switcher), offer to open what it uses.
  const startedHere = useRef<Set<string> | null>(null);
  useEffect(() => {
    if (!tasks || !deviceId) return;
    const now = new Set(tasks.filter((t) => t.in_progress && t.in_progress_on.includes(deviceId)).map((t) => t.id));
    const before = startedHere.current;
    startedHere.current = now;
    if (!before) return;
    const fresh = [...now].filter((id) => !before.has(id) && switched?.to.task_id !== id);
    if (fresh.length) setPickUpId(fresh[fresh.length - 1]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tasks, deviceId]);

  // A switch made on its own asks about the task left behind and helps with the one moved to.
  const switchedKey = switched ? `${switched.to.task_id}:${switched.from.map((f) => f.task_id).join(',')}` : null;
  useEffect(() => {
    if (!switched) return;
    // Its things are already in front, so only a note on where it was left is worth showing.
    setPickUpId(switched.to.task_id, true);
    if (switched.from.length === 1) setLeftOffFor({ id: switched.from[0].task_id, title: switched.from[0].title });
    // Only a new switch should prompt; the snapshot is re-sent often.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [switchedKey]);

  const pause = (t: { id: string; title: string }) =>
    void run(() => api.setInProgress(t.id, false)).then((ok) => {
      if (!ok) return;
      setLeftOffFor({ id: t.id, title: t.title });
      if (picking?.id === t.id) setPickUpId(null);
    });

  const toggleExpanded = () =>
    setExpanded((cur) => {
      try {
        localStorage.setItem(EXPANDED_KEY, cur ? '0' : '1');
      } catch {
        // Storage unavailable: the choice just won't be remembered.
      }
      return !cur;
    });

  useEffect(() => {
    const apply = (opacity: number) => document.documentElement.style.setProperty('--widget-alpha', `${opacity}%`);
    void api.widgetOpacity().then(apply, () => {});
    return onWidgetAppearance(apply);
  }, []);

  const header = useRef<HTMLElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const { size, dragging, grip } = useWidgetSize(header, body);

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
    <div
      className={`widget-card${dragging ? ' resizing' : ''}`}
      ref={card}
      style={{ width: size.width, maxHeight: size.maxHeight }}
    >
      <ResizeEdges grip={grip} />
      <header className="widget-header" ref={header} data-tauri-drag-region>
        <h1 data-tauri-drag-region>
          In progress <span className="count">{active.length}</span>
        </h1>
        {today > 0 && (
          <span className="widget-today" title="Time spent on tasks today" data-tauri-drag-region>
            <Timer size={12} aria-hidden /> {hours(today)}
          </span>
        )}
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

      <div className="widget-scroll">
        <div className="widget-body" ref={body}>
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
              <button
                className="note-action"
                onClick={() =>
                  void run(() => api.switchTask(suggestion.task_id)).then((ok) => ok && setPickUpId(suggestion.task_id))
                }
                aria-label={active.length ? 'Switch to it' : 'Start it'}
                title={active.length ? 'Switch to it' : 'Start it'}
              >
                <Play size={14} />
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

          {nudge && (
            <DueNudge
              key={`due-${nudge.id}`}
              task={nudge}
              now={now}
              somethingInProgress={active.length > 0}
              run={run}
              onLater={() => nudgesPutOff((n) => n + 1)}
            />
          )}

          {snapshot.link_offer && <LinkOfferNote offer={snapshot.link_offer} run={run} />}

          {leftOffFor && (
            <LeftOffPrompt
              key={`left-off-${leftOffFor.id}`}
              task={leftOffFor}
              run={run}
              onClose={() => setLeftOffFor(null)}
            />
          )}

          {pickUp && (
            <PickUpBar
              key={`pick-up-${pickUp.id}`}
              task={pickUp}
              deviceId={deviceId}
              noteOnly={!!picking?.noteOnly}
              run={run}
              onClose={() => setPickUpId(null)}
            />
          )}

          {switched && (
            <div className="suggestion switched" role="status">
              <ArrowRightLeft size={14} className="away-icon" aria-hidden />
              <div className="suggestion-text">
                <span className="suggestion-title" title={switched.to.title}>
                  {switched.from.length ? 'Switched to' : 'Started'} “{switched.to.title}”
                </span>
                <span className="suggestion-reason" title={switched.reason}>
                  {switched.from.length ? `Paused ${switched.from.map((t) => `“${t.title}”`).join(', ')} · ` : ''}
                  {switched.reason}
                </span>
              </div>
              <button
                className="note-action quiet"
                onClick={() => void run(api.undoSwitch)}
                aria-label="Undo the switch"
                title="Undo the switch"
              >
                <Undo2 size={14} />
              </button>
              <button className="icon-btn" onClick={() => void run(api.dismissSwitch)} aria-label="OK" title="OK">
                <X size={14} />
              </button>
            </div>
          )}

          {paused.length > 0 && (
            <div className="suggestion away" role="status">
              <Coffee size={14} className="away-icon" aria-hidden />
              <div className="suggestion-text">
                <span className="suggestion-title" title={paused.map((p) => p.title).join(', ')}>
                  {paused.length === 1 ? `Paused “${paused[0].title}”` : `Paused ${paused.length} tasks`}
                </span>
                <span className="suggestion-reason">{awayText(paused[0].since)}</span>
              </div>
              <button
                className="note-action"
                onClick={() =>
                  void run(async () => Promise.all(paused.map((p) => api.setInProgress(p.task_id, true)))).then(
                    (ok) => ok && setPickUpId(paused[0].task_id),
                  )
                }
                aria-label="Resume"
                title="Resume"
              >
                <Play size={14} />
              </button>
              <button
                className="icon-btn"
                onClick={() => void run(api.dismissPaused)}
                aria-label="Keep paused"
                title="Keep paused"
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
                            <span className="meta accent" title="Time spent on it in total">
                              {workedTime(t, now)}
                            </span>
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
                            onClick={() => pause(t)}
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
              onStarted={(t) => setPickUpId(t.id)}
              target={activeZone}
              onDragChange={setDragState}
              onCollapse={toggleExpanded}
            />
          )}
        </div>
      </div>
    </div>
  );
}

/** When the person left, as a time today or a day and time before that. */
function awayText(since: string): string {
  const day =
    localDate(since) === localDate(new Date().toISOString())
      ? ''
      : `${new Date(since).toLocaleDateString(undefined, { weekday: 'short' })} `;
  return `You were away from ${day}${localTime(since)}`;
}

/** Grips on the card's left and bottom edges; it's pinned top-right, so those are the ones that move. */
function ResizeEdges({ grip }: { grip: ReturnType<typeof useWidgetSize>['grip'] }) {
  const edge = (which: Edge, label: string) => (
    <div
      className={`resize-edge resize-${which.toLowerCase()}`}
      onPointerDown={(e) => void grip.start(which, e)}
      onPointerMove={grip.move}
      onPointerUp={grip.end}
      onPointerCancel={grip.end}
      onLostPointerCapture={grip.end}
      aria-hidden
      title={label}
    />
  );
  return (
    <>
      {edge('West', 'Drag to make it wider or narrower')}
      {edge('South', 'Drag to set how tall it can grow')}
      {edge('SouthWest', 'Drag to resize')}
    </>
  );
}
