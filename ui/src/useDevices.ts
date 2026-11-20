import { useEffect, useState } from 'react';
import { api, onSyncStatus, type SyncDevice, type TaskView } from './api';

/** This computer and the others sync knows about, for saying where a task was started. */
export function useDevices() {
  const [thisId, setThisId] = useState<string | null>(null);
  const [devices, setDevices] = useState<SyncDevice[]>([]);

  useEffect(() => {
    void api.thisDevice().then(
      (d) => setThisId(d.id),
      () => {},
    );
    void api.syncStatus().then(
      (s) => setDevices(s.devices),
      () => {},
    );
    return onSyncStatus((s) => setDevices(s.devices));
  }, []);

  const nameOf = (id: string) => devices.find((d) => d.id === id)?.name ?? 'another computer';

  /** The computer(s) a task in progress was started on, when it wasn't started here. */
  const startedOn = (task: TaskView): string | null => {
    if (!task.in_progress || (thisId !== null && task.in_progress_on.includes(thisId))) return null;
    if (task.started_on_name) return task.started_on_name;
    const names = [...new Set(task.in_progress_on.map(nameOf))];
    return names.length ? names.join(', ') : null;
  };

  return { thisId, startedOn };
}
