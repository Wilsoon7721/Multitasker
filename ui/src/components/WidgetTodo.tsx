import { CalendarDays, GripVertical, Play } from 'lucide-react';
import { useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from 'react';
import { api, type TaskView } from '../api';
import { dueLabel } from '../time';
import type { Run } from '../useTasks';

/** How far the pointer must move before a press becomes a drag (so clicks still work). */
const DRAG_THRESHOLD = 4;

interface Drag {
  task: TaskView;
  x: number;
  y: number;
  started: boolean;
  overTarget: boolean;
}

/** The widget's expanded to-do list; drag a task up onto the in-progress list, or press play, to start it. */
export function WidgetTodo(props: {
  tasks: TaskView[];
  run: Run;
  target: RefObject<HTMLElement | null>;
  onDragChange: (state: 'idle' | 'dragging' | 'over') => void;
}) {
  const { tasks, run, target, onDragChange } = props;
  const [drag, setDrag] = useState<Drag | null>(null);
  const origin = useRef({ x: 0, y: 0 });

  const start = (task: TaskView) => void run(() => api.setInProgress(task.id, true));

  const isOverTarget = (x: number, y: number) => {
    const el = document.elementFromPoint(x, y);
    return !!(el && target.current?.contains(el));
  };

  // Pointer events, not HTML drag and drop, which Tauri takes over for dropping files in from the desktop.
  const onPointerDown = (e: ReactPointerEvent<HTMLLIElement>, task: TaskView) => {
    if (e.button !== 0 || (e.target as Element).closest('button')) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    origin.current = { x: e.clientX, y: e.clientY };
    setDrag({ task, x: e.clientX, y: e.clientY, started: false, overTarget: false });
  };

  const onPointerMove = (e: ReactPointerEvent) => {
    if (!drag) return;
    const moved = Math.hypot(e.clientX - origin.current.x, e.clientY - origin.current.y) > DRAG_THRESHOLD;
    if (!drag.started && !moved) return;
    const overTarget = isOverTarget(e.clientX, e.clientY);
    setDrag({ ...drag, x: e.clientX, y: e.clientY, started: true, overTarget });
    onDragChange(overTarget ? 'over' : 'dragging');
  };

  const finish = (drop: boolean) => {
    if (drag?.started && drop && drag.overTarget) start(drag.task);
    setDrag(null);
    onDragChange('idle');
  };

  return (
    <section className="widget-todo">
      <h2>
        To do <span className="count">{tasks.length}</span>
        {tasks.length > 0 && <span className="widget-todo-hint">Drag up to start</span>}
      </h2>

      {tasks.length === 0 ? (
        <p className="widget-todo-empty">No other tasks waiting.</p>
      ) : (
        <ul className="widget-list widget-todo-list">
          {tasks.map((t) => {
            const due = t.due_date ? dueLabel(t.due_date) : null;
            const dragging = drag?.started && drag.task.id === t.id;
            return (
              <li
                key={t.id}
                className={`widget-item todo-item${dragging ? ' dragging' : ''}`}
                onPointerDown={(e) => onPointerDown(e, t)}
                onPointerMove={onPointerMove}
                onPointerUp={() => finish(true)}
                onPointerCancel={() => finish(false)}
              >
                <GripVertical size={14} className="grip" />
                <div className="widget-task">
                  <span className="widget-title" title={t.title}>
                    {t.title}
                  </span>
                  {(t.category || due) && (
                    <span className="card-meta">
                      {t.category && <span className="tag">{t.category}</span>}
                      {due && (
                        <span className={`meta${due.late ? ' late' : ''}`}>
                          <CalendarDays size={11} /> {due.text}
                        </span>
                      )}
                    </span>
                  )}
                </div>
                <div className="widget-actions">
                  <button className="icon-btn" onClick={() => start(t)} aria-label={`Start "${t.title}"`} title="Start">
                    <Play size={14} />
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {drag?.started && (
        <div className="drag-ghost" style={{ left: drag.x, top: drag.y }} aria-hidden>
          {drag.task.title}
        </div>
      )}
    </section>
  );
}
