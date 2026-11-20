import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type TaskStatus = 'todo' | 'done';

export interface TaskView {
  id: string;
  title: string;
  description: string | null;
  category: string | null;
  /** YYYY-MM-DD */
  due_date: string | null;
  status: TaskStatus;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  in_progress: boolean;
  started_at: string | null;
}

export interface NewTask {
  title: string;
  description?: string | null;
  category?: string | null;
  due_date?: string | null;
}

/** Only the fields sent change; `null` clears an optional field. */
export interface TaskChanges {
  title?: string;
  description?: string | null;
  category?: string | null;
  due_date?: string | null;
  status?: TaskStatus;
}

export const api = {
  list: () => invoke<TaskView[]>('list_tasks'),
  create: (task: NewTask) => invoke('create_task', { task }),
  update: (id: string, changes: TaskChanges) => invoke('update_task', { id, changes }),
  remove: (id: string) => invoke('delete_task', { id }),
  setInProgress: (id: string, inProgress: boolean) => invoke('set_in_progress', { id, inProgress }),
  openMain: () => invoke('open_main'),
  toggleWidget: () => invoke<boolean>('toggle_widget'),
};

/** Fires whenever any window changes a task. Returns an unsubscribe function. */
export function onTasksChanged(handler: () => void): () => void {
  const unlisten = listen('tasks-changed', handler);
  return () => void unlisten.then((stop) => stop());
}
