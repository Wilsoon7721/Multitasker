import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useMemo, useState } from 'react';
import type { TaskView } from '../api';
import { hours, spentBetween, startOfDay, startOfWeek, useSessions } from '../useTimeSpent';

type Range = 'today' | 'week';

/** Where time went today or this week: a total, a bar per day, and each task's share. */
export function TimeView({ tasks, now, onOpen }: { tasks: TaskView[]; now: number; onOpen: (id: string) => void }) {
  const [range, setRange] = useState<Range>('today');
  const [offset, setOffset] = useState(0);
  const today = new Date(now);
  const from = range === 'today' ? startOfDay(offset, today) : startOfWeek(offset, today);
  const to = range === 'today' ? startOfDay(offset + 1, today) : startOfWeek(offset + 1, today);
  const sessions = useSessions(from, to);
  const spent = useMemo(() => spentBetween(sessions, tasks, from, to, now), [sessions, tasks, from.getTime(), now]);
  const [hovered, setHovered] = useState<number | null>(null);

  const byCategory = useMemo(() => {
    const totals = new Map<string, number>();
    for (const t of spent.byTask) {
      const name = t.task?.category ?? 'No category';
      totals.set(name, (totals.get(name) ?? 0) + t.seconds);
    }
    return [...totals].sort((a, b) => b[1] - a[1]);
  }, [spent]);

  const label =
    range === 'today'
      ? offset === 0
        ? 'Today'
        : offset === -1
          ? 'Yesterday'
          : from.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'short' })
      : offset === 0
        ? 'This week'
        : offset === -1
          ? 'Last week'
          : `Week of ${from.toLocaleDateString(undefined, { day: 'numeric', month: 'short' })}`;
  // This week only counts the days so far, so Monday's average isn't divided by seven.
  const daysSoFar = Math.max(1, spent.byDay.filter((d) => d.day.getTime() <= now).length);
  const longestDay = Math.max(3600, ...spent.byDay.map((d) => d.seconds));
  const biggest = spent.byTask[0]?.seconds ?? 1;

  return (
    <div className="time-view">
      <div className="time-controls">
        <div className="segmented" role="tablist" aria-label="Range">
          {(['today', 'week'] as Range[]).map((r) => (
            <button
              key={r}
              role="tab"
              aria-selected={range === r}
              className={range === r ? 'on' : ''}
              onClick={() => {
                setRange(r);
                setOffset(0);
              }}
            >
              {r === 'today' ? 'Day' : 'Week'}
            </button>
          ))}
        </div>
        <div className="time-step">
          <button className="icon-btn" onClick={() => setOffset((o) => o - 1)} aria-label="Earlier">
            <ChevronLeft size={16} />
          </button>
          <span>{label}</span>
          <button
            className="icon-btn"
            onClick={() => setOffset((o) => o + 1)}
            disabled={offset >= 0}
            aria-label="Later"
          >
            <ChevronRight size={16} />
          </button>
        </div>
      </div>

      <div className="time-hero">
        <span className="time-total">{hours(spent.total)}</span>
        <span className="time-caption">
          on {spent.byTask.length === 1 ? '1 task' : `${spent.byTask.length} tasks`}
          {range === 'week' && spent.total > 0 && ` · ${hours(spent.total / daysSoFar)} a day on average`}
        </span>
      </div>

      {range === 'week' && (
        <div className="time-chart" role="img" aria-label={`Time spent each day, ${label.toLowerCase()}`}>
          {spent.byDay.map((d, i) => (
            <div
              key={d.day.toISOString()}
              className="time-day"
              onPointerEnter={() => setHovered(i)}
              onPointerLeave={() => setHovered((h) => (h === i ? null : h))}
            >
              <div className="time-bar-area">
                {hovered === i && (
                  <div
                    className="time-tip"
                    role="tooltip"
                    style={{ bottom: `calc(${(d.seconds / longestDay) * 100}% + 6px)` }}
                  >
                    <strong>
                      {d.day.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'short' })}
                    </strong>
                    <span>{d.seconds ? hours(d.seconds) : 'Nothing logged'}</span>
                  </div>
                )}
                <div
                  className={`time-bar${d.seconds ? '' : ' empty'}`}
                  style={{ height: `${Math.max(d.seconds ? 3 : 0, (d.seconds / longestDay) * 100)}%` }}
                />
              </div>
              <span className={`time-day-label${startOfDay(0, today).getTime() === d.day.getTime() ? ' today' : ''}`}>
                {d.day.toLocaleDateString(undefined, { weekday: 'short' })}
              </span>
            </div>
          ))}
        </div>
      )}

      {spent.byTask.length === 0 ? (
        <p className="empty">
          Nothing logged {range === 'today' ? 'this day' : 'this week'}. Start a task and its time adds up here.
        </p>
      ) : (
        <>
          <h3 className="time-heading">By task</h3>
          <table className="time-table">
            <tbody>
              {spent.byTask.map((t) => (
                <tr key={t.taskId} onClick={() => t.task && onOpen(t.taskId)} className={t.task ? 'clickable' : ''}>
                  <td className="time-task">
                    <span className="time-task-title">
                      {t.task?.in_progress && <span className="live-dot" aria-label="In progress" />}
                      {t.task?.title ?? 'A deleted task'}
                    </span>
                    <span className="time-share" aria-hidden>
                      <span style={{ width: `${(t.seconds / biggest) * 100}%` }} />
                    </span>
                  </td>
                  <td className="time-cat">{t.task?.category ?? ''}</td>
                  <td className="time-num">{hours(t.seconds)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {byCategory.length > 1 && (
            <>
              <h3 className="time-heading">By category</h3>
              <ul className="time-categories">
                {byCategory.map(([name, seconds]) => (
                  <li key={name}>
                    <span>{name}</span>
                    <span className="time-num">{hours(seconds)}</span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </>
      )}
    </div>
  );
}
