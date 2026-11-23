import { Clock, FolderOpen, PackageCheck, NotebookPen, Pause, Play, Tag, Trash2, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, type Activity, type TaskChanges, type TaskView } from '../api';
import { localDate, localTime, toDueAt, workedTime } from '../time';
import type { Run } from '../useTasks';
import { AttachmentList } from './Attachments';
import { AutoTextarea } from './AutoTextarea';
import { useTaskSource } from '../useTaskSource';
import { CheckButton } from './CheckButton';
import { DeliverableList } from './Deliverables';
import { reopenable } from './PickUp';
import { Checklist } from './Checklist';
import { ProjectSection } from './ProjectSection';
import { DueDateField } from './DueDateField';
import { TimeCorrection } from './TimeCorrection';

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
  /** Called once the task has been marked done here. */
  onDone: (task: TaskView) => void;
  onHandIn: (task: TaskView) => void;
}) {
  const { task, run, now, deviceId, live, startedOn, onClose, onDone, onHandIn } = props;
  const [title, setTitle] = useState(task.title);
  const [description, setDescription] = useState(task.description ?? '');
  const [category, setCategory] = useState(task.category ?? '');
  const [leftOff, setLeftOff] = useState(task.left_off?.text ?? '');
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
    setLeftOff(task.left_off?.text ?? '');
  }, [task.id, task.title, task.description, task.category, task.left_off?.text]);

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
        <CheckButton
          done={done}
          title={task.title}
          onToggle={() =>
            void run(() => api.update(task.id, { status: done ? 'todo' : 'done' })).then(
              (ok) => ok && !done && onDone(task),
            )
          }
        />
        <span className="detail-status">
          {done
            ? 'Done'
            : task.in_progress
              ? `In progress · ${workedTime(task, now)} in total${startedOn ? ` · started on ${startedOn}` : ''}`
              : 'To do'}
          {!task.in_progress && workedTime(task, now) && (
            <>
              {' · '}
              <TimeCorrection task={task} label={`${workedTime(task, now)} spent`} run={run} />
            </>
          )}
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
          <DueDateField
            value={dueDate}
            onChange={(date) => {
              if (!date) save({ due_date: null });
              else save(dueTime ? { due_date: date, due_at: toDueAt(date, dueTime) } : { due_date: date });
            }}
          />
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

        {!done && (
          <label className="left-off-field">
            <NotebookPen size={14} />
            <input
              placeholder="Where I left off"
              value={leftOff}
              onChange={(e) => setLeftOff(e.target.value)}
              onBlur={() =>
                leftOff.trim() !== (task.left_off?.text ?? '') && void run(() => api.setLeftOff(task.id, leftOff))
              }
              onKeyDown={(e) => e.key === 'Enter' && e.currentTarget.blur()}
              maxLength={500}
              aria-label="Where I left off"
            />
          </label>
        )}

        <Checklist task={task} run={run} />

        {!done && reopenable(task, deviceId) > 1 && (
          <button
            className="secondary small open-all"
            onClick={() =>
              void run(async () => {
                const r = await api.openTaskThings(task.id);
                if (r.skipped.length) throw r.skipped.join(' ');
              })
            }
          >
            <FolderOpen size={13} /> Open all {reopenable(task, deviceId)}
          </button>
        )}

        <AttachmentList taskId={task.id} attachments={task.attachments} run={run} deviceId={deviceId} />
        <ProjectSection task={task} run={run} />
        <DeliverableList task={task} run={run} now={now} onHandIn={() => onHandIn(task)} />
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
        <button className="secondary" onClick={() => onHandIn(task)} title="Keep a copy of work for this task">
          <PackageCheck size={15} /> Hand in…
        </button>
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
