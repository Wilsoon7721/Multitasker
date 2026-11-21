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
