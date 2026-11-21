import { useEffect, useState } from 'react';

/** "45m", "2h 10m", "3d 4h": how long since `iso`. */
export function elapsed(iso: string, now = Date.now()): string {
  const minutes = Math.max(0, Math.floor((now - new Date(iso).getTime()) / 60_000));
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** "45m", "2h 10m", "3d 4h" for a length of time. */
export function duration(ms: number): string {
  const minutes = Math.max(0, Math.floor(ms / 60_000));
  if (minutes < 1) return '<1m';
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

/** All the time spent on a task, counting the stretch in progress now; null if none yet. */
export function workedTime(
  task: { worked_secs: number; in_progress: boolean; started_at: string | null },
  now = Date.now(),
): string | null {
  const running = task.in_progress && task.started_at ? Math.max(0, now - new Date(task.started_at).getTime()) : 0;
  const total = task.worked_secs * 1000 + running;
  if (total === 0 && !task.in_progress) return null;
  return duration(total);
}

/** "Today", "Tomorrow", "3 days late", "Mon 24 Nov" for a YYYY-MM-DD due date. */
export function dueLabel(date: string, today = new Date()): { text: string; late: boolean } {
  const [y, m, d] = date.split('-').map(Number);
  const due = new Date(y, m - 1, d);
  const start = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const days = Math.round((due.getTime() - start.getTime()) / 86_400_000);

  if (days === 0) return { text: 'Today', late: false };
  if (days === 1) return { text: 'Tomorrow', late: false };
  if (days < 0) return { text: days === -1 ? '1 day late' : `${-days} days late`, late: true };
  return {
    text: due.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' }),
    late: false,
  };
}

/** "45m", "2h 10m", "3d" for a span of milliseconds. */
function span(ms: number): string {
  const minutes = Math.max(1, Math.round(ms / 60_000));
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return minutes % 60 ? `${hours}h ${minutes % 60}m` : `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

const pad = (n: number) => String(n).padStart(2, '0');

/** The local YYYY-MM-DD of a moment. */
export function localDate(iso: string): string {
  const d = new Date(iso);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** The local HH:MM of a moment. */
export function localTime(iso: string): string {
  const d = new Date(iso);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** A local date and time as the exact moment they mean on this computer. */
export function toDueAt(date: string, time: string): string {
  return new Date(`${date}T${time}`).toISOString();
}

export interface Due {
  text: string;
  late: boolean;
  /** Due within the next day, so worth a countdown. */
  soon: boolean;
}

/** What to show for a task's due date: a countdown when it has a time and it's close, otherwise the day. */
export function dueInfo(task: { due_date: string | null; due_at: string | null }, now = Date.now()): Due | null {
  if (task.due_at) {
    const at = new Date(task.due_at);
    const ms = at.getTime() - now;
    if (ms < 0) return { text: `${span(-ms)} late`, late: true, soon: false };
    if (ms < 86_400_000) return { text: `in ${span(ms)}`, late: false, soon: true };
    const time = at.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
    return { text: `${dueLabel(localDate(task.due_at), new Date(now)).text}, ${time}`, late: false, soon: false };
  }
  return task.due_date ? { ...dueLabel(task.due_date, new Date(now)), soon: false } : null;
}

/** Re-renders every `ms` so relative times stay current. */
export function useNow(ms = 30_000): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(id);
  }, [ms]);
  return now;
}
