import { useCallback, useEffect, useState } from 'react';
import { api, onTasksChanged, type TaskView } from './api';

/** The task list, kept fresh across windows. Errors from any action land in `error`. */
export function useTasks() {
  const [tasks, setTasks] = useState<TaskView[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setTasks(await api.list());
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    return onTasksChanged(() => void refresh());
  }, [refresh]);

  /** Runs an action, surfacing its error instead of throwing. Windows refresh via the change event. */
  const run = useCallback(async (action: () => Promise<unknown>) => {
    try {
      await action();
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  }, []);

  return { tasks, error, clearError: () => setError(null), run };
}
