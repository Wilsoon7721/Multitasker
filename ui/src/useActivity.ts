import { useEffect, useState } from 'react';
import { api, onActivity, type Activity, type ActivitySnapshot } from './api';

/** Activity this recent counts as "active now". */
const ACTIVE_FOR_MS = 90_000;

const EMPTY: ActivitySnapshot = { enabled: false, active: [], suggestion: null };

export function useActivity() {
  const [snapshot, setSnapshot] = useState<ActivitySnapshot>(EMPTY);

  useEffect(() => {
    void api.activity().then(setSnapshot, () => {});
    return onActivity(setSnapshot);
  }, []);

  /** The task's recent activity, if it was worked on within the last minute and a half. */
  const activeNow = (taskId: string, now: number): Activity | undefined => {
    const a = snapshot.active.find((x) => x.task_id === taskId);
    return a && now - new Date(a.at).getTime() < ACTIVE_FOR_MS ? a : undefined;
  };

  return { snapshot, activeNow };
}
