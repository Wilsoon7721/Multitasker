import { Check, HelpCircle, Pause } from 'lucide-react';
import { useEffect, useRef } from 'react';
import type { Activity, TaskView } from '../api';

const HOUR = 3_600_000;
/** Shared by the widget and Settings, which run in the same app and so see the same storage. */
export const STILL_ON_KEY = 'still-on-after-hours';
export const STILL_ON_CHOICES = [1, 2, 3, 4, 0];
const DEFAULT_HOURS = 2;
const asked = (id: string) => `still-on-asked:${id}`;

export function stillOnHours(): number {
  try {
    const saved = localStorage.getItem(STILL_ON_KEY);
    return saved === null ? DEFAULT_HOURS : Number(saved);
  } catch {
    return DEFAULT_HOURS;
  }
}

/** When each in-progress task last had its things in front, remembered past the hour the snapshot keeps it. */
export function useLastActive(active: Activity[]): Map<string, number> {
  const seen = useRef(new Map<string, number>());
  useEffect(() => {
    for (const a of active) seen.current.set(a.task_id, Math.max(seen.current.get(a.task_id) ?? 0, Date.parse(a.at)));
  }, [active]);
  return seen.current;
}

/** The in-progress task here that's gone longest with none of its things in front, once that's past the setting. */
export function stillOnFor(
  tasks: TaskView[],
  deviceId: string | null,
  lastActive: Map<string, number>,
  openedAt: number,
  now = Date.now(),
): TaskView | null {
  const hours = stillOnHours();
  if (!hours || !deviceId) return null;
  const limit = hours * HOUR;
  const quiet = tasks
    .filter((t) => t.in_progress && t.started_at && t.in_progress_on.includes(deviceId))
    .map((t) => {
      // Activity from before the widget opened isn't known, so the count starts no earlier than that.
      const since = Math.max(Date.parse(t.started_at!), lastActive.get(t.id) ?? 0, openedAt);
      let lastAsked = 0;
      try {
        lastAsked = Number(localStorage.getItem(asked(t.id)) ?? 0);
      } catch {
        // Without storage it may ask again after a restart.
      }
      return { t, quietFor: now - Math.max(since, lastAsked) };
    })
    .filter((q) => q.quietFor >= limit)
    .sort((a, b) => b.quietFor - a.quietFor);
  return quiet[0]?.t ?? null;
}

function remember(id: string) {
  try {
    localStorage.setItem(asked(id), String(Date.now()));
  } catch {
    // Without storage it may ask again after a restart.
  }
}

/** Asks whether a long-running task is still being worked on, since none of its things have been in front for a while. */
export function StillOnNote(props: { task: TaskView; hours: number; onKeep: () => void; onPause: () => void }) {
  const { task, hours, onKeep, onPause } = props;
  return (
    <div className="suggestion still-on" role="status">
      <HelpCircle size={14} className="away-icon" aria-hidden />
      <div className="suggestion-text">
        <span className="suggestion-title" title={task.title}>
          Still on “{task.title}”?
        </span>
        <span className="suggestion-reason">
          Nothing of it has been open for {hours === 1 ? 'an hour' : `${hours} hours`}
        </span>
      </div>
      <button
        className="note-action quiet"
        onClick={() => {
          remember(task.id);
          onKeep();
        }}
        aria-label="Keep going"
        title="Keep going"
      >
        <Check size={14} />
      </button>
      <button
        className="note-action"
        onClick={() => {
          remember(task.id);
          onPause();
        }}
        aria-label={`Pause “${task.title}”`}
        title="Pause it"
      >
        <Pause size={14} />
      </button>
    </div>
  );
}
