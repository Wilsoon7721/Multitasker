import { AlarmClock, Play, X } from 'lucide-react';
import { api, type TaskView } from '../api';
import type { Run } from '../useTasks';

const HOUR = 3_600_000;
/** A task due at a set time is brought up this long before. */
const TIMED_AHEAD = 6 * HOUR;
/** Overdue tasks stop being brought up after this long; they're plain late by then. */
const LATE_FOR = 2 * 24 * HOUR;

/** When a task is due, as a moment: its time, or the end of its day. */
function dueMoment(t: TaskView): number | null {
  if (t.due_at) return new Date(t.due_at).getTime();
  if (t.due_date) return new Date(`${t.due_date}T23:59:59`).getTime();
  return null;
}

const key = (t: TaskView) => `nudge:${t.id}:${t.due_at ?? t.due_date}`;

function dismissed(t: TaskView): boolean {
  try {
    return localStorage.getItem(key(t)) === '1';
  } catch {
    return false;
  }
}

/** The task most worth a nudge: due soon or just overdue, and not started or worked on at all. */
export function nudgeFor(tasks: TaskView[], now = Date.now()): TaskView | null {
  const today = new Date(now).toDateString();
  const due = tasks
    .filter((t) => t.status === 'todo' && !t.in_progress && t.worked_secs === 0 && !dismissed(t))
    .map((t) => ({ t, at: dueMoment(t) }))
    .filter(({ t, at }) => {
      if (at === null) return false;
      if (at < now) return now - at < LATE_FOR;
      return t.due_at ? at - now <= TIMED_AHEAD : new Date(at).toDateString() === today;
    })
    .sort((a, b) => (a.at ?? 0) - (b.at ?? 0));
  return due[0]?.t ?? null;
}

function when(t: TaskView, now: number): string {
  const at = dueMoment(t) ?? now;
  if (at < now) {
    const hours = Math.floor((now - at) / HOUR);
    return hours < 24 ? 'Overdue' : 'A day overdue';
  }
  if (!t.due_at) return 'Due today';
  const minutes = Math.round((at - now) / 60_000);
  return minutes < 60 ? `Due in ${minutes}m` : `Due in ${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** Brings up a task that's due soon and hasn't been touched, offering to start it now. */
export function DueNudge(props: {
  task: TaskView;
  now: number;
  somethingInProgress: boolean;
  run: Run;
  /** Called once it's put off, so the widget can look again. */
  onLater: () => void;
}) {
  const { task, now, somethingInProgress, run, onLater } = props;

  const later = () => {
    try {
      localStorage.setItem(key(task), '1');
    } catch {
      // Without storage it simply comes back next time.
    }
    onLater();
  };

  return (
    <div className="suggestion due-nudge" role="status">
      <AlarmClock size={14} className="away-icon" aria-hidden />
      <div className="suggestion-text">
        <span className="suggestion-title" title={task.title}>
          {task.title}
        </span>
        <span className="suggestion-reason">{when(task, now)} and not started yet</span>
      </div>
      <button
        className="note-action"
        onClick={() => void run(() => api.switchTask(task.id))}
        aria-label={somethingInProgress ? 'Switch to it' : 'Start it'}
        title={somethingInProgress ? 'Switch to it' : 'Start it'}
      >
        <Play size={14} />
      </button>
      <button className="icon-btn" onClick={later} aria-label="Not now" title="Not now (until its due date changes)">
        <X size={14} />
      </button>
    </div>
  );
}
