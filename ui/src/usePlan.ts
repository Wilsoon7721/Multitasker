import { useCallback, useEffect, useState } from 'react';
import { localDate } from './time';

/** The widget and the main window run in the same app, so they share this storage and both see the plan. */
const planKey = (day = localDate(new Date().toISOString())) => `plan:${day}`;
const skipKey = (day = localDate(new Date().toISOString())) => `plan-skipped:${day}`;
/** Asks the main window to show a view, from the widget. */
export const OPEN_VIEW_KEY = 'open-view';
const CHANGED = 'plan-changed';

function read(): string[] {
  try {
    const saved = JSON.parse(localStorage.getItem(planKey()) ?? '[]');
    return Array.isArray(saved) ? saved.filter((id): id is string => typeof id === 'string') : [];
  } catch {
    return [];
  }
}

function skipped(): boolean {
  try {
    return localStorage.getItem(skipKey()) === '1';
  } catch {
    return false;
  }
}

/** Today's plan: the tasks picked as what today is for, kept on this computer and fresh each day. */
export function usePlan() {
  const [planned, setPlanned] = useState<string[]>(read);
  const [isSkipped, setSkipped] = useState(skipped);

  useEffect(() => {
    const reload = () => {
      setPlanned(read());
      setSkipped(skipped());
    };
    // Another window changing it arrives as a storage event; this one's own changes as its own event.
    window.addEventListener('storage', reload);
    window.addEventListener(CHANGED, reload);
    // A new day starts a new plan even if the window stays open overnight.
    const timer = setInterval(reload, 60_000);
    return () => {
      window.removeEventListener('storage', reload);
      window.removeEventListener(CHANGED, reload);
      clearInterval(timer);
    };
  }, []);

  const write = (key: string, value: string) => {
    try {
      localStorage.setItem(key, value);
    } catch {
      // Without storage the plan lasts only while this window is open.
    }
    window.dispatchEvent(new Event(CHANGED));
  };

  const toggle = useCallback((id: string) => {
    const now = read();
    write(planKey(), JSON.stringify(now.includes(id) ? now.filter((x) => x !== id) : [...now, id]));
  }, []);

  const skip = useCallback(() => write(skipKey(), '1'), []);

  return { planned: new Set(planned), order: planned, isSkipped, toggle, skip };
}

/** Asks the main window to open a view, then brings it up. */
export function requestView(view: string) {
  try {
    localStorage.setItem(OPEN_VIEW_KEY, `${view}:${Date.now()}`);
  } catch {
    // Without storage the main window opens where it was.
  }
}
