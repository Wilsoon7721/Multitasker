import { useEffect, useState } from 'react';
import { api, onSyncStatus, type SyncDevice, type TaskView } from './api';

/** This computer and the others sync knows about, for labelling where tasks are in progress. */
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

  /** "On LAPTOP" when it's only going elsewhere, "Also on LAPTOP" when here too, or null when only here. */
  const whereLabel = (task: TaskView): { text: string; elsewhereOnly: boolean } | null => {
    const others = task.in_progress_on.filter((id) => id !== thisId);
    if (!task.in_progress || others.length === 0) return null;
    const names = [...new Set(others.map(nameOf))].join(', ');
    const here = thisId !== null && task.in_progress_on.includes(thisId);
    return { text: here ? `Also on ${names}` : `On ${names}`, elsewhereOnly: !here };
  };

  return { thisId, whereLabel };
}
