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
  /** Name of the computer it was started on, when the task list keeps it. */
  started_on_name: string | null;
  attachments: Attachment[];
  /** How many pieces of work have been handed in for it. */
  deliverables: number;
  /** Seconds worked before the stretch in progress now, across computers. */
  worked_secs: number;
  /** Where work on it stopped, as noted when it was paused. */
  left_off: { text: string; at: string; device: string } | null;
}

/** A copy of handed-in work, kept as it was each time it was handed in. */
export interface Revision {
  id: string;
  number: number;
  size: number;
  sha256: string;
  /** Where it was handed in from. */
  source: string;
  device: string | null;
  added_at: string;
  /** The copy is on this computer; one handed in elsewhere arrives with sync. */
  available: boolean;
}

export interface Deliverable {
  id: string;
  task_id: string;
  name: string;
  /** Newest first. */
  revisions: Revision[];
}

export interface HandedIn {
  name: string;
  number: number;
  /** It matched the latest version, so nothing new was kept. */
  unchanged: boolean;
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
  /** Shown while it's still being saved. */
  pending?: boolean;
}

export type NewAttachment =
  | { type: 'path'; path: string }
  | { type: 'link'; url: string; name?: string | null }
  | { type: 'app'; path: string; name: string };

/** A program with a window open, offered when linking an app to a task. */
/** What's worth knowing about a file before handing it in. */
export interface FileCheck {
  path: string;
  name: string;
  size: number;
  modified: string | null;
  /** Programs holding it open, whose unsaved changes wouldn't be included. */
  open_in: string[];
  problem: string | null;
  same_as_last: boolean;
}

/** Whether a link opens for someone without your sign-in. */
export interface LinkCheck {
  url: string;
  title: string | null;
  verdict: 'ok' | 'sign_in' | 'broken' | 'unreachable' | 'unchecked';
  message: string;
}

/** This build's version, and an update waiting for a restart, if there is one. */
export interface UpdateInfo {
  current: string;
  ready: string | null;
  /** Development builds don't update themselves or start with Windows. */
  enabled: boolean;
}

/** Whether the widget stays above other windows, and when it steps aside. */
export interface OnTop {
  enabled: boolean;
  except_fullscreen: boolean;
  exceptions: { exe: string; name: string }[];
}

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
  /** Tasks paused because nobody was at this computer, with when they left. */
  paused: { task_id: string; title: string; since: string }[];
  /** A switch detection made on its own, until undone or dismissed. */
  switched: { to: TaskRef; from: TaskRef[]; reason: string } | null;
  /** A page that's been in front a while, offered as a link for the task in progress. */
  link_offer: { task_id: string; task_title: string; url: string; title: string | null } | null;
}

export interface TaskRef {
  task_id: string;
  title: string;
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
  state: 'off' | 'paused' | 'syncing' | 'ok' | 'error';
  last_synced: string | null;
  error: string | null;
  devices: SyncDevice[];
  /** The app carrying a shared folder, when known; `running` is null if it couldn't be checked. */
  client: { name: string; running: boolean | null } | null;
}

export type TaskSource =
  | { kind: 'local' }
  | { kind: 'java_dashboard'; account: string }
  | { kind: 'todoist'; account: string }
  | { kind: 'notion'; account: string; database: string; name: string };

/** Databases shared with a Notion integration, offered when connecting it. */
export interface NotionDatabases {
  workspace: string;
  databases: { id: string; name: string }[];
}

