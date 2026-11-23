import { Check, ListChecks, Plus, X } from 'lucide-react';
import { useState } from 'react';
import { api, isPendingCheck, type CheckItem, type TaskView } from '../api';
import type { Run } from '../useTasks';

/** "3/7" for a checklist, or nothing when there isn't one. */
export function checkCount(task: TaskView): string | null {
  if (!task.checklist.length) return null;
  return `${task.checklist.filter((c) => c.done).length}/${task.checklist.length}`;
}

/** The first step not done yet. */
export const nextStep = (task: TaskView): CheckItem | null => task.checklist.find((c) => !c.done) ?? null;

/** A task's steps, ticked off as they're done; they live in the task list itself where it has its own. */
export function Checklist({ task, run }: { task: TaskView; run: Run }) {
  const [text, setText] = useState('');
  const count = checkCount(task);

  // The step shows at once, so the next can be typed straight away; one that fails to save comes back to the box.
  const add = async () => {
    const step = text.trim();
    if (!step) return;
    setText('');
    const ok = await run(() => api.addCheck(task.id, step));
    if (!ok) setText((current) => current || step);
  };

  return (
    <section className="attachments checklist">
      <header className="attachments-header">
        <h3>Checklist {count && <span className="count">{count}</span>}</h3>
      </header>
      {task.checklist.length > 0 && (
        <ul className="check-list">
          {task.checklist.map((c) => (
            <li key={c.id} className={`${c.done ? 'done' : ''}${isPendingCheck(c) ? ' saving' : ''}`}>
              <button
                className={`check-box${c.done ? ' on' : ''}`}
                disabled={isPendingCheck(c)}
                onClick={() => void run(() => api.setCheck(task.id, c.id, !c.done))}
                aria-label={c.done ? `Mark “${c.text}” not done` : `Mark “${c.text}” done`}
                aria-pressed={c.done}
              >
                {c.done && <Check size={12} strokeWidth={3} />}
              </button>
              <span className="check-text">{c.text}</span>
              <button
                className="icon-btn check-remove"
                disabled={isPendingCheck(c)}
                onClick={() => void run(() => api.removeCheck(task.id, c.id))}
                aria-label={`Remove “${c.text}”`}
                title="Remove this step"
              >
                <X size={13} />
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        className="check-add"
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        {task.checklist.length ? <Plus size={14} /> : <ListChecks size={14} />}
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={task.checklist.length ? 'Add a step' : 'Break it into steps'}
          aria-label="New step"
          maxLength={300}
        />
      </form>
    </section>
  );
}
