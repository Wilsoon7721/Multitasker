import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import type { Attachment, AttachmentKind, NewAttachment, NewTask, TaskChanges, TaskView } from '../api';

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
    due_date: day(1),
    in_progress: true,
    started_at: iso(95 * 60_000),
  }),
  task('Fix calendar sync bug in JavaDashboard', { category: 'Dev', in_progress: true, started_at: iso(20 * 60_000) }),
  task('Draft project proposal', { category: 'School', due_date: day(-1) }),
  task('Book dentist appointment', { due_date: day(4) }),
  task('Read chapter 5', { category: 'School', description: 'Pages 120–158.' }),
  task('Renew domain', { status: 'done', completed_at: iso(86_400_000) }),
];

function task(title: string, extra: Partial<TaskView> = {}): TaskView {
  return {
    id: crypto.randomUUID(),
    title,
    description: null,
    category: null,
    due_date: null,
    status: 'todo',
    created_at: iso(3 * 86_400_000),
    updated_at: iso(3 * 86_400_000),
    completed_at: null,
    in_progress: false,
    started_at: null,
    attachments: [],
    ...extra,
  };
}

let widgetVisible = true;
const DEVICE = { id: 'preview-device', name: 'PREVIEW-PC' };

const KINDS: [RegExp, AttachmentKind][] = [
  [/\.pdf$/i, 'pdf'],
  [/\.(docx?|odt|rtf|txt|md|xlsx?|csv|pptx?)$/i, 'document'],
  [/\.(png|jpe?g|gif|webp|bmp|svg|heic)$/i, 'image'],
  [/\.(mp4|mov|mkv|avi|webm|wmv)$/i, 'video'],
];

function attachment(taskId: string, item: NewAttachment): Attachment {
  const base = { id: crypto.randomUUID(), task_id: taskId, added_at: new Date().toISOString() };
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

tasks[0].attachments = [
  attachment(tasks[0].id, { type: 'path', path: 'C:\\Users\\you\\Documents\\School\\GS module notes.pdf' }),
  attachment(tasks[0].id, { type: 'path', path: 'C:\\Users\\you\\Documents\\School\\Lecture slides' }),
  attachment(tasks[0].id, { type: 'link', url: 'https://claude.ai/chat/0f3c', name: 'Claude: GS module summary' }),
];

const blank = (v: string | null | undefined) => (v && v.trim() ? v.trim() : null);

function find(id: string): TaskView {
  const t = tasks.find((t) => t.id === id);
  if (!t) throw `task ${id} was not found`;
  return t;
}

function changed() {
  setTimeout(() => void emit('tasks-changed'), 0);
}

const label = new URLSearchParams(location.search).get('window') ?? (location.pathname.includes('widget') ? 'widget' : 'main');
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
        if (c.status !== undefined && c.status !== t.status) {
          t.status = c.status;
          t.completed_at = c.status === 'done' ? new Date().toISOString() : null;
          if (c.status === 'done') Object.assign(t, { in_progress: false, started_at: null });
        }
        changed();
        return t;
      }

      case 'delete_task':
        find(a.id as string);
        tasks = tasks.filter((t) => t.id !== a.id);
        changed();
        return null;

      case 'set_in_progress': {
        const t = find(a.id as string);
        if (a.inProgress) {
          Object.assign(t, { status: 'todo', completed_at: null, in_progress: true, started_at: t.started_at ?? new Date().toISOString() });
        } else {
          Object.assign(t, { in_progress: false, started_at: null });
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

      case 'this_device':
        return DEVICE;

      case 'plugin:dialog|open':
        return ['C:\\Users\\you\\Documents\\School\\Revision plan.docx', 'C:\\Users\\you\\Pictures\\diagram.png'];

      case 'open_main':
        console.info('[preview] open_main');
        return null;

      case 'widget_visible':
        return widgetVisible;

      case 'set_widget_visible':
        widgetVisible = !!a.visible;
        setTimeout(() => void emit('widget-visibility', widgetVisible), 0);
        return widgetVisible;

      default:
        // Window and plugin calls (minimize, setSize, dialogs...) do nothing in the browser.
        if (cmd.startsWith('plugin:')) return null;
        throw `[preview] no mock for command '${cmd}'`;
    }
  },
  { shouldMockEvents: true },
);
