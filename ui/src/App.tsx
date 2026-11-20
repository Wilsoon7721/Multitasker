import { useMemo, useState, type FormEvent } from 'react';
import { api, type TaskView } from './api';
import { dueLabel, elapsed, useNow } from './time';
import { useTasks } from './useTasks';

type Run = (action: () => Promise<unknown>) => Promise<boolean>;

export function App() {
  const { tasks, error, clearError, run } = useTasks();
  const [showDone, setShowDone] = useState(false);
  const now = useNow();

  const groups = useMemo(() => {
    const all = tasks ?? [];
    return {
      active: all.filter((t) => t.in_progress),
      todo: all.filter((t) => !t.in_progress && t.status === 'todo'),
      done: all.filter((t) => t.status === 'done'),
      categories: [...new Set(all.map((t) => t.category).filter((c): c is string => !!c))].sort(),
    };
  }, [tasks]);

  return (
    <div className="app">
      <header className="app-header">
        <h1>MultiTasker</h1>
        <button className="ghost" onClick={() => void run(api.toggleWidget)}>
          Show / hide widget
        </button>
      </header>

      {error && (
        <div className="banner" role="alert">
          <span>{error}</span>
          <button className="ghost small" onClick={clearError}>
            Dismiss
          </button>
        </div>
      )}

      <AddTask run={run} categories={groups.categories} />

      {tasks === null ? (
        <p className="muted">Loading…</p>
      ) : (
        <>
          <Section title="In progress" count={groups.active.length} empty="Start a task below and it shows up in the widget.">
            {groups.active.map((t) => (
              <TaskRow key={t.id} task={t} run={run} now={now} categories={groups.categories} />
            ))}
          </Section>

          <Section title="To do" count={groups.todo.length} empty="Nothing left to do.">
            {groups.todo.map((t) => (
              <TaskRow key={t.id} task={t} run={run} now={now} categories={groups.categories} />
            ))}
          </Section>

          {groups.done.length > 0 && (
            <section className="section">
              <button className="section-toggle" onClick={() => setShowDone((s) => !s)} aria-expanded={showDone}>
                {showDone ? '▾' : '▸'} Done <span className="count">{groups.done.length}</span>
              </button>
              {showDone && (
                <ul className="tasks">
                  {groups.done.map((t) => (
                    <TaskRow key={t.id} task={t} run={run} now={now} categories={groups.categories} />
                  ))}
                </ul>
              )}
            </section>
          )}
        </>
      )}
    </div>
  );
}

function Section(props: { title: string; count: number; empty: string; children: React.ReactNode }) {
  return (
    <section className="section">
      <h2>
        {props.title} <span className="count">{props.count}</span>
      </h2>
      {props.count === 0 ? <p className="muted empty">{props.empty}</p> : <ul className="tasks">{props.children}</ul>}
    </section>
  );
}

function AddTask({ run, categories }: { run: Run; categories: string[] }) {
  const [title, setTitle] = useState('');
  const [category, setCategory] = useState('');
  const [due, setDue] = useState('');

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!title.trim()) return;
    const ok = await run(() => api.create({ title, category: category || null, due_date: due || null }));
    if (ok) {
      setTitle('');
      setDue('');
    }
  };

  return (
    <form className="add" onSubmit={submit}>
      <input
        className="grow"
        placeholder="Add a task…"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        maxLength={200}
        aria-label="Task title"
      />
      <input
        placeholder="Category"
        list="categories"
        value={category}
        onChange={(e) => setCategory(e.target.value)}
        maxLength={60}
        aria-label="Category"
      />
      <datalist id="categories">
        {categories.map((c) => (
          <option key={c} value={c} />
        ))}
      </datalist>
      <input type="date" value={due} onChange={(e) => setDue(e.target.value)} aria-label="Due date" />
      <button type="submit" disabled={!title.trim()}>
        Add
      </button>
    </form>
  );
}

function TaskRow({ task, run, now, categories }: { task: TaskView; run: Run; now: number; categories: string[] }) {
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const done = task.status === 'done';
  const due = task.due_date && !done ? dueLabel(task.due_date) : null;

  return (
    <li className={`task${done ? ' done' : ''}${task.in_progress ? ' active' : ''}`}>
      <input
        type="checkbox"
        checked={done}
        onChange={() => void run(() => api.update(task.id, { status: done ? 'todo' : 'done' }))}
        aria-label={done ? 'Mark as not done' : 'Mark as done'}
      />

      {editing ? (
        <EditTask task={task} run={run} categories={categories} close={() => setEditing(false)} />
      ) : (
        <button className="task-body" onClick={() => setEditing(true)} title="Edit">
          <span className="title">{task.title}</span>
          <span className="meta">
            {task.category && <span className="chip">{task.category}</span>}
            {due && <span className={due.late ? 'late' : ''}>{due.text}</span>}
            {task.in_progress && task.started_at && <span className="accent">for {elapsed(task.started_at, now)}</span>}
          </span>
        </button>
      )}

      {!editing && (
        <div className="actions">
          {!done && (
            <button className={task.in_progress ? 'ghost' : ''} onClick={() => void run(() => api.setInProgress(task.id, !task.in_progress))}>
              {task.in_progress ? 'Pause' : 'Start'}
            </button>
          )}
          {confirmDelete ? (
            <>
              <button className="danger" onClick={() => void run(() => api.remove(task.id))}>
                Delete
              </button>
              <button className="ghost" onClick={() => setConfirmDelete(false)}>
                Keep
              </button>
            </>
          ) : (
            <button className="ghost icon" onClick={() => setConfirmDelete(true)} aria-label="Delete task" title="Delete">
              ✕
            </button>
          )}
        </div>
      )}
    </li>
  );
}

function EditTask({ task, run, categories, close }: { task: TaskView; run: Run; categories: string[]; close: () => void }) {
  const [title, setTitle] = useState(task.title);
  const [category, setCategory] = useState(task.category ?? '');
  const [due, setDue] = useState(task.due_date ?? '');

  const save = async (e: FormEvent) => {
    e.preventDefault();
    const ok = await run(() => api.update(task.id, { title, category: category || null, due_date: due || null }));
    if (ok) close();
  };

  return (
    <form className="edit" onSubmit={save} onKeyDown={(e) => e.key === 'Escape' && close()}>
      <input className="grow" value={title} onChange={(e) => setTitle(e.target.value)} maxLength={200} autoFocus aria-label="Task title" />
      <input placeholder="Category" list="categories" value={category} onChange={(e) => setCategory(e.target.value)} maxLength={60} aria-label="Category" />
      <datalist id="categories">
        {categories.map((c) => (
          <option key={c} value={c} />
        ))}
      </datalist>
      <input type="date" value={due} onChange={(e) => setDue(e.target.value)} aria-label="Due date" />
      <button type="submit" disabled={!title.trim()}>
        Save
      </button>
      <button type="button" className="ghost" onClick={close}>
        Cancel
      </button>
    </form>
  );
}
