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

/** Re-renders every `ms` so relative times stay current. */
export function useNow(ms = 30_000): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(id);
  }, [ms]);
  return now;
}
