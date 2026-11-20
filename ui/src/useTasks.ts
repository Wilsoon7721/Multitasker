import { useCallback, useEffect, useMemo, useState } from 'react';
import { api, dropPending, onPendingChanged, onTasksChanged, pendingChanges, type TaskView } from './api';

/** Runs an action, surfacing its error instead of throwing. Resolves to whether it worked. */
export type Run = (action: () => Promise<unknown>) => Promise<boolean>;

/** The task list, kept fresh across windows. Errors from any action land in `error`. */
export function useTasks() {
  const [tasks, setTasks] = useState<TaskView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(pendingChanges);

  const refresh = useCallback(async () => {
    // Changes the backend has finished are in this fetch, so their stand-ins can go once it lands.
    const done = pendingChanges().filter((p) => p.settled);
    try {
      setTasks(await api.list());
    } catch (e) {
      setError(String(e));
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
      setError(null);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  }, []);

  return { tasks: shown, error, setError, clearError: () => setError(null), run };
}
