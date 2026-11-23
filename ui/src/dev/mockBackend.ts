import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import type {
  ActivitySnapshot,
  Attachment,
  AttachmentKind,
  NewAttachment,
  NewTask,
  RunningApp,
  RunView,
  SyncStatus,
  SyncTarget,
  TaskChanges,
  Deliverable,
  TaskSource,
  TaskView,
} from '../api';

// Stands in for the Rust backend with in-memory sample tasks when the UI runs in a plain browser (see README).

const now = Date.now();
const iso = (msAgo: number) => new Date(now - msAgo).toISOString();
const day = (offset: number) => {
  const d = new Date(now + offset * 86_400_000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
};

let tasks: TaskView[] = [
  task('Review AI-generated GS module notes', {
    description: 'Check the summary against the lecture slides.\nFlag anything that needs a second pass.',
    category: 'School',
    due_date: day(0),
    due_at: new Date(now + (2 * 60 + 15) * 60_000).toISOString(),
    in_progress: true,
    started_at: iso(95 * 60_000),
    in_progress_on: ['preview-device'],
    worked_secs: 2 * 3600 + 10 * 60,
  }),
  task('Fix calendar sync bug in JavaDashboard', {
    category: 'Dev',
    in_progress: true,
    started_at: iso(20 * 60_000),
    in_progress_on: ['laptop'],
  }),
  task('Draft project proposal', {
    category: 'School',
    due_date: day(-1),
    worked_secs: 50 * 60,
    left_off: { text: 'Section 3 needs the budget table', at: iso(3600_000), device: 'preview-device' },
  }),
  task('Book dentist appointment', { due_date: day(4) }),
  task('Read chapter 5', {
    category: 'School',
    description: 'Pages 120–158.',
    due_date: day(0),
    due_at: new Date(now + 3 * 3600_000).toISOString(),
  }),
  task('Renew domain', { status: 'done', completed_at: iso(86_400_000) }),
];

function task(title: string, extra: Partial<TaskView> = {}): TaskView {
  return {
    id: crypto.randomUUID(),
    title,
    description: null,
    category: null,
    due_date: null,
    due_at: null,
    status: 'todo',
    created_at: iso(3 * 86_400_000),
    updated_at: iso(3 * 86_400_000),
    completed_at: null,
    in_progress: false,
    started_at: null,
    in_progress_on: [],
    started_on_name: null,
    attachments: [],
    deliverables: 0,
    worked_secs: 0,
    left_off: null,
    checklist: [],
    ...extra,
  };
}

let widgetVisible = true;
const DEVICE = { id: 'preview-device', name: 'PREVIEW-PC' };

// Work handed in on the laptop, still on its way here, to show how that looks.
let deliverables: Deliverable[] = [
  {
    id: 'gs-notes',
    task_id: tasks[0].id,
    name: 'GS module notes.pdf',
    revisions: [
      {
        id: 'gs-notes-2',
        number: 2,
        size: 2_400_000,
        sha256: 'b'.repeat(64),
        source: 'C:\\Users\\you\\GS module notes.pdf',
        device: 'laptop',
        added_at: iso(10 * 60_000),
        available: false,
      },
      {
        id: 'gs-notes-1',
        number: 1,
        size: 2_100_000,
        sha256: 'a'.repeat(64),
        source: 'C:\\Users\\you\\GS module notes.pdf',
        device: 'preview-device',
        added_at: iso(26 * 60 * 60_000),
        available: true,
      },
    ],
  },
];
const countDeliverables = () => {
  for (const t of tasks) t.deliverables = deliverables.filter((d) => d.task_id === t.id).length;
};
countDeliverables();

const KINDS: [RegExp, AttachmentKind][] = [
  [/\.pdf$/i, 'pdf'],
  [/\.(docx?|odt|rtf|txt|md|xlsx?|csv|pptx?)$/i, 'document'],
  [/\.(png|jpe?g|gif|webp|bmp|svg|heic)$/i, 'image'],
  [/\.(mp4|mov|mkv|avi|webm|wmv)$/i, 'video'],
];

const APPS: RunningApp[] = [
  {
    exe_path: 'C:\\Program Files\\Microsoft Office\\WINWORD.EXE',
    exe: 'WINWORD.EXE',
    name: 'Microsoft Word',
    title: 'GS module notes - Word',
  },
  {
    exe_path: 'C:\\Program Files\\Google\\Chrome\\chrome.exe',
    exe: 'chrome.exe',
    name: 'Google Chrome',
    title: 'Claude - Google Chrome',
  },
  {
    exe_path: 'C:\\Users\\you\\AppData\\Local\\Programs\\cursor\\Cursor.exe',
    exe: 'Cursor.exe',
    name: 'Cursor',
    title: 'page.jsx - JavaDashboard - Cursor',
  },
];

function attachment(taskId: string, item: NewAttachment): Attachment {
  const base = { id: crypto.randomUUID(), task_id: taskId, added_at: new Date().toISOString() };
  if (item.type === 'app') return { ...base, kind: 'app', name: item.name, target: item.path, device: DEVICE.id };
  if (item.type === 'link') {
    const url = /^https?:\/\//i.test(item.url) ? item.url : `https://${item.url}`;
    if (!/^https?:\/\/[^\s/]+\.[^\s/]+/i.test(url)) throw `'${item.url}' doesn't look like a web link.`;
    const host = url.replace(/^https?:\/\/(www\.)?/i, '').split(/[/?#]/)[0];
    return { ...base, kind: 'link', name: item.name?.trim() || host, target: url, device: null };
  }
  const name = item.path.split(/[\\/]/).filter(Boolean).pop() ?? item.path;
  const kind = KINDS.find(([re]) => re.test(name))?.[1] ?? (name.includes('.') ? 'file' : 'folder');
  return { ...base, kind, name, target: item.path, device: DEVICE.id };
}

// One unfinished task has a document attached, to try handing work in.
tasks[2].attachments.push(
  attachment(tasks[2].id, { type: 'path', path: 'C:\\Users\\you\\School\\Proposal draft.docx' }),
);

tasks[0].attachments = [
  attachment(tasks[0].id, { type: 'path', path: 'C:\\Users\\you\\Documents\\School\\GS module notes.pdf' }),
  attachment(tasks[0].id, { type: 'path', path: 'C:\\Users\\you\\Documents\\School\\Lecture slides' }),
  attachment(tasks[0].id, { type: 'link', url: 'https://claude.ai/chat/0f3c', name: 'Claude: GS module summary' }),
];

let activity: ActivitySnapshot = {
  enabled: true,
  active: [{ task_id: tasks[0].id, at: new Date().toISOString(), reason: '“GS module notes.pdf” is open' }],
  suggestion: { task_id: tasks[2].id, title: tasks[2].title, reason: '“proposal.docx” is open' },
  switched: {
    to: { task_id: tasks[1].id, title: tasks[1].title },
    from: [{ task_id: tasks[4].id, title: tasks[4].title }],
    reason: '“calendar-sync” is open',
  },
  link_offer: {
    task_id: tasks[0].id,
    task_title: tasks[0].title,
    url: 'https://www.youtube.com/watch?v=abc',
    title: 'Lecture 7 recap - YouTube',
  },
  paused: [{ task_id: tasks[3].id, title: tasks[3].title, since: new Date(Date.now() - 40 * 60_000).toISOString() }],
};
let pauseAfterAway: number | null = 15;
let autoSwitch = true;
let startAfter = 30;
let startup = true;
const projectRuns = new Map<string, RunView>();
const projectAutoruns = new Map<string, string[]>();
let switcherShortcut: string | null = 'Ctrl+Alt+Space';
let onTop = { enabled: true, except_fullscreen: true, exceptions: [{ exe: 'POWERPNT.EXE', name: 'PowerPoint' }] };
// The sample Word document stays open until this is set from the console.
const wordClosed = false;

let sync: SyncStatus = {
  target: { kind: 'folder', path: 'C:\\Users\\you\\OneDrive' },
  state: 'ok',
  last_synced: iso(60_000),
  error: null,
  devices: [
    { id: DEVICE.id, name: DEVICE.name, last_seen: iso(60_000), this_device: true },
    { id: 'laptop', name: 'LAPTOP', last_seen: iso(4 * 60_000), this_device: false },
  ],
  client: { name: 'OneDrive', running: false },
};

function syncTo(target: SyncTarget) {
  if (target.kind === 'off') {
    sync = { target, state: 'off', last_synced: null, error: null, devices: [], client: null };
  } else {
    const seen = new Date().toISOString();
    sync = {
      target,
      state: 'ok',
      last_synced: seen,
      error: null,
      devices: [
        { id: DEVICE.id, name: DEVICE.name, last_seen: seen, this_device: true },
        { id: 'laptop', name: 'LAPTOP', last_seen: iso(4 * 60_000), this_device: false },
      ],
      client: target.kind === 'folder' ? { name: 'OneDrive', running: true } : null,
    };
  }
  setTimeout(() => void emit('sync-status', sync), 0);
}

let signInCancelled = false;
let taskSource: TaskSource = { kind: 'local' };
let widgetOpacity = 94;
let reminders = { minutes_before: 15 as number | null, at_due_time: true };

function activityChanged() {
  setTimeout(() => void emit('activity', activity), 0);
}

const blank = (v: string | null | undefined) => (v && v.trim() ? v.trim() : null);

function find(id: string): TaskView {
  const t = tasks.find((t) => t.id === id);
  if (!t) throw `task ${id} was not found`;
  return t;
}

function changed() {
  setTimeout(() => void emit('tasks-changed'), 0);
}

const label =
  new URLSearchParams(location.search).get('window') ??
  (location.pathname.includes('widget') ? 'widget' : location.pathname.includes('switcher') ? 'switcher' : 'main');
mockWindows(label);

mockIPC(
  (cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case 'list_tasks':
        return structuredClone(tasks);

      case 'create_task': {
        const input = a.task as NewTask;
        if (!input.title.trim()) throw 'A task needs a title.';
        const t = task(input.title.trim(), {
          description: blank(input.description),
          category: blank(input.category),
          due_date: input.due_date ?? null,
          due_at: input.due_at ?? null,
          created_at: new Date().toISOString(),
        });
        tasks.push(t);
        changed();
        return t;
      }

      case 'update_task': {
        const t = find(a.id as string);
        const c = a.changes as TaskChanges;
        if (c.title !== undefined) {
          if (!c.title.trim()) throw 'A task needs a title.';
          t.title = c.title.trim();
        }
        if (c.description !== undefined) t.description = blank(c.description);
        if (c.category !== undefined) t.category = blank(c.category);
        if (c.due_date !== undefined) t.due_date = c.due_date;
        if (c.due_date === null) t.due_at = null;
        if (c.due_at !== undefined) t.due_at = c.due_at;
        if (c.status !== undefined && c.status !== t.status) {
          t.status = c.status;
          t.completed_at = c.status === 'done' ? new Date().toISOString() : null;
          if (c.status === 'done') Object.assign(t, { in_progress: false, started_at: null, in_progress_on: [] });
        }
        changed();
        return t;
      }

      case 'delete_task':
        find(a.id as string);
        tasks = tasks.filter((t) => t.id !== a.id);
        changed();
        return null;

      // Switching starts the task; pausing what came before is the app's job.
      case 'switch_task':
        Object.assign(a, { id: a.taskId, inProgress: true });
      // falls through
      case 'correct_worked': {
        find(a.taskId as string).worked_secs = a.totalSecs as number;
        changed();
        return null;
      }

      case 'set_in_progress': {
        const t = find(a.id as string);
        if (a.inProgress) {
          Object.assign(t, {
            status: 'todo',
            completed_at: null,
            in_progress: true,
            started_at: t.started_at ?? new Date().toISOString(),
            in_progress_on: [...new Set([...t.in_progress_on, DEVICE.id])],
          });
          if (activity.suggestion?.task_id === t.id || activity.paused.some((p) => p.task_id === t.id)) {
            const suggestion = activity.suggestion?.task_id === t.id ? null : activity.suggestion;
            activity = { ...activity, suggestion, paused: activity.paused.filter((p) => p.task_id !== t.id) };
            activityChanged();
          }
        } else {
          const ran = t.started_at ? Math.floor((Date.now() - new Date(t.started_at).getTime()) / 1000) : 0;
          Object.assign(t, {
            in_progress: false,
            started_at: null,
            in_progress_on: [],
            worked_secs: t.worked_secs + ran,
          });
        }
        changed();
        return null;
      }

      case 'add_attachments': {
        const t = find(a.id as string);
        const added = (a.items as NewAttachment[]).map((item) => attachment(t.id, item));
        for (const att of added) if (!t.attachments.some((x) => x.target === att.target)) t.attachments.push(att);
        changed();
        return added;
      }

      case 'remove_attachment':
        for (const t of tasks) t.attachments = t.attachments.filter((x) => x.id !== a.id);
        changed();
        return null;

      case 'open_attachment':
      case 'reveal_attachment':
        console.info(`[preview] ${cmd}`, a.id);
        return null;

      case 'list_deliverables':
        return deliverables.filter((d) => d.task_id === a.taskId);

      // A link is kept as an internet shortcut, so it's handed in like a file.
      case 'hand_in_link':
        a.paths = [`${String(a.title ?? a.url).slice(0, 60)}.url`];
      // falls through
      case 'hand_in': {
        const t = find(a.taskId as string);
        const result = (a.paths as string[]).map((path) => {
          const name = path.split(/[\\/]/).pop() ?? path;
          let d = deliverables.find((x) => x.task_id === t.id && x.name.toLowerCase() === name.toLowerCase());
          if (!d) {
            d = { id: crypto.randomUUID(), task_id: t.id, name, revisions: [] };
            deliverables.push(d);
          }
          const number = (d.revisions[0]?.number ?? 0) + 1;
          d.revisions.unshift({
            id: crypto.randomUUID(),
            number,
            size: 20_000 + Math.round(Math.random() * 2_000_000),
            sha256: crypto.randomUUID(),
            source: path,
            device: DEVICE.id,
            added_at: new Date().toISOString(),
            available: true,
          });
          return { name, number, unchanged: false };
        });
        countDeliverables();
        changed();
        return result;
      }

      case 'remove_deliverable':
        deliverables = deliverables.filter((d) => d.id !== a.id);
        countDeliverables();
        changed();
        return null;

      case 'open_deliverable':
      case 'export_deliverable':
        console.info(`[preview] ${cmd}`, a.revisionId, a.dest ?? '');
        return null;

      case 'this_device':
        return DEVICE;

      case 'sync_status':
        return sync;

      case 'set_sync_target':
        syncTo(a.target as SyncTarget);
        return null;

      case 'widget_opacity':
        return widgetOpacity;

      case 'set_widget_opacity': {
        const value = Math.min(100, Math.max(30, Number(a.opacity)));
        if (a.save) widgetOpacity = value;
        setTimeout(() => void emit('widget-appearance', value), 0);
        return value;
      }

      case 'task_source':
        return {
          ...taskSource,
          active:
            taskSource.kind === 'local'
              ? 'sqlite'
              : taskSource.kind === 'java_dashboard'
                ? 'javadashboard'
                : taskSource.kind,
          supports_due_time: taskSource.kind !== 'java_dashboard',
          wilsoon_available: true,
        };

      case 'sign_in_wilsoon':
        signInCancelled = false;
        return new Promise((resolve, reject) =>
          setTimeout(() => {
            if (signInCancelled) return reject('Sign-in was cancelled.');
            taskSource = { kind: 'java_dashboard', account: 'you@wilsoon.dev' };
            changed();
            resolve('you@wilsoon.dev');
          }, 1500),
        );

      case 'connect_todoist':
        if (!String(a.token).trim()) throw 'Paste your Todoist API token.';
        taskSource = { kind: 'todoist', account: 'Preview Person' };
        changed();
        return 'Preview Person';

      case 'notion_databases':
        if (!String(a.token).trim()) throw "Paste your Notion integration's token.";
        return {
          workspace: "Preview's Space",
          databases: [
            { id: 'db-1', name: 'Tasks' },
            { id: 'db-2', name: 'Reading list' },
          ],
        };

      case 'connect_notion':
        taskSource = { kind: 'notion', account: "Preview's Space", database: String(a.database), name: 'Tasks' };
        changed();
        return 'Tasks';

      case 'use_local_tasks':
        taskSource = { kind: 'local' };
        changed();
        return null;

      case 'reminder_settings':
        return reminders;

      case 'set_reminder_settings':
        reminders = a.reminders as typeof reminders;
        return null;

      case 'built_in_clients':
        return { google: false, microsoft: true };

      case 'sign_in_cloud': {
        signInCancelled = false;
        const service = a.service as 'google' | 'microsoft';
        return new Promise((resolve, reject) =>
          setTimeout(() => {
            if (signInCancelled) return reject('Sign-in was cancelled.');
            const account = service === 'google' ? 'you@gmail.com' : 'you@outlook.com';
            syncTo(service === 'google' ? { kind: 'google_drive', account } : { kind: 'one_drive', account });
            resolve(account);
          }, 1500),
        );
      }

      case 'cancel_sign_in':
        signInCancelled = true;
        return null;

      case 'sync_now':
        syncTo(sync.target);
        return null;

      case 'sync_folder_suggestions':
        return [
          { label: 'OneDrive', path: 'C:\\Users\\you\\OneDrive' },
          { label: 'Google Drive', path: 'G:\\My Drive' },
        ];

      case 'detection_enabled':
        return activity.enabled;

      case 'plugin:dialog|open':
        return (a.options as { directory?: boolean } | undefined)?.directory
          ? 'C:\\Users\\you\\Dropbox'
          : ['C:\\Users\\you\\Documents\\School\\Revision plan.docx', 'C:\\Users\\you\\Pictures\\diagram.png'];

      case 'running_apps':
        return APPS;

      case 'activity':
        return activity;

      case 'dismiss_suggestion':
        activity = { ...activity, suggestion: null };
        activityChanged();
        return null;

      case 'undo_switch':
      case 'dismiss_switch':
        activity = { ...activity, switched: null };
        activityChanged();
        return null;

      case 'auto_start_after':
        return startAfter;

      case 'set_auto_start_after':
        startAfter = Number(a.secs);
        return null;

      case 'auto_switch':
        return autoSwitch;

      case 'set_auto_switch':
        autoSwitch = !!a.enabled;
        return null;

      case 'accept_link_offer':
      case 'refuse_link_offer':
        activity = { ...activity, link_offer: null };
        activityChanged();
        return null;

      case 'set_left_off': {
        const t = find(a.taskId as string);
        const text = String(a.text).trim();
        t.left_off = text ? { text, at: new Date().toISOString(), device: DEVICE.id } : null;
        changed();
        return null;
      }

      case 'add_check': {
        const t = find(a.taskId as string);
        t.checklist = [
          ...t.checklist,
          { id: crypto.randomUUID(), text: String(a.text), done: false, added_at: new Date().toISOString() },
        ];
        changed();
        return null;
      }

      case 'set_check': {
        const t = find(a.taskId as string);
        t.checklist = t.checklist.map((c) => (c.id === a.id ? { ...c, done: !!a.done } : c));
        changed();
        return null;
      }

      case 'remove_check': {
        const t = find(a.taskId as string);
        t.checklist = t.checklist.filter((c) => c.id !== a.id);
        changed();
        return null;
      }

      case 'task_openable': {
        const t = find(a.taskId as string);
        const pages = t.attachments.filter((x) => x.kind === 'link').length;
        const files = t.attachments.filter((x) => x.kind !== 'link' && x.kind !== 'app').length;
        return { files, pages, unavailable: [] };
      }

      case 'open_task_things':
        return { opened: find(a.taskId as string).attachments.length, skipped: [] };

      // Sample history: a few stretches on each of the past days.
      case 'work_sessions': {
        const from = new Date(a.from as string).getTime();
        const to = new Date(a.to as string).getTime();
        const out = [];
        for (let day = 0; day < 9; day++) {
          const base = new Date();
          base.setHours(9, 0, 0, 0);
          base.setDate(base.getDate() - day);
          const plan = [
            [tasks[0].id, 0, 50 + day * 7],
            [tasks[2].id, 90, 130 - day * 4],
            [tasks[1].id, 200, 215 + (day % 3) * 30],
          ];
          for (const [taskId, start, end] of plan) {
            const s = base.getTime() + (start as number) * 60_000;
            const e = base.getTime() + (end as number) * 60_000;
            if (e > from && s < to && e < Date.now())
              out.push({
                id: `${day}-${taskId}`,
                task_id: taskId,
                device: DEVICE.id,
                started_at: new Date(s).toISOString(),
                ended_at: new Date(e).toISOString(),
              });
          }
        }
        return out;
      }

      // A sample web project on the first task's linked folder.
      case 'project_commands': {
        const t = find(a.taskId as string);
        if (!t.attachments.some((x) => x.kind === 'folder')) return [];
        const folder = 'C:\\Users\\you\\Documents\\School\\Lecture slides';
        return ['dev', 'build', 'preview', 'lint', 'test'].map((label) => ({
          id: `${folder}|npm run ${label}`,
          folder,
          project: 'Lecture slides',
          label,
          command: `npm run ${label}`,
        }));
      }

      case 'project_runs':
        return [...projectRuns.values()];

      case 'run_project_command': {
        const id = a.commandId as string;
        projectRuns.set(id, {
          command_id: id,
          task_id: a.taskId as string,
          label: id.split(' ').pop() ?? id,
          project: 'Lecture slides',
          command: id.split('|')[1],
          running: true,
          url: id.endsWith('dev') ? 'http://localhost:5173' : null,
          output: [
            '> slides@1.0.0 dev',
            '> vite',
            '',
            '  VITE v8.3.8  ready in 412 ms',
            '',
            '  ➜  Local:   http://localhost:5173/',
          ],
          started_at: new Date().toISOString(),
          exit: null,
        });
        void emit('project-runs');
        return null;
      }

      case 'stop_project_command': {
        const r = projectRuns.get(a.commandId as string);
        if (r) projectRuns.set(r.command_id, { ...r, running: false, exit: -1 });
        void emit('project-runs');
        return null;
      }

      case 'open_project_url':
        return null;

      case 'project_autoruns':
        return projectAutoruns.get(a.taskId as string) ?? [];

      case 'set_project_autorun': {
        const list = (projectAutoruns.get(a.taskId as string) ?? []).filter((id) => id !== a.commandId);
        if (a.on) list.push(a.commandId as string);
        projectAutoruns.set(a.taskId as string, list);
        return null;
      }

      case 'hide_switcher':
        return null;

      case 'switcher_problem':
        return null;

      case 'switcher_shortcut':
        return switcherShortcut;

      case 'set_switcher_shortcut':
        switcherShortcut = (a.shortcut as string | null) ?? null;
        return null;

      case 'update_info':
        return { current: '0.1.0', ready: '0.2.0', enabled: true };

      case 'check_for_updates':
        return true;

      case 'install_update':
        return null;

      case 'start_with_windows':
        return startup;

      case 'set_start_with_windows':
        startup = !!a.enabled;
        return null;

      case 'widget_on_top':
        return onTop;

      case 'set_widget_on_top':
        onTop = a.onTop as typeof onTop;
        return null;

      // Sample checks: one file open in Word, one empty, the rest fine.
      case 'check_hand_in':
        return (a.paths as string[]).map((path) => {
          const name = path.split(/[\\/]/).pop() ?? path;
          return {
            path,
            name,
            size: /empty/i.test(name) ? 0 : 248_000,
            modified: new Date(Date.now() - 3 * 3600_000).toISOString(),
            open_in: /draft/i.test(name) && !wordClosed ? ['Microsoft Word'] : [],
            problem: /empty/i.test(name) ? "It's empty." : null,
            same_as_last: false,
          };
        });

      case 'check_link': {
        const url = String(a.url);
        const check = /docs\.google/.test(url)
          ? {
              url,
              title: null,
              verdict: 'sign_in',
              message: 'Others have to sign in to open it. Check its sharing settings let anyone with the link see it.',
            }
          : { url, title: 'Final report – Example', verdict: 'ok', message: 'It opens for anyone with the link.' };
        return new Promise((resolve) => setTimeout(() => resolve(check), 600));
      }

      case 'dismiss_paused':
        activity = { ...activity, paused: [] };
        activityChanged();
        return null;

      case 'pause_after_away':
        return pauseAfterAway;

      case 'set_pause_after_away':
        pauseAfterAway = (a.minutes as number | null) || null;
        return null;

      case 'set_detection':
        activity = { ...activity, enabled: !!a.enabled, active: [], suggestion: null };
        activityChanged();
        return null;

      case 'open_main':
        console.info('[preview] open_main');
        return null;

      case 'widget_visible':
        return widgetVisible;

      case 'set_widget_visible':
        widgetVisible = !!a.visible;
        setTimeout(() => void emit('widget-visibility', widgetVisible), 0);
        return widgetVisible;

      // ?relay=stuck shows the notice for a browser relay that got stuck, with Windhawk in it.
      case 'relay_problem':
        return new URLSearchParams(location.search).get('relay') === 'stuck'
          ? {
              since: new Date().toISOString(),
              suspects: [
                {
                  name: 'Windhawk',
                  files: ['windhawk.dll', 'explorer-details-better-file-sizes_1.5.1.dll', 'libc++.whl'],
                },
              ],
              restarts: 1,
              gave_up: false,
            }
          : null;

      case 'dismiss_relay_problem':
      case 'windhawk_present':
        return false;

      default:
        // Window and plugin calls (minimize, setSize, dialogs...) do nothing in the browser.
        if (cmd.startsWith('plugin:')) return null;
        throw `[preview] no mock for command '${cmd}'`;
    }
  },
  { shouldMockEvents: true },
);
