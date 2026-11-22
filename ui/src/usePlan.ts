import { useCallback, useEffect, useRef, useState } from 'react';
import { api, onPlanChanged, type DayPlan } from './api';
import { localDate } from './time';

/** Asks the main window to show a view, from the widget. */
export const OPEN_VIEW_KEY = 'open-view';

const today = () => localDate(new Date().toISOString());
const empty = (): DayPlan => ({ day: today(), tasks: [], skipped: false });

/** A plan made before plans were shared lived only in this computer's storage. */
function oldPlan(): DayPlan | null {
  try {
    const day = today();
    const saved = JSON.parse(localStorage.getItem(`plan:${day}`) ?? '[]');
    const tasks = Array.isArray(saved) ? saved.filter((id): id is string => typeof id === 'string') : [];
    const skipped = localStorage.getItem(`plan-skipped:${day}`) === '1';
    for (const key of Object.keys(localStorage)) {
      if (key.startsWith('plan:') || key.startsWith('plan-skipped:')) localStorage.removeItem(key);
    }
    return tasks.length || skipped ? { day, tasks, skipped } : null;
  } catch {
    return null;
  }
}

/** Today's plan: the tasks picked as what today is for, the same on every computer and fresh each day. */
export function usePlan() {
  const [plan, setPlan] = useState<DayPlan>(empty);
  const current = useRef(plan);

  const show = (next: DayPlan) => {
    current.current = next;
    setPlan(next);
  };

  useEffect(() => {
    const load = () =>
      void api.dayPlan().then(
        (saved) => {
          if (saved?.day === today()) return show(saved);
          const old = oldPlan();
          if (old) void api.setDayPlan(old);
          show(old ?? empty());
        },
        () => {},
      );
    load();
    const stop = onPlanChanged(load);
    // A new day starts a new plan even if the window stays open overnight.
    const timer = setInterval(load, 60_000);
    return () => {
      stop();
      clearInterval(timer);
    };
  }, []);

  const save = useCallback((change: (plan: DayPlan) => DayPlan) => {
    const base = current.current.day === today() ? current.current : empty();
    const next = change(base);
    show(next);
    void api.setDayPlan(next);
  }, []);

  const toggle = useCallback(
    (id: string) =>
      save((p) => ({ ...p, tasks: p.tasks.includes(id) ? p.tasks.filter((x) => x !== id) : [...p.tasks, id] })),
    [save],
  );

  const skip = useCallback(() => save((p) => ({ ...p, skipped: true })), [save]);

  return { planned: new Set(plan.tasks), order: plan.tasks, isSkipped: plan.skipped, toggle, skip };
}

/** Asks the main window to show a task's details. */
export const OPEN_TASK_KEY = 'open-task';

export function requestTask(id: string) {
  try {
    localStorage.setItem(OPEN_TASK_KEY, `${id}|${Date.now()}`);
  } catch {
    // Without storage the main window opens where it was.
  }
}

/** Asks the main window to open a view, then brings it up. */
export function requestView(view: string) {
  try {
    localStorage.setItem(OPEN_VIEW_KEY, `${view}:${Date.now()}`);
  } catch {
    // Without storage the main window opens where it was.
  }
}
