import { CalendarDays, ListChecks, NotebookPen, PackageCheck, Paperclip, Pause, Play, Timer } from 'lucide-react';
import { api, type Activity, type TaskView } from '../api';
import { dueInfo, workedTime } from '../time';
import { checkCount } from './Checklist';
import type { Run } from '../useTasks';
import { CheckButton } from './CheckButton';

export function TaskCard(props: {
  task: TaskView;
  run: Run;
  now: number;
  selected: boolean;
  live?: Activity;
  onOpen: () => void;
  /** Called once the task has been marked done here. */
  onDone: (task: TaskView) => void;
}) {
  const { task, run, now, selected, live, onOpen, onDone } = props;
  const done = task.status === 'done';
  const due = done ? null : dueInfo(task, now);
  const worked = workedTime(task, now);

  return (
    <li
      className={`card${done ? ' done' : ''}${task.in_progress ? ' active' : ''}${selected ? ' selected' : ''}`}
      data-drop={`task:${task.id}`}
      onClick={onOpen}
      onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && e.target === e.currentTarget && onOpen()}
      tabIndex={0}
      aria-label={`${task.title}, open details`}
    >
      <CheckButton
        done={done}
        title={task.title}
        onToggle={() =>
          void run(() => api.update(task.id, { status: done ? 'todo' : 'done' })).then(
            (ok) => ok && !done && onDone(task),
          )
        }
      />

      <div className="card-body">
        <span className="card-title">
          {live && <span className="live-dot" title={`Active now: ${live.reason}`} />}
          {task.title}
        </span>
        {task.description && <span className="card-desc">{task.description}</span>}
        {task.left_off && !done && (
          <span className="card-left-off" title={`Where you left off: ${task.left_off.text}`}>
            <NotebookPen size={12} /> {task.left_off.text}
          </span>
        )}
        <span className="card-meta">
          {checkCount(task) && (
            <span className="meta" title="Checklist steps done">
              <ListChecks size={12} /> {checkCount(task)}
            </span>
          )}
          {worked && (
            <span className={`meta${task.in_progress ? ' accent' : ''}`} title="Time spent on it in total">
              <Timer size={12} /> {worked}
            </span>
          )}
          {task.category && <span className="tag">{task.category}</span>}
          {due && (
            <span className={`meta${due.late ? ' late' : due.soon ? ' soon' : ''}`}>
              <CalendarDays size={12} /> {due.text}
            </span>
          )}
          {task.attachments.length > 0 && (
            <span className="meta" title={task.attachments.map((a) => a.name).join('\n')}>
              <Paperclip size={12} /> {task.attachments.length}
            </span>
          )}
          {task.deliverables > 0 && (
            <span className="meta" title={`${task.deliverables} handed in`}>
              <PackageCheck size={12} /> {task.deliverables}
            </span>
          )}
        </span>
      </div>

      {!done && (
        <button
          className={`icon-btn start${task.in_progress ? ' on' : ''}`}
          onClick={(e) => {
            e.stopPropagation();
            void run(() => api.setInProgress(task.id, !task.in_progress));
          }}
          aria-label={task.in_progress ? `Pause "${task.title}"` : `Start "${task.title}"`}
          title={task.in_progress ? 'Pause on every computer' : 'Start — shows in the widget on every computer'}
        >
          {task.in_progress ? <Pause size={15} /> : <Play size={15} />}
        </button>
      )}
    </li>
  );
}
