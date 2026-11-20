import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';

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
  attachments: Attachment[];
}

export type AttachmentKind = 'document' | 'pdf' | 'image' | 'video' | 'folder' | 'file' | 'link' | 'app';

export interface Attachment {
  id: string;
  task_id: string;
  kind: AttachmentKind;
  name: string;
  /** Absolute path (file, folder or program), or URL for links. */
  target: string;
  /** The computer a file path belongs to; null for links. */
  device: string | null;
  added_at: string;
}

export type NewAttachment =
  | { type: 'path'; path: string }
  | { type: 'link'; url: string; name?: string | null }
  | { type: 'app'; path: string; name: string };

/** A program with a window open, offered when linking an app to a task. */
export interface RunningApp {
  exe_path: string;
  exe: string;
  name: string;
  title: string;
}

/** An in-progress task that was just worked on. */
export interface Activity {
  task_id: string;
  at: string;
  reason: string;
}

/** What detection currently sees. */
export interface ActivitySnapshot {
  enabled: boolean;
  active: Activity[];
  suggestion: { task_id: string; title: string; reason: string } | null;
}

export interface Device {
  id: string;
  name: string;
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
  create: (task: NewTask) => invoke<{ id: string }>('create_task', { task }),
  update: (id: string, changes: TaskChanges) => invoke('update_task', { id, changes }),
  remove: (id: string) => invoke('delete_task', { id }),
  setInProgress: (id: string, inProgress: boolean) => invoke('set_in_progress', { id, inProgress }),
  addAttachments: (id: string, items: NewAttachment[]) => invoke<Attachment[]>('add_attachments', { id, items }),
  removeAttachment: (id: string) => invoke('remove_attachment', { id }),
  openAttachment: (id: string) => invoke('open_attachment', { id }),
  revealAttachment: (id: string) => invoke('reveal_attachment', { id }),
  thisDevice: () => invoke<Device>('this_device'),
  runningApps: () => invoke<RunningApp[]>('running_apps'),
  activity: () => invoke<ActivitySnapshot>('activity'),
  dismissSuggestion: (taskId: string) => invoke('dismiss_suggestion', { taskId }),
  setDetection: (enabled: boolean) => invoke('set_detection', { enabled }),
  /** Asks for files with the system picker; empty if cancelled. */
  pickFiles: async (): Promise<string[]> => {
    const picked = await open({ multiple: true, title: 'Attach files' });
    return picked ?? [];
  },
  openMain: () => invoke('open_main'),
  widgetVisible: () => invoke<boolean>('widget_visible'),
  /** Returns whether the widget is now visible. */
  setWidgetVisible: (visible: boolean) => invoke<boolean>('set_widget_visible', { visible }),
};

function subscribe<T>(event: string, handler: (payload: T) => void): () => void {
  const unlisten = listen<T>(event, (e) => handler(e.payload));
  return () => void unlisten.then((stop) => stop());
}

/** Fires whenever any window changes a task. Returns an unsubscribe function. */
export const onTasksChanged = (handler: () => void) => subscribe('tasks-changed', handler);

/** Fires when detection sees a task being worked on, or stops seeing it. */
export const onActivity = (handler: (snapshot: ActivitySnapshot) => void) => subscribe('activity', handler);

/** Fires when the widget is shown or hidden, from anywhere (tray, widget, main window). */
export const onWidgetVisibility = (handler: (visible: boolean) => void) => subscribe('widget-visibility', handler);
