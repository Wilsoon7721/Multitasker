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
  /** The exact moment it's due, when a time was given. */
  due_at: string | null;
  status: TaskStatus;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  in_progress: boolean;
  started_at: string | null;
  /** Device ids it's in progress on. */
  in_progress_on: string[];
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

export interface S3Settings {
  /** Which service the form was filled in for, e.g. "aws", "r2", "supabase", "custom". */
  provider?: string | null;
  /** Empty for AWS; the service URL for Cloudflare R2, MinIO and others. */
  endpoint: string | null;
  region: string;
  bucket: string;
  /** A folder inside the bucket. */
  prefix: string | null;
  access_key_id: string;
}

export type CloudService = 'google' | 'microsoft';

export type SyncTarget =
  | { kind: 'off' }
  | { kind: 'folder'; path: string }
  | ({ kind: 's3' } & S3Settings)
  | { kind: 'google_drive'; account: string; client_id?: string | null }
  | { kind: 'one_drive'; account: string; client_id?: string | null };

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
  /** The app carrying a shared folder, when known; `running` is null if it couldn't be checked. */
  client: { name: string; running: boolean | null } | null;
}

export type TaskSource = { kind: 'local' } | { kind: 'java_dashboard'; account: string };

export type TaskSourceView = TaskSource & {
  /** The provider actually in use; local if a JavaDashboard sign-in has gone. */
  active: string;
  supports_due_time: boolean;
  /** Whether this build can sign in to WilsoonID. */
  wilsoon_available: boolean;
};

export interface ReminderSettings {
  /** How long before a due time to remind; null for no early reminder. */
  minutes_before: number | null;
  at_due_time: boolean;
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
  due_at?: string | null;
}

/** Only the fields sent change; `null` clears an optional field. */
export interface TaskChanges {
  title?: string;
  description?: string | null;
  category?: string | null;
  due_date?: string | null;
  due_at?: string | null;
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
  /** `secret` is only sent for S3, and only when it changed; it's kept in the system credential store. */
  setSyncTarget: (target: SyncTarget, secret?: string) => invoke('set_sync_target', { target, secret: secret || null }),
  syncNow: () => invoke('sync_now'),
  /** Opens the browser to sign in; resolves with the account once storage answers. */
  signInCloud: (service: CloudService, clientId?: string, clientSecret?: string) =>
    invoke<string>('sign_in_cloud', { service, clientId: clientId || null, clientSecret: clientSecret || null }),
  cancelSignIn: () => invoke('cancel_sign_in'),
  /** Which services this build can sign in to without a client ID being entered. */
  taskSource: () => invoke<TaskSourceView>('task_source'),
  /** Opens the browser to sign in to WilsoonID; resolves with the account once JavaDashboard answers. */
  signInWilsoon: () => invoke<string>('sign_in_wilsoon'),
  useLocalTasks: () => invoke('use_local_tasks'),
  widgetOpacity: () => invoke<number>('widget_opacity'),
  /** Previews an opacity on the widget; `save` keeps it. Resolves with the value actually used. */
  setWidgetOpacity: (opacity: number, save: boolean) => invoke<number>('set_widget_opacity', { opacity, save }),
  reminderSettings: () => invoke<ReminderSettings>('reminder_settings'),
  setReminderSettings: (reminders: ReminderSettings) => invoke('set_reminder_settings', { reminders }),
  builtInClients: () => invoke<Record<CloudService, boolean>>('built_in_clients'),
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

/** Fires when the widget's background opacity changes, with the new percentage. */
export const onWidgetAppearance = (handler: (opacity: number) => void) => subscribe('widget-appearance', handler);

/** Fires when the widget is shown or hidden, from anywhere (tray, widget, main window). */
export const onWidgetVisibility = (handler: (visible: boolean) => void) => subscribe('widget-visibility', handler);
