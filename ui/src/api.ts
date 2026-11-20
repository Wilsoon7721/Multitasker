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

export type SyncTarget = { kind: 'off' } | { kind: 'folder'; path: string };

export interface SyncDevice {
  id: string;
  name: string;
  last_seen: string;
  this_device: boolean;
}

export interface SyncStatus {
  target: SyncTarget;
  state: 'off' | 'syncing' | 'ok' | 'error';
  last_synced: string | null;
  error: string | null;
  devices: SyncDevice[];
}

export interface FolderSuggestion {
  label: string;
  path: string;
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
  detectionEnabled: () => invoke<boolean>('detection_enabled'),
  syncStatus: () => invoke<SyncStatus>('sync_status'),
  setSyncTarget: (target: SyncTarget) => invoke('set_sync_target', { target }),
  syncNow: () => invoke('sync_now'),
  syncFolderSuggestions: () => invoke<FolderSuggestion[]>('sync_folder_suggestions'),
  /** Asks for a folder with the system picker; null if cancelled. */
  pickFolder: async (): Promise<string | null> => {
    const picked = await open({ directory: true, title: 'Choose a folder to sync through' });
    return typeof picked === 'string' ? picked : null;
  },
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

/** Fires whenever sync starts, finishes or fails. */
export const onSyncStatus = (handler: (status: SyncStatus) => void) => subscribe('sync-status', handler);

/** Fires when the widget is shown or hidden, from anywhere (tray, widget, main window). */
export const onWidgetVisibility = (handler: (visible: boolean) => void) => subscribe('widget-visibility', handler);
