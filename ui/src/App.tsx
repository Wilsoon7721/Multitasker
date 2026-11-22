import {
  ChartColumn,
  Check,
  CircleCheckBig,
  Hash,
  Inbox,
  ListTodo,
  Package,
  PanelRight,
  Pause,
  Play,
  Settings,
  Sun,
  Timer,
  Undo2,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { api, type NewAttachment, type TaskView } from './api';
import { Composer } from './components/Composer';
import { handInCandidates } from './components/Deliverables';
import { HandInDialog } from './components/HandInDialog';
import { SettingsPage } from './components/SettingsPage';
import { TaskCard } from './components/TaskCard';
import { TaskDetail } from './components/TaskDetail';
import { TimeView } from './components/TimeView';
import { TitleBar } from './components/TitleBar';
import { useNow } from './time';
import { useActivity } from './useActivity';
import { useDevices } from './useDevices';
import { useTasks } from './useTasks';
import { ErrorNote } from './components/ErrorNote';
import { WindhawkNote } from './components/WindhawkNote';
import { useContextMenu, type MenuEntry } from './components/ContextMenu';
import { OPEN_TASK_KEY, OPEN_VIEW_KEY, usePlan } from './usePlan';
import { pathItems, taskZone, useFileDrop } from './useFileDrop';

type View = 'today' | 'open' | 'active' | 'todo' | 'done' | 'time' | 'settings';
type TaskViewId = Exclude<View, 'settings' | 'time'>;

const VIEWS: { id: TaskViewId; label: string; icon: ReactNode; empty: string }[] = [
  {
    id: 'today',
    label: 'Today',
    icon: <Sun size={16} />,
    empty: "Pick what today is for: press the sun on a task, and it's kept at the top of the widget too.",
  },
  { id: 'open', label: 'All open', icon: <Inbox size={16} />, empty: 'Nothing open. Add a task above.' },
  {
    id: 'active',
    label: 'In progress',
    icon: <Timer size={16} />,
    empty: 'Start a task and it shows up here and in the widget.',
  },
  { id: 'todo', label: 'To do', icon: <ListTodo size={16} />, empty: 'Nothing waiting to be started.' },
  { id: 'done', label: 'Done', icon: <CircleCheckBig size={16} />, empty: 'Finished tasks land here.' },
];

const inView = (t: TaskView, view: TaskViewId, planned: Set<string>) =>
  view === 'today'
    ? planned.has(t.id)
    : view === 'open'
      ? t.status === 'todo'
      : view === 'active'
        ? t.in_progress
        : view === 'todo'
          ? t.status === 'todo' && !t.in_progress
          : t.status === 'done';

export function App() {
  const { tasks, error, setError, clearError, run, retry, retrying, unsaved } = useTasks();
  const plan = usePlan();
  const [view, setView] = useState<View>('open');
  const [category, setCategory] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pending, setPending] = useState<NewAttachment[]>([]);
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const [handingIn, setHandingIn] = useState<{ task: TaskView; paths?: string[] } | null>(null);
  const now = useNow();
  const { activeNow } = useActivity();
  const { startedOn } = useDevices();

  // The widget asks for a view (planning the day) through shared storage.
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === OPEN_VIEW_KEY && e.newValue) setView(e.newValue.split(':')[0] as View);
      if (e.key === OPEN_TASK_KEY && e.newValue) {
        setView('open');
        setSelectedId(e.newValue.split('|')[0]);
      }
    };
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []);

  useEffect(
    () =>
      void api.thisDevice().then(
        (d) => setDeviceId(d.id),
        () => {},
      ),
    [],
  );

  // Files dropped on a task attach to it; on the composer they wait for the new task.
  useFileDrop((paths, zone) => {
    const target = taskZone(zone) ?? (zone ? null : selectedId);
    if (zone === 'composer') setPending((cur) => [...cur, ...pathItems(paths)]);
    else if (zone?.startsWith('deliver:')) {
      const task = all.find((t) => t.id === zone.slice(8));
      if (task) setHandingIn({ task, paths });
    } else if (target) void run(() => api.addAttachments(target, pathItems(paths)));
    else setError('Drop files onto a task, or onto "Add a task" to start a new one with them.');
  });

  const all = tasks ?? [];
  const categories = useMemo(
    () => [...new Set(all.map((t) => t.category).filter((c): c is string => !!c))].sort((a, b) => a.localeCompare(b)),
    [all],
  );
  const counts = useMemo(
    () =>
      Object.fromEntries(VIEWS.map((v) => [v.id, all.filter((t) => inView(t, v.id, plan.planned)).length])) as Record<
        TaskViewId,
        number
      >,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [all, plan.order.join(',')],
  );

  const taskView = view === 'settings' || view === 'time' ? null : view;
  const shown = taskView
    ? all.filter((t) => inView(t, taskView, plan.planned) && (!category || t.category === category))
    : [];
  const selected = all.find((t) => t.id === selectedId) ?? null;
  const current = VIEWS.find((v) => v.id === view);
  const close = useCallback(() => setSelectedId(null), []);
  // Finishing a task with files attached offers to keep copies of them as handed-in work.
  const offerHandIn = (task: TaskView) => {
    if (handInCandidates(task, deviceId).length > 0) setHandingIn({ task });
  };

  const menu = useContextMenu();

  /** Everything that can be done to a task, on right-click. */
  const cardMenu = (t: TaskView): MenuEntry[] => {
    const done = t.status === 'done';
    return [
      { label: 'Open details', icon: <PanelRight size={15} />, onSelect: () => setSelectedId(t.id) },
      null,
      ...(done
        ? []
        : [
            {
              label: t.in_progress ? 'Pause' : 'Start',
              icon: t.in_progress ? <Pause size={15} /> : <Play size={15} />,
              onSelect: () => void run(() => api.setInProgress(t.id, !t.in_progress)),
            },
          ]),
      {
        label: done ? 'Mark not done' : 'Mark done',
        icon: done ? <Undo2 size={15} /> : <Check size={15} />,
        onSelect: () =>
          void run(() => api.update(t.id, { status: done ? 'todo' : 'done' })).then(
            (ok) => ok && !done && offerHandIn(t),
          ),
      },
      { label: 'Hand in…', icon: <Package size={15} />, onSelect: () => setHandingIn({ task: t }) },
      ...(done
        ? []
        : [
            null,
            {
              label: plan.planned.has(t.id) ? "Take off today's plan" : 'Plan for today',
              icon: <Sun size={15} />,
              onSelect: () => plan.toggle(t.id),
            },
          ]),
    ];
  };

  const card = (t: TaskView) => (
    <TaskCard
      key={t.id}
      task={t}
      run={run}
      now={now}
      selected={t.id === selectedId}
      live={activeNow(t.id, now)}
      onOpen={() => setSelectedId(t.id)}
      onDone={offerHandIn}
      planned={plan.planned.has(t.id)}
      onTogglePlan={() => plan.toggle(t.id)}
      onMenu={(e) => menu.show(e, cardMenu(t))}
    />
  );

  return (
    <div className="shell">
      <TitleBar onError={setError} />
      {menu.menu}

      <div className="body">
        <nav className="sidebar" aria-label="Views">
          {VIEWS.map((v) => (
            <button key={v.id} className={`nav${view === v.id ? ' current' : ''}`} onClick={() => setView(v.id)}>
              {v.icon}
              <span className="nav-label">{v.label}</span>
              <span className="nav-count">{counts[v.id] || ''}</span>
            </button>
          ))}

          {categories.length > 0 && (
            <>
              <h3 className="sidebar-heading">Categories</h3>
              {categories.map((c) => (
                <button
                  key={c}
                  className={`nav${category === c ? ' current' : ''}`}
                  onClick={() => {
                    setCategory((cur) => (cur === c ? null : c));
                    if (view === 'settings' || view === 'time') setView('open');
                  }}
                  aria-pressed={category === c}
                >
                  <Hash size={15} />
                  <span className="nav-label">{c}</span>
                </button>
              ))}
            </>
          )}

          <span className="spacer" />
          <button className={`nav${view === 'time' ? ' current' : ''}`} onClick={() => setView('time')}>
            <ChartColumn size={16} />
            <span className="nav-label">Time</span>
          </button>
          <button className={`nav${view === 'settings' ? ' current' : ''}`} onClick={() => setView('settings')}>
            <Settings size={16} />
            <span className="nav-label">Settings</span>
          </button>
        </nav>

        <main className="content">
          <div className="content-inner">
            <header className="content-header">
              <h1>{current?.label ?? (view === 'time' ? 'Where time went' : 'Settings')}</h1>
              {category && taskView && (
                <button
                  className="filter-chip"
                  onClick={() => setCategory(null)}
                  aria-label={`Clear filter ${category}`}
                >
                  <Hash size={12} /> {category} <X size={12} />
                </button>
              )}
            </header>

            <WindhawkNote />

            {error && (
              <ErrorNote
                className="banner"
                error={error}
                unsaved={unsaved}
                retrying={retrying}
                onRetry={() => void retry()}
                onDismiss={clearError}
              />
            )}

            {view === 'settings' && <SettingsPage run={run} />}
            {view === 'time' && <TimeView tasks={all} now={now} onOpen={setSelectedId} />}

            {taskView && taskView !== 'done' && (
              <Composer run={run} categories={categories} pending={pending} setPending={setPending} />
            )}

            {!current || tasks === null ? null : shown.length === 0 ? (
              <p className="empty">{category ? `Nothing in ${category} here.` : current.empty}</p>
            ) : view === 'open' ? (
              <>
                <Group title="In progress" tasks={shown.filter((t) => t.in_progress)} render={card} />
                <Group title="To do" tasks={shown.filter((t) => !t.in_progress)} render={card} />
              </>
            ) : (
              <ul className="cards">{shown.map(card)}</ul>
            )}
          </div>
        </main>

        {selected && (
          <>
            <div className="scrim" onClick={close} />
            <TaskDetail
              task={selected}
              run={run}
              now={now}
              deviceId={deviceId}
              live={activeNow(selected.id, now)}
              startedOn={startedOn(selected)}
              onClose={close}
              onDone={offerHandIn}
              onHandIn={(task) => setHandingIn({ task })}
            />
          </>
        )}
      </div>
      {handingIn && (
        <HandInDialog
          key={handingIn.task.id}
          task={handingIn.task}
          paths={handingIn.paths}
          deviceId={deviceId}
          run={run}
          onClose={() => setHandingIn(null)}
        />
      )}
    </div>
  );
}

function Group({ title, tasks, render }: { title: string; tasks: TaskView[]; render: (t: TaskView) => ReactNode }) {
  if (tasks.length === 0) return null;
  return (
    <section className="group">
      <h2>
        {title} <span className="count">{tasks.length}</span>
      </h2>
      <ul className="cards">{tasks.map(render)}</ul>
    </section>
  );
}
