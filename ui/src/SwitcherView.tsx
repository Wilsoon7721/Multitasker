import { CalendarDays, CornerDownLeft, Plus, Search } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { api, onSwitcherOpened, type TaskView } from './api';
import { dueInfo, useNow, workedTime } from './time';
import { useTasks } from './useTasks';
import { ErrorNote } from './components/ErrorNote';

type Item = { kind: 'task'; task: TaskView } | { kind: 'add'; title: string };

/** In-progress tasks first, then what's due soonest, then the rest as they come. */
function order(a: TaskView, b: TaskView): number {
  if (a.in_progress !== b.in_progress) return a.in_progress ? -1 : 1;
  const due = (t: TaskView) => t.due_at ?? (t.due_date ? `${t.due_date}T23:59` : '9999');
  return due(a).localeCompare(due(b));
}

/** Opened by the hotkey from anywhere: find a task and switch to it, start it alongside, pause it, or add one. */
export function SwitcherView() {
  const { tasks, error, setError, run, retry, retrying, unsaved } = useTasks();
  const now = useNow();
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);

  useEffect(
    () =>
      onSwitcherOpened(() => {
        setQuery('');
        setSelected(0);
        setError(null);
        input.current?.focus();
      }),
    [setError],
  );

  const items: Item[] = useMemo(() => {
    const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const open = (tasks ?? [])
      .filter((t) => t.status === 'todo')
      .filter((t) => words.every((w) => `${t.title} ${t.category ?? ''}`.toLowerCase().includes(w)))
      .sort(order)
      .map((task): Item => ({ kind: 'task', task }));
    const exact = open.some((i) => i.kind === 'task' && i.task.title.toLowerCase() === query.trim().toLowerCase());
    return query.trim() && !exact ? [...open, { kind: 'add', title: query.trim() }] : open;
  }, [tasks, query]);

  useEffect(() => setSelected((s) => Math.min(s, Math.max(0, items.length - 1))), [items.length]);
  useEffect(() => {
    list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
  }, [selected]);

  const done = () => void api.hideSwitcher();
  const act = (item: Item | undefined, how: 'switch' | 'alongside' | 'pause') =>
    void run(async () => {
      if (!item) return;
      if (item.kind === 'add') {
        const created = await api.create({ title: item.title });
        await (how === 'alongside' ? api.setInProgress(created.id, true) : api.switchTask(created.id));
      } else if (how === 'pause') {
        if (item.task.in_progress) await api.setInProgress(item.task.id, false);
      } else if (how === 'alongside' || item.task.in_progress) {
        await api.setInProgress(item.task.id, true);
      } else {
        await api.switchTask(item.task.id);
      }
    }).then((ok) => ok && done());

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') return done();
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const step = e.key === 'ArrowDown' ? 1 : -1;
      setSelected((s) => (items.length ? (s + step + items.length) % items.length : 0));
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      act(items[selected], e.ctrlKey ? 'pause' : e.shiftKey ? 'alongside' : 'switch');
    }
  };

  return (
    <div className="switcher" onKeyDown={onKey}>
      <label className="switcher-search">
        <Search size={18} />
        <input
          ref={input}
          autoFocus
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setSelected(0);
          }}
          placeholder="Switch to a task, or type a new one"
          aria-label="Find a task"
          aria-controls="switcher-list"
        />
      </label>
      {error && (
        <ErrorNote
          className="switcher-error"
          error={error}
          unsaved={unsaved}
          retrying={retrying}
          onRetry={() => void retry()}
        />
      )}
      <ul className="switcher-list" id="switcher-list" role="listbox" ref={list}>
        {items.map((item, i) => {
          const due = item.kind === 'task' ? dueInfo(item.task, now) : null;
          const worked = item.kind === 'task' ? workedTime(item.task, now) : null;
          return (
            <li
              key={item.kind === 'task' ? item.task.id : 'add'}
              role="option"
              aria-selected={i === selected}
              className={`switcher-item${i === selected ? ' selected' : ''}`}
              onMouseMove={() => setSelected(i)}
              onClick={(e) => act(item, e.shiftKey ? 'alongside' : 'switch')}
            >
              {item.kind === 'add' ? (
                <>
                  <Plus size={15} className="switcher-icon" />
                  <span className="switcher-title">Add “{item.title}” and switch to it</span>
                </>
              ) : (
                <>
                  <span className={`dot${item.task.in_progress ? ' on' : ''}`} />
                  <span className="switcher-text">
                    <span className="switcher-title">{item.task.title}</span>
                    <span className="switcher-meta">
                      {item.task.in_progress && <span className="accent">In progress</span>}
                      {worked && <span>{worked}</span>}
                      {item.task.category && <span>{item.task.category}</span>}
                      {due && (
                        <span className={due.late ? 'late' : due.soon ? 'soon' : ''}>
                          <CalendarDays size={11} /> {due.text}
                        </span>
                      )}
                    </span>
                  </span>
                  {i === selected && (
                    <span className="switcher-hint">
                      {item.task.in_progress ? 'Ctrl+Enter to pause' : 'Switch'} <CornerDownLeft size={12} />
                    </span>
                  )}
                </>
              )}
            </li>
          );
        })}
        {tasks !== null && items.length === 0 && (
          <li className="switcher-empty">No open tasks yet. Type to add one.</li>
        )}
      </ul>
      <footer className="switcher-footer">
        <span>
          <kbd>Enter</kbd> switch
        </span>
        <span>
          <kbd>Shift</kbd>+<kbd>Enter</kbd> start alongside
        </span>
        <span>
          <kbd>Ctrl</kbd>+<kbd>Enter</kbd> pause
        </span>
        <span>
          <kbd>Esc</kbd> close
        </span>
      </footer>
    </div>
  );
}
