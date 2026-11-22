import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api, dropPending, onPendingChanged, onTasksChanged, pendingChanges, type TaskView } from './api';

/** Runs an action, surfacing its error instead of throwing. Resolves to whether it worked. */
export type Run = (action: () => Promise<unknown>) => Promise<boolean>;

/** Errors that come from the connection rather than the request, so the same thing may work a moment later. */
export const isTransient = (error: string) =>
  /couldn't reach|timeout|timed out|connection|network|dns|unreachable|try again/i.test(error);

/** How long to wait before reading the list again after each failed try in a row. */
const RETRY_AFTER = [10_000, 30_000, 60_000];

/** The task list, kept fresh across windows. Errors from any action land in `error`, and can be retried. */
export function useTasks() {
  const [tasks, setTasks] = useState<TaskView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(pendingChanges);
  const [retrying, setRetrying] = useState(false);
  // Changes that failed, in order, kept for Retry.
  const failed = useRef<(() => Promise<unknown>)[]>([]);
  const [unsaved, setUnsaved] = useState(0);
  const [readFailed, setReadFailed] = useState(false);
  const attempts = useRef(0);

  const latest = useRef(0);

  const refresh = useCallback(async () => {
    // Changes the backend has finished are in this fetch, so their stand-ins can go once it lands.
    const done = pendingChanges().filter((p) => p.settled);
    // Two quick changes (added, then started) start two reads; the first can answer last, with the older list.
    const mine = ++latest.current;
    try {
      const list = await api.list();
      if (mine !== latest.current) return;
      setTasks(list);
      setReadFailed(false);
      attempts.current = 0;
      if (!failed.current.length) setError(null);
    } catch (e) {
      if (mine !== latest.current) return;
      setError(String(e));
      setReadFailed(true);
    }
    if (done.length) dropPending(done);
  }, []);

  useEffect(() => {
    void refresh();
    return onTasksChanged(() => void refresh());
  }, [refresh]);

  useEffect(
    () =>
      onPendingChanged(() => {
        const now = pendingChanges();
        setPending(now);
        if (now.some((p) => p.settled)) void refresh();
      }),
    [refresh],
  );

  const shown = useMemo(() => tasks && pending.reduce((list, p) => p.apply(list), tasks), [tasks, pending]);

  // Windows refresh through the change event the backend sends after every change.
  const run: Run = useCallback(async (action) => {
    try {
      await action();
      if (!failed.current.length) setError(null);
      return true;
    } catch (e) {
      failed.current.push(action);
      setUnsaved(failed.current.length);
      setError(String(e));
      return false;
    }
  }, []);

  /** Tries the failed changes again in the order they were made, then reads the list again. */
  const retry = useCallback(async () => {
    setRetrying(true);
    const queue = failed.current.splice(0);
    for (const [i, action] of queue.entries()) {
      try {
        await action();
      } catch (e) {
        // What failed again, and everything after it, waits for the next retry so the order holds.
        failed.current.unshift(...queue.slice(i));
        setError(String(e));
        break;
      }
    }
    setUnsaved(failed.current.length);
    await refresh();
    setRetrying(false);
  }, [refresh]);

  // A list that couldn't be read is read again on its own once the connection may be back.
  useEffect(() => {
    if (!readFailed || !error || !isTransient(error) || failed.current.length) return;
    const wait = RETRY_AFTER[Math.min(attempts.current, RETRY_AFTER.length - 1)];
    const timer = setTimeout(() => {
      attempts.current += 1;
      void refresh();
    }, wait);
    const now = () => void refresh();
    window.addEventListener('online', now);
    window.addEventListener('focus', now);
    return () => {
      clearTimeout(timer);
      window.removeEventListener('online', now);
      window.removeEventListener('focus', now);
    };
  }, [readFailed, error, refresh]);

  const clearError = useCallback(() => {
    failed.current = [];
    setUnsaved(0);
    setError(null);
  }, []);

  return { tasks: shown, error, setError, clearError, run, retry, retrying, unsaved };
}