export type TaskSourceView = TaskSource & {
  /** The provider actually in use; local if the online list's sign-in or token has gone. */
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

/** A change shown before the backend confirms it, so slow task lists still feel instant. */
export interface Pending {
  apply: (tasks: TaskView[]) => TaskView[];
  /** The backend has finished; dropped once the list has been fetched again. */
  settled: boolean;
}

const pendings = new Set<Pending>();
const pendingListeners = new Set<() => void>();
const notifyPending = () => pendingListeners.forEach((fn) => fn());

export function pendingChanges(): Pending[] {
  return [...pendings];
}

export function onPendingChanged(fn: () => void): () => void {
  pendingListeners.add(fn);
  return () => pendingListeners.delete(fn);
}

export function dropPending(done: Pending[]) {
  done.forEach((p) => pendings.delete(p));
  notifyPending();
}

function optimistic<T>(apply: Pending['apply'], call: () => Promise<T>): Promise<T> {
  const p: Pending = { apply, settled: false };
  pendings.add(p);
  notifyPending();
  return call().then(
    (value) => {
      p.settled = true;
      notifyPending();
      return value;
    },
    (e) => {
      dropPending([p]);
      throw e;
    },
  );
}

const editTask = (id: string, f: (t: TaskView) => TaskView) => (tasks: TaskView[]) =>
  tasks.map((t) => (t.id === id ? f(t) : t));

let placeholders = 0;
const placeholder = (taskId: string, item: NewAttachment): Attachment => {
  const name =
    item.type === 'path'
      ? (item.path.split(/[\\/]/).filter(Boolean).pop() ?? item.path)
      : item.type === 'link'
        ? (item.name ?? item.url)
        : item.name;
  return {
    id: `pending-${++placeholders}`,
    task_id: taskId,
    kind: item.type === 'path' ? 'file' : item.type,
    name,
    target: item.type === 'link' ? item.url : item.path,
    device: null,
    added_at: new Date().toISOString(),
    pending: true,
  };
};

export const api = {
  list: () => invoke<TaskView[]>('list_tasks'),
  create: (task: NewTask) => invoke<{ id: string }>('create_task', { task }),
  update: (id: string, changes: TaskChanges) => invoke('update_task', { id, changes }),
  remove: (id: string) => invoke('delete_task', { id }),
  setInProgress: (id: string, inProgress: boolean) =>
    optimistic(
      editTask(id, (t) =>
        inProgress
          ? {
              ...t,
              status: 'todo',
              completed_at: null,
              in_progress: true,
              started_at: t.started_at ?? new Date().toISOString(),
            }
          : {
              ...t,
              in_progress: false,
              started_at: null,
              in_progress_on: [],
              started_on_name: null,
              // The stretch just ended counts toward the total straight away.
              worked_secs:
                t.worked_secs +
                (t.started_at ? Math.max(0, Math.floor((Date.now() - new Date(t.started_at).getTime()) / 1000)) : 0),
            },
      ),
      () => invoke('set_in_progress', { id, inProgress }),
    ),
  addAttachments: (id: string, items: NewAttachment[]) =>
    optimistic(
      editTask(id, (t) => ({ ...t, attachments: [...t.attachments, ...items.map((item) => placeholder(id, item))] })),
      () => invoke<Attachment[]>('add_attachments', { id, items }),
    ),
  removeAttachment: (id: string) =>
    optimistic(
      (tasks) => tasks.map((t) => ({ ...t, attachments: t.attachments.filter((a) => a.id !== id) })),
      () => invoke('remove_attachment', { id }),
    ),
  openAttachment: (id: string) => invoke('open_attachment', { id }),
  revealAttachment: (id: string) => invoke('reveal_attachment', { id }),
  listDeliverables: (taskId: string) => invoke<Deliverable[]>('list_deliverables', { taskId }),
  handIn: (taskId: string, paths: string[]) => invoke<HandedIn[]>('hand_in', { taskId, paths }),
  checkHandIn: (taskId: string, paths: string[]) => invoke<FileCheck[]>('check_hand_in', { taskId, paths }),
  checkLink: (url: string) => invoke<LinkCheck>('check_link', { url }),
  handInLink: (taskId: string, url: string, title: string | null) =>
    invoke<HandedIn[]>('hand_in_link', { taskId, url, title }),
  openDeliverable: (revisionId: string) => invoke('open_deliverable', { revisionId }),
  exportDeliverable: (revisionId: string, dest: string) => invoke('export_deliverable', { revisionId, dest }),
  removeDeliverable: (id: string) => invoke('remove_deliverable', { id }),
  thisDevice: () => invoke<Device>('this_device'),
  runningApps: () => invoke<RunningApp[]>('running_apps'),
  widgetOnTop: () => invoke<OnTop>('widget_on_top'),
  updateInfo: () => invoke<UpdateInfo>('update_info'),
  hideSwitcher: () => invoke('hide_switcher'),
  switcherShortcut: () => invoke<string | null>('switcher_shortcut'),
  setSwitcherShortcut: (shortcut: string | null) => invoke('set_switcher_shortcut', { shortcut }),
  checkForUpdates: () => invoke<boolean>('check_for_updates'),
  installUpdate: () => invoke('install_update'),
  startWithWindows: () => invoke<boolean>('start_with_windows'),
  setStartWithWindows: (enabled: boolean) => invoke('set_start_with_windows', { enabled }),
  setWidgetOnTop: (onTop: OnTop) => invoke('set_widget_on_top', { onTop }),
  activity: () => invoke<ActivitySnapshot>('activity'),
  dismissSuggestion: (taskId: string) => invoke('dismiss_suggestion', { taskId }),
  setDetection: (enabled: boolean) => invoke('set_detection', { enabled }),
  detectionEnabled: () => invoke<boolean>('detection_enabled'),
  dismissPaused: () => invoke('dismiss_paused'),
  acceptLinkOffer: () => invoke('accept_link_offer'),
  refuseLinkOffer: () => invoke('refuse_link_offer'),
  setLeftOff: (taskId: string, text: string) => invoke('set_left_off', { taskId, text }),
  openTaskThings: (taskId: string) => invoke<{ opened: number; skipped: string[] }>('open_task_things', { taskId }),
  switchTask: (taskId: string) => invoke('switch_task', { taskId }),
  undoSwitch: () => invoke('undo_switch'),
  dismissSwitch: () => invoke('dismiss_switch'),
  autoSwitch: () => invoke<boolean>('auto_switch'),
  setAutoSwitch: (enabled: boolean) => invoke('set_auto_switch', { enabled }),
  pauseAfterAway: () => invoke<number | null>('pause_after_away'),
  setPauseAfterAway: (minutes: number | null) => invoke('set_pause_after_away', { minutes }),
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
  /** Checks a personal Todoist token and switches to Todoist; resolves with the account's name. */
  connectTodoist: (token: string) => invoke<string>('connect_todoist', { token }),
  /** Checks a Notion integration token and lists the databases shared with it. */
  notionDatabases: (token: string) => invoke<NotionDatabases>('notion_databases', { token }),
  /** Switches to a Notion database; resolves with its name. */
  connectNotion: (token: string, database: string) => invoke<string>('connect_notion', { token, database }),
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
  pickFiles: async (title = 'Attach files'): Promise<string[]> => {
    const picked = await open({ multiple: true, title });
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
export const onSwitcherOpened = (handler: () => void) => subscribe('switcher-opened', handler);
export const onUpdateReady = (handler: (version: string) => void) => subscribe('update-ready', handler);

/** Fires when the widget is shown or hidden, from anywhere (tray, widget, main window). */
export const onWidgetVisibility = (handler: (visible: boolean) => void) => subscribe('widget-visibility', handler);
