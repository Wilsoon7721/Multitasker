import { CalendarDays, Laptop, Paperclip, Pause, Play, Timer } from 'lucide-react';
import { api, type Activity, type TaskView } from '../api';
import { dueLabel, elapsed } from '../time';
import type { Run } from '../useTasks';
import { CheckButton } from './CheckButton';

export function TaskCard(props: {
  task: TaskView;
  run: Run;
  now: number;
  selected: boolean;
  live?: Activity;
  where: { text: string; elsewhereOnly: boolean } | null;
  onOpen: () => void;
}) {
  const { task, run, now, selected, live, where, onOpen } = props;
  // Started on another computer only: the button picks it up here instead of pausing it everywhere.
  const pausing = task.in_progress && !where?.elsewhereOnly;
  const done = task.status === 'done';
  const due = task.due_date && !done ? dueLabel(task.due_date) : null;

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
        onToggle={() => void run(() => api.update(task.id, { status: done ? 'todo' : 'done' }))}
      />

      <div className="card-body">
        <span className="card-title">
          {live && <span className="live-dot" title={`Active now: ${live.reason}`} />}
          {task.title}
        </span>
        {task.description && <span className="card-desc">{task.description}</span>}
        <span className="card-meta">
          {task.in_progress && task.started_at && (
            <span className="meta accent">
              <Timer size={12} /> {elapsed(task.started_at, now)}
            </span>
          )}
          {where && (
            <span className="meta device">
              <Laptop size={12} /> {where.text}
            </span>
          )}
          {task.category && <span className="tag">{task.category}</span>}
          {due && (
            <span className={`meta${due.late ? ' late' : ''}`}>
              <CalendarDays size={12} /> {due.text}
            </span>
          )}
          {task.attachments.length > 0 && (
            <span className="meta" title={task.attachments.map((a) => a.name).join('\n')}>
              <Paperclip size={12} /> {task.attachments.length}
            </span>
          )}
        </span>
      </div>

      {!done && (
        <button
          className={`icon-btn start${task.in_progress ? ' on' : ''}`}
          onClick={(e) => {
            e.stopPropagation();
            void run(() => api.setInProgress(task.id, !pausing));
          }}
          aria-label={
            pausing ? `Pause "${task.title}"` : where ? `Continue "${task.title}" here` : `Start "${task.title}"`
          }
          title={pausing ? 'Pause' : where ? 'Continue here' : 'Start — shows in the widget'}
        >
          {pausing ? <Pause size={15} /> : <Play size={15} />}
        </button>
      )}
    </li>
  );
}
