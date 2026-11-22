import { setUp, turnOff, turnOn } from './replies.js';

// Talks to MultiTasker through the relay it registers with the browser, and tells it which tab is in front.
const HOST = 'dev.wilsoon.multitasker';

let port = null;
let nextId = 1;
const waiting = new Map();

/** Turns the browser's connection errors into something a person can act on. */
function friendly(message) {
  if (/not found/i.test(message))
    return "MultiTasker isn't set up in this browser yet. Open MultiTasker once, then try again.";
  if (/forbidden/i.test(message)) return "This copy of the extension isn't the one MultiTasker trusts.";
  return message || 'MultiTasker closed the connection.';
}

function connect() {
  port = chrome.runtime.connectNative(HOST);
  port.onMessage.addListener((reply) => {
    if (reply?.push === 'open_tabs') return void openTabs(reply.urls ?? []);
    if (reply?.push === 'focus_tab') return void focusTab(reply.tab_id);
    const resolve = waiting.get(reply?.id);
    if (resolve) {
      waiting.delete(reply.id);
      resolve(reply);
    }
  });
  port.onDisconnect.addListener(() => {
    const error = friendly(chrome.runtime.lastError?.message ?? '');
    port = null;
    for (const resolve of waiting.values()) resolve({ error, offline: true });
    waiting.clear();
  });
}

/** Sends one request to MultiTasker and waits for its reply. */
function ask(request) {
  if (!port) connect();
  const id = nextId++;
  return new Promise((resolve) => {
    waiting.set(id, resolve);
    try {
      port.postMessage({ id, ...request });
    } catch (e) {
      waiting.delete(id);
      resolve({ error: friendly(String(e)), offline: true });
    }
  });
}

const linkable = (url) => /^https?:\/\//i.test(url ?? '');

/** An address as compared: no fragment, no "www.", no trailing slash; `withQuery` keeps what follows "?". */
function comparable(url, withQuery) {
  try {
    const u = new URL(url);
    const path = u.pathname.replace(/\/+$/, '');
    return `${u.hostname.replace(/^www\./, '')}${path}${withQuery ? u.search : ''}`.toLowerCase();
  } catch {
    return url;
  }
}

/**
 * Brings a task's pages up together: switches to the first one already open, moves any others that are open next to
 * it, and opens the rest right after them. A page matches exactly where it can, and otherwise ignoring what follows "?".
 */
async function openTabs(urls) {
  const tabs = (await chrome.tabs.query({})).filter((t) => linkable(t.url));
  const used = new Set();
  const find = (url, withQuery) =>
    tabs.find((t) => !used.has(t.id) && comparable(t.url, withQuery) === comparable(url, withQuery));
  const found = [];
  const missing = [];
  for (const url of urls) {
    const tab = find(url, true) ?? find(url, false);
    if (tab) {
      used.add(tab.id);
      found.push(tab);
    } else {
      missing.push(url);
    }
  }

  let anchor = found[0];
  if (!anchor) {
    const win = await chrome.windows.getLastFocused({ windowTypes: ['normal'] }).catch(() => null);
    anchor = await chrome.tabs.create({ url: missing.shift(), active: true, ...(win ? { windowId: win.id } : {}) });
  }
  await chrome.tabs.update(anchor.id, { active: true });
  await chrome.windows.update(anchor.windowId, { focused: true });

  let index = (await chrome.tabs.get(anchor.id)).index;
  for (const tab of found.slice(1)) {
    index += 1;
    await chrome.tabs.move(tab.id, { windowId: anchor.windowId, index }).catch(() => {});
  }
  for (const url of missing) {
    index += 1;
    await chrome.tabs.create({ url, windowId: anchor.windowId, index, active: false });
  }
}

/** Brings a tab and its window to the front. */
async function focusTab(tabId) {
  const tab = await chrome.tabs.update(tabId, { active: true }).catch(() => null);
  if (tab) await chrome.windows.update(tab.windowId, { focused: true }).catch(() => {});
}

/** Whether a tab is the one in front of a browser window that has focus. */
async function looking(tabId) {
  const win = await chrome.windows.getLastFocused().catch(() => null);
  if (!win?.focused) return false;
  const [tab] = await chrome.tabs.query({ active: true, windowId: win.id });
  return tab?.id === tabId;
}

function showLinked(tabId, linked) {
  const names = linked.map((l) => l.title).join(', ');
  void chrome.action.setBadgeText({ tabId, text: linked.length ? '✓' : '' });
  void chrome.action.setBadgeBackgroundColor({ tabId, color: '#3d5afe' });
  void chrome.action.setTitle({ tabId, title: linked.length ? `MultiTasker: linked to ${names}` : 'MultiTasker' });
}

let lastReported;

/** Tells MultiTasker which page is in front, or that the browser isn't. */
async function report() {
  const win = await chrome.windows.getLastFocused().catch(() => null);
  const [tab] = win?.focused ? await chrome.tabs.query({ active: true, windowId: win.id }) : [];
  const url = tab && linkable(tab.url) ? tab.url : null;
  const title = url ? (tab.title ?? '').replace(/^\(\d+\+?\)\s*/, '') : null;
  // The title often arrives after the address, so a new title is worth reporting too.
  const key = JSON.stringify([url, title]);
  if (key === lastReported) return;
  lastReported = key;
  const reply = await ask({ type: 'active_tab', url, title });
  // Not running yet: the same page is reported again on the next change.
  if (reply.error) lastReported = undefined;
  if (tab) showLinked(tab.id, reply.linked ?? []);
}

chrome.tabs.onActivated.addListener(() => void report());
chrome.windows.onFocusChanged.addListener(() => void report());
chrome.tabs.onUpdated.addListener((_id, change, tab) => {
  if (tab.active && (change.url || change.status === 'complete')) void report();
});

setUp(ask, looking);

// The popup turns reply notices on (after the browser grants their permissions) and off.
chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  if (message?.to !== 'replies') return false;
  const done = message.on ? turnOn(ask, looking) : turnOff();
  void done.then(
    () => respond({ ok: true }),
    (e) => respond({ error: String(e) }),
  );
  return true;
});

// The popup's requests go through here, so there's only one connection to MultiTasker.
chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  if (message?.to !== 'multitasker') return false;
  void ask(message.request).then((reply) => {
    // Linking changes what the badge should say.
    lastReported = undefined;
    void report();
    respond(reply);
  });
  return true;
});
