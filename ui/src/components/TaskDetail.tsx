import { CalendarDays, Pause, Play, Tag, Trash2, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, type Activity, type TaskChanges, type TaskView } from '../api';
import { elapsed } from '../time';
import type { Run } from '../useTasks';
import { AttachmentList } from './Attachments';
import { AutoTextarea } from './AutoTextarea';
import { CheckButton } from './CheckButton';

/** The side panel for one task. Fields save as you leave them. */
export function TaskDetail(props: {
  task: TaskView;
  run: Run;
  now: number;
  deviceId: string | null;
  live?: Activity;
  where: { text: string; elsewhereOnly: boolean } | null;
  onClose: () => void;
}) {
  const { task, run, now, deviceId, live, where, onClose } = props;
  const [title, setTitle] = useState(task.title);
  const [description, setDescription] = useState(task.description ?? '');
  const [category, setCategory] = useState(task.category ?? '');
  const [confirmDelete, setConfirmDelete] = useState(false);
  const done = task.status === 'done';

  // Follow changes made elsewhere (the widget, another window) and switching tasks.
  useEffect(() => {
    setTitle(task.title);
    setDescription(task.description ?? '');
    setCategory(task.category ?? '');
  }, [task.id, task.title, task.description, task.category]);

  useEffect(() => setConfirmDelete(false), [task.id]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const save = (changes: TaskChanges) => void run(() => api.update(task.id, changes));

  return (
    <aside className="detail" aria-label="Task details" data-drop={`task:${task.id}`}>
      <header className="detail-header">
        <CheckButton done={done} title={task.title} onToggle={() => save({ status: done ? 'todo' : 'done' })} />
        <span className="detail-status">
          {done
            ? 'Done'
            : task.in_progress && task.started_at
              ? `In progress · ${elapsed(task.started_at, now)}${where ? ` · ${where.text.replace(/^On /, 'on ')}` : ''}`
              : 'To do'}
          {live && (
            <span className="detail-live">
              <span className="live-dot" /> Active now — {live.reason}
            </span>
          )}
        </span>
        <button className="icon-btn" onClick={onClose} aria-label="Close details">
          <X size={16} />
        </button>
      </header>

      <div className="detail-body">
        <AutoTextarea
          className="detail-title"
          value={title}
          onChange={(e) => setTitle(e.target.value.replace(/\n/g, ' '))}
          onBlur={() => title.trim() !== task.title && (title.trim() ? save({ title }) : setTitle(task.title))}
          onKeyDown={(e) => e.key === 'Enter' && (e.preventDefault(), e.currentTarget.blur())}
          maxLength={200}
          aria-label="Title"
        />

        <AutoTextarea
          className="detail-desc"
          placeholder="Add details…"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          onBlur={() => description !== (task.description ?? '') && save({ description: description || null })}
          minRows={4}
          maxRows={20}
          aria-label="Details"
        />

        <div className="detail-fields">
          <label className="field-pill" title="Category">
            <Tag size={14} />
            <input
              placeholder="Category"
              list="categories"
              value={category}
              onChange={(e) => setCategory(e.target.value)}
              onBlur={() => category !== (task.category ?? '') && save({ category: category || null })}
              maxLength={60}
              aria-label="Category"
            />
          </label>
          <label className={`field-pill${task.due_date ? '' : ' empty'}`} title="Due date">
            <CalendarDays size={14} />
            <input
              type="date"
              value={task.due_date ?? ''}
              onChange={(e) => save({ due_date: e.target.value || null })}
              aria-label="Due date"
            />
          </label>
        </div>

        <AttachmentList taskId={task.id} attachments={task.attachments} run={run} deviceId={deviceId} />
      </div>

      <footer className="detail-footer">
        {!done && (
          <button
            className={task.in_progress ? 'secondary' : 'primary'}
            onClick={() => void run(() => api.setInProgress(task.id, !task.in_progress))}
          >
            {task.in_progress ? <Pause size={15} /> : <Play size={15} />}
            {task.in_progress ? 'Pause' : 'Start'}
          </button>
        )}
        <span className="spacer" />
        {confirmDelete ? (
          <>
            <button className="secondary" onClick={() => setConfirmDelete(false)}>
              Keep
            </button>
            <button
              className="danger"
              onClick={() => void run(() => api.remove(task.id)).then((ok) => ok && onClose())}
            >
              <Trash2 size={15} /> Delete
            </button>
          </>
        ) : (
          <button className="icon-btn" onClick={() => setConfirmDelete(true)} aria-label="Delete task" title="Delete">
            <Trash2 size={16} />
          </button>
        )}
      </footer>
    </aside>
  );
}
