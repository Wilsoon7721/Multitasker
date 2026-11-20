import { useCallback, useEffect, useState } from 'react';
import { api, onTasksChanged, type TaskView } from './api';

/** Runs an action, surfacing its error instead of throwing. Resolves to whether it worked. */
export type Run = (action: () => Promise<unknown>) => Promise<boolean>;

/** The task list, kept fresh across windows. Errors from any action land in `error`. */
export function useTasks() {
  const [tasks, setTasks] = useState<TaskView[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setTasks(await api.list());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    return onTasksChanged(() => void refresh());
  }, [refresh]);

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

  return { tasks, error, setError, clearError: () => setError(null), run };
}
