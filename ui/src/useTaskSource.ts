import { useEffect, useState } from 'react';
import { api, onTasksChanged, type TaskSourceView } from './api';

/** Where tasks come from, refreshed whenever the task list changes (switching sources announces a change). */
export function useTaskSource() {
  const [source, setSource] = useState<TaskSourceView | null>(null);

  useEffect(() => {
    const load = () => void api.taskSource().then(setSource, () => {});
    load();
    return onTasksChanged(load);
  }, []);

  return source;
}
