import { CalendarDays, Clock, Pause, Play, Tag, Trash2, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, type Activity, type TaskChanges, type TaskView } from '../api';
import { elapsed, localDate, localTime, toDueAt } from '../time';
import type { Run } from '../useTasks';
import { AttachmentList } from './Attachments';
import { AutoTextarea } from './AutoTextarea';
import { useTaskSource } from '../useTaskSource';
import { CheckButton } from './CheckButton';

/** The side panel for one task. Fields save as you leave them. */
export function TaskDetail(props: {
  task: TaskView;
  run: Run;
  now: number;
  deviceId: string | null;
  live?: Activity;
  /** The computer it was started on, when that isn't this one. */
  startedOn: string | null;
  onClose: () => void;
}) {
  const { task, run, now, deviceId, live, startedOn, onClose } = props;
  const [title, setTitle] = useState(task.title);
  const [description, setDescription] = useState(task.description ?? '');
  const [category, setCategory] = useState(task.category ?? '');
  const [confirmDelete, setConfirmDelete] = useState(false);
  const done = task.status === 'done';
  // A timed task's date and time are shown in this computer's time zone.
  const dueDate = task.due_at ? localDate(task.due_at) : (task.due_date ?? '');
  const dueTime = task.due_at ? localTime(task.due_at) : '';
  const timesAllowed = useTaskSource()?.supports_due_time ?? true;

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
              ? `In progress · ${elapsed(task.started_at, now)}${startedOn ? ` · started on ${startedOn}` : ''}`
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
          <label className={`field-pill${dueDate ? '' : ' empty'}`} title="Due date">
            <CalendarDays size={14} />
            <input
              type="date"
              value={dueDate}
              onChange={(e) => {
                const date = e.target.value;
                if (!date) save({ due_date: null });
                else save(dueTime ? { due_date: date, due_at: toDueAt(date, dueTime) } : { due_date: date });
              }}
              aria-label="Due date"
            />
          </label>
          {dueDate && timesAllowed && (
            <label className={`field-pill${dueTime ? '' : ' empty'}`} title="Due time (optional)">
              <Clock size={14} />
              <input
                type="time"
                value={dueTime}
                onChange={(e) =>
                  save(
                    e.target.value ? { due_date: dueDate, due_at: toDueAt(dueDate, e.target.value) } : { due_at: null },
                  )
                }
                aria-label="Due time"
              />
            </label>
          )}
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
