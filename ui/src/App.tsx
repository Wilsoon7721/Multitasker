import { CircleCheckBig, Hash, Inbox, ListTodo, Timer, X } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { api, type NewAttachment, type TaskView } from './api';
import { Composer } from './components/Composer';
import { TaskCard } from './components/TaskCard';
import { TaskDetail } from './components/TaskDetail';
import { TitleBar } from './components/TitleBar';
import { useNow } from './time';
import { useActivity } from './useActivity';
import { useTasks } from './useTasks';
import { pathItems, taskZone, useFileDrop } from './useFileDrop';

type View = 'open' | 'active' | 'todo' | 'done';

const VIEWS: { id: View; label: string; icon: ReactNode; empty: string }[] = [
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

const inView = (t: TaskView, view: View) =>
  view === 'open'
    ? t.status === 'todo'
    : view === 'active'
      ? t.in_progress
      : view === 'todo'
        ? t.status === 'todo' && !t.in_progress
        : t.status === 'done';

export function App() {
  const { tasks, error, setError, clearError, run } = useTasks();
  const [view, setView] = useState<View>('open');
  const [category, setCategory] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pending, setPending] = useState<NewAttachment[]>([]);
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const now = useNow();
  const { activeNow } = useActivity();

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
    else if (target) void run(() => api.addAttachments(target, pathItems(paths)));
    else setError('Drop files onto a task, or onto "Add a task" to start a new one with them.');
  });

  const all = tasks ?? [];
  const categories = useMemo(
    () => [...new Set(all.map((t) => t.category).filter((c): c is string => !!c))].sort((a, b) => a.localeCompare(b)),
    [all],
  );
  const counts = useMemo(
    () =>
      Object.fromEntries(VIEWS.map((v) => [v.id, all.filter((t) => inView(t, v.id)).length])) as Record<View, number>,
    [all],
  );

  const shown = all.filter((t) => inView(t, view) && (!category || t.category === category));
  const selected = all.find((t) => t.id === selectedId) ?? null;
  const current = VIEWS.find((v) => v.id === view)!;
  const close = useCallback(() => setSelectedId(null), []);

  const card = (t: TaskView) => (
    <TaskCard
      key={t.id}
      task={t}
      run={run}
      now={now}
      selected={t.id === selectedId}
      live={activeNow(t.id, now)}
      onOpen={() => setSelectedId(t.id)}
    />
  );

  return (
    <div className="shell">
      <TitleBar onError={setError} />

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
                  onClick={() => setCategory((cur) => (cur === c ? null : c))}
                  aria-pressed={category === c}
                >
                  <Hash size={15} />
                  <span className="nav-label">{c}</span>
                </button>
              ))}
            </>
          )}
        </nav>

        <main className="content">
          <div className="content-inner">
            <header className="content-header">
              <h1>{current.label}</h1>
              {category && (
                <button
                  className="filter-chip"
                  onClick={() => setCategory(null)}
                  aria-label={`Clear filter ${category}`}
                >
                  <Hash size={12} /> {category} <X size={12} />
                </button>
              )}
            </header>

            {error && (
              <div className="banner" role="alert">
                <span>{error}</span>
                <button className="icon-btn" onClick={clearError} aria-label="Dismiss">
                  <X size={15} />
                </button>
              </div>
            )}

            {view !== 'done' && (
              <Composer run={run} categories={categories} pending={pending} setPending={setPending} />
            )}

            {tasks === null ? null : shown.length === 0 ? (
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
              onClose={close}
            />
          </>
        )}
      </div>
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
