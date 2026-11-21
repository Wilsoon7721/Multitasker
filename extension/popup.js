const $ = (id) => document.getElementById(id);

/** Asks MultiTasker something, through the extension's background worker. */
const ask = (request) => chrome.runtime.sendMessage({ to: 'multitasker', request });

const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
const url = /^https?:\/\//i.test(tab?.url ?? '') ? tab.url : null;
let tasks = [];

$('page-title').textContent = tab?.title || 'This page';
$('page-url').textContent = url ? url.replace(/^https?:\/\/(www\.)?/, '') : "Browser pages can't be linked to tasks.";
$('new-title').value = url ? (tab.title ?? '').slice(0, 200) : '';

function notice(text, isError = false) {
  const el = $('notice');
  el.textContent = text;
  el.classList.toggle('error', isError);
  el.hidden = !text;
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

async function run(request) {
  const reply = await ask(request);
  if (reply.error) notice(reply.error, true);
  await load();
}

function render() {
  const query = $('filter').value.trim().toLowerCase();
  const shown = tasks.filter(
    (t) => !query || t.title.toLowerCase().includes(query) || (t.category ?? '').toLowerCase().includes(query),
  );
  const list = $('tasks');
  list.replaceChildren(
    ...shown.map((t) => {
      const li = document.createElement('li');
      li.className = 'task';

      const dot = document.createElement('span');
      dot.className = `dot${t.in_progress ? ' on' : ''}`;
      dot.title = t.in_progress ? 'In progress' : 'To do';

      const text = document.createElement('div');
      text.className = 'task-text';
      const title = document.createElement('div');
      title.className = 'task-title';
      title.textContent = t.title;
      text.append(title);
      if (t.category || t.in_progress) {
        const meta = document.createElement('div');
        meta.className = 'task-meta';
        meta.textContent = [t.in_progress ? 'In progress' : null, t.category].filter(Boolean).join(' · ');
        text.append(meta);
      }

      li.append(dot, text);
      if (!t.in_progress) li.append(button('Start', '', () => run({ type: 'start', task_id: t.id })));
      li.append(
        t.linked
          ? button('Linked', 'linked', () => run({ type: 'unlink', attachment_id: t.linked }))
          : button('Link', 'primary', () => run({ type: 'link', task_id: t.id, url, title: tab.title }), !url),
      );
      if (t.linked) li.lastChild.title = 'Linked to this page; click to unlink';
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
  await run({ type: 'create', title, url, page_title: tab?.title });
  $('new-title').value = '';
});

$('show-app').addEventListener('click', async () => {
  const reply = await ask({ type: 'show_app' });
  if (reply.error) notice(reply.error, true);
  else window.close();
});

await load();
