import { useEffect, useState } from 'react';
import { api, onTasksChanged, type TaskView, type WorkSession } from './api';

/** Midnight at the start of the local day `days` from today (negative for earlier). */
export function startOfDay(days = 0, now = new Date()): Date {
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() + days);
}

/** Monday at midnight, local time, of the week `weeks` from this one. */
export function startOfWeek(weeks = 0, now = new Date()): Date {
  const back = (now.getDay() + 6) % 7;
  return startOfDay(weeks * 7 - back, now);
}

/** Seconds of `[start, end)` that fall inside `[from, to)`. */
function overlap(start: number, end: number, from: number, to: number): number {
  return Math.max(0, Math.min(end, to) - Math.max(start, from)) / 1000;
}

export interface Spent {
  total: number;
  /** Seconds per task, biggest first. */
  byTask: { task: TaskView | null; taskId: string; seconds: number }[];
  /** Seconds per local day, one entry per day in the range. */
  byDay: { day: Date; seconds: number }[];
}

/** Where time went between two moments: finished stretches of work, plus what's in progress now. */
export function spentBetween(
  sessions: WorkSession[],
  tasks: TaskView[],
  from: Date,
  to: Date,
  now = Date.now(),
): Spent {
  const stretches: { taskId: string; start: number; end: number }[] = sessions.map((s) => ({
    taskId: s.task_id,
    start: new Date(s.started_at).getTime(),
    end: new Date(s.ended_at).getTime(),
  }));
  for (const t of tasks) {
    if (t.in_progress && t.started_at)
      stretches.push({ taskId: t.id, start: new Date(t.started_at).getTime(), end: now });
  }

  const perTask = new Map<string, number>();
  const days: { day: Date; seconds: number }[] = [];
  for (let d = new Date(from); d < to; d = startOfDay(1, d)) days.push({ day: d, seconds: 0 });
  for (const s of stretches) {
    const seconds = overlap(s.start, s.end, from.getTime(), to.getTime());
    if (!seconds) continue;
    perTask.set(s.taskId, (perTask.get(s.taskId) ?? 0) + seconds);
    for (const d of days) d.seconds += overlap(s.start, s.end, d.day.getTime(), startOfDay(1, d.day).getTime());
  }
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const byTask = [...perTask]
    .map(([taskId, seconds]) => ({ taskId, task: byId.get(taskId) ?? null, seconds }))
    .sort((a, b) => b.seconds - a.seconds);
  return { total: byTask.reduce((sum, t) => sum + t.seconds, 0), byTask, byDay: days };
}

/** Finished stretches of work in a range, kept fresh as tasks change. */
export function useSessions(from: Date, to: Date): WorkSession[] {
  const [sessions, setSessions] = useState<WorkSession[]>([]);
  const fromIso = from.toISOString();
  const toIso = to.toISOString();
  useEffect(() => {
    const load = () => void api.workSessions(fromIso, toIso).then(setSessions, () => {});
    load();
    const timer = setInterval(load, 60_000);
    const stop = onTasksChanged(load);
    return () => {
      clearInterval(timer);
      stop();
    };
  }, [fromIso, toIso]);
  return sessions;
}

/** "2h 05m" style, for totals. */
export function hours(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, '0')}m`;
}
