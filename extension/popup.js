const $ = (id) => document.getElementById(id);

/** Asks MultiTasker something, through the extension's background worker. */
const ask = (request) => chrome.runtime.sendMessage({ to: 'multitasker', request });

const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
const url = /^https?:\/\//i.test(tab?.url ?? '') ? tab.url : null;
// Sites put unread counts in front of their titles, like "(23) Video - YouTube".
const pageTitle = (tab?.title ?? '').replace(/^\(\d+\+?\)\s*/, '');
let tasks = [];
let confirmTimer;
let freshId = null;

$('page-title').textContent = pageTitle || 'This page';
$('page-url').textContent = url ? url.replace(/^https?:\/\/(www\.)?/, '') : (tab?.url ?? '');
$('new-title').value = url ? pageTitle.slice(0, 200) : '';
if (!url) {
  $('unlinkable').hidden = false;
  $('new-title').placeholder = 'New task (without this page)';
}

function notice(text, isError = false) {
  const el = $('notice');
  el.textContent = text;
  el.classList.toggle('error', isError);
  el.hidden = !text;
}

/** Says what just happened, for a few seconds. */
function confirm(text) {
  $('confirm-text').textContent = text;
  $('confirm').hidden = false;
  clearTimeout(confirmTimer);
  confirmTimer = setTimeout(() => ($('confirm').hidden = true), 6000);
}

async function load() {
  // MultiTasker is started if it isn't running, which takes a few seconds.
  if (!tasks.length) notice('Connecting to MultiTasker…');
  const reply = await ask({ type: 'tasks', url });
  if (reply.error) {
    $('main').hidden = true;
    return notice(reply.error, !reply.offline);
  }
  notice('');
  tasks = reply.tasks;
  $('main').hidden = false;
  render();
}

function button(label, className, onClick, disabled = false) {
  const b = document.createElement('button');
  b.type = 'button';
  b.textContent = label;
  if (className) b.className = className;
  b.disabled = disabled;
  b.addEventListener('click', onClick);
  return b;
}

/** Shows a change straight away, then checks it with MultiTasker. */
async function run(request, change) {
  const before = tasks.map((t) => ({ ...t }));
  change();
  render();
  const reply = await ask(request);
  if (reply.error) {
    tasks = before;
    render();
    notice(reply.error, true);
    return null;
  }
  void load();
  return reply;
}

const update = (id, fields) => () => (tasks = tasks.map((t) => (t.id === id ? { ...t, ...fields } : t)));

function render() {
  const query = $('filter').value.trim().toLowerCase();
  const shown = tasks.filter(
    (t) => !query || t.title.toLowerCase().includes(query) || (t.category ?? '').toLowerCase().includes(query),
  );
  const list = $('tasks');
  list.replaceChildren(
    ...shown.map((t) => {
      const li = document.createElement('li');
      const fresh = t.id === freshId;
      li.className = `task${fresh ? ' fresh' : ''}`;

      const dot = document.createElement('span');
      dot.className = `dot${t.in_progress ? ' on' : ''}`;
      dot.title = t.in_progress ? 'In progress' : 'To do';

      const text = document.createElement('div');
      text.className = 'task-text';
      const title = document.createElement('div');
      title.className = 'task-title';
      title.textContent = t.title;
      text.append(title);
      const meta = [t.pending ? 'Adding…' : fresh ? 'Just added' : null, t.in_progress ? 'In progress' : null, t.category];
      if (meta.some(Boolean)) {
        const el = document.createElement('div');
        el.className = 'task-meta';
        el.textContent = meta.filter(Boolean).join(' · ');
        text.append(el);
      }

      li.append(dot, text);
      if (t.pending) return li;
      if (!t.in_progress) li.append(button('Start', '', () => run({ type: 'start', task_id: t.id }, update(t.id, { in_progress: true }))));
      if (t.linked) {
        const b = button(
          'Linked',
          'linked',
          () => run({ type: 'unlink', attachment_id: t.linked }, update(t.id, { linked: null })),
          t.linked === 'pending',
        );
        b.title = 'Linked to this page; click to unlink';
        li.append(b);
      } else if (url) {
        li.append(
          button('Link', 'primary', () =>
            run({ type: 'link', task_id: t.id, url, title: pageTitle }, update(t.id, { linked: 'pending' })),
          ),
        );
      }
      return li;
    }),
  );
  $('empty').hidden = shown.length > 0;
}

$('filter').addEventListener('input', render);

$('create').addEventListener('submit', async (e) => {
  e.preventDefault();
  const title = $('new-title').value.trim();
  if (!title) return;
  const add = $('add');
  add.disabled = true;
  add.textContent = 'Adding…';
  $('filter').value = '';
  const placeholder = { id: `pending-${Date.now()}`, title, category: null, in_progress: false, linked: null, pending: true };
  const reply = await run({ type: 'create', title, url, page_title: pageTitle }, () => (tasks = [placeholder, ...tasks]));
  add.disabled = false;
  add.textContent = 'Add';
  if (!reply) return;
  $('new-title').value = '';
  freshId = reply.task.id;
  tasks = tasks.map((t) => (t.id === placeholder.id ? reply.task : t));
  render();
  $('tasks').scrollTop = 0;
  confirm(url ? `Added “${title}” with this page linked.` : `Added “${title}”.`);
});

$('confirm-open').addEventListener('click', () => $('show-app').click());

$('show-app').addEventListener('click', async () => {
  const reply = await ask({ type: 'show_app' });
  if (reply.error) notice(reply.error, true);
  else window.close();
});

await load();
