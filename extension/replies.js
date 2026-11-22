// Reply notices: tells MultiTasker when an AI chat finishes a reply, so it can say so while you're elsewhere. It's off
// until turned on in the popup, which asks the browser for the permissions it needs then.

/** The chats it watches, and what to call each. */
export const SITES = {
  'claude.ai': 'Claude',
  'chatgpt.com': 'ChatGPT',
  'chat.openai.com': 'ChatGPT',
  'gemini.google.com': 'Gemini',
  'aistudio.google.com': 'AI Studio',
  'www.perplexity.ai': 'Perplexity',
  'perplexity.ai': 'Perplexity',
  'copilot.microsoft.com': 'Copilot',
  'chat.deepseek.com': 'DeepSeek',
  'grok.com': 'Grok',
};
export const ORIGINS = Object.keys(SITES).map((host) => `https://${host}/*`);
export const PERMISSIONS = { permissions: ['webRequest', 'scripting'], origins: ORIGINS };

const SCRIPT_ID = 'replies';
/** After the page and its requests both go quiet, wait this long before calling the reply done. */
const SETTLE = 1500;
/** The request carrying a reply can start a moment before the page shows it being written. */
const LEAD = 3000;
/** Requests open longer than this are a site's background connection, not a reply. */
const STALE = 3 * 60_000;
/** A site in use with no reply seen for this long may have changed so replies are no longer noticed. */
export const SILENT_AFTER = 3 * 86_400_000;

export const isOn = () => chrome.permissions.contains(PERMISSIONS);

const hostOf = (url) => {
  try {
    return new URL(url).hostname;
  } catch {
    return '';
  }
};

/** What's known about each tab's chat: whether its page looks busy, its open requests, and whether a reply started. */
const tabs = new Map();
const tabState = (tabId) => {
  if (!tabs.has(tabId)) tabs.set(tabId, { page: false, requests: new Map(), started: 0, settle: null });
  return tabs.get(tabId);
};

/** When each site was last used and last seen replying, for telling when noticing replies stops working there. */
async function remember(host, field) {
  const { health = {} } = await chrome.storage.local.get('health');
  const site = (health[host] ??= {});
  site[field] = Date.now();
  site.first ??= Date.now();
  await chrome.storage.local.set({ health });
}

export async function health() {
  const { health = {} } = await chrome.storage.local.get('health');
  return health;
}

/**
 * A reply starts when the page shows one being written, never from requests alone, so a site's own background traffic
 * can't pass for a reply. It ends once the page is quiet and every request made since it started has finished, so a
 * pause while the model thinks doesn't end it early.
 */
function update(tabId, url, ask, looking) {
  const s = tabState(tabId);
  const now = Date.now();
  const waiting = s.started && [...s.requests.values()].some((at) => at >= s.started - LEAD && now - at < STALE);
  const busy = s.page || waiting;

  if (busy && !s.started) {
    s.started = now;
    void ask({ type: 'ai_started', tab_id: tabId, site: SITES[hostOf(url)] ?? hostOf(url) });
  }
  if (busy) {
    clearTimeout(s.settle);
    s.settle = null;
    return;
  }
  if (s.started && !s.settle) {
    s.settle = setTimeout(async () => {
      s.settle = null;
      const later = Date.now();
      const still = s.page || [...s.requests.values()].some((at) => at >= s.started - LEAD && later - at < STALE);
      if (still) return;
      s.started = 0;
      void remember(hostOf(url), 'replied');
      void ask({ type: 'ai_done', tab_id: tabId, looking: await looking(tabId) });
    }, SETTLE);
  }
}

let listening = false;

/** Follows requests to the chats, only when they start and end; nothing about what's in them. */
function listen(ask, looking) {
  if (listening || !chrome.webRequest) return;
  listening = true;
  const filter = { urls: ORIGINS, types: ['xmlhttprequest', 'other'] };
  chrome.webRequest.onBeforeRequest.addListener((d) => {
    if (d.tabId < 0) return;
    tabState(d.tabId).requests.set(d.requestId, Date.now());
  }, filter);
  const ended = (d) => {
    const s = tabs.get(d.tabId);
    if (!s?.requests.delete(d.requestId)) return;
    if (s.started)
      void chrome.tabs.get(d.tabId).then(
        (t) => update(d.tabId, t.url, ask, looking),
        () => {},
      );
  };
  chrome.webRequest.onCompleted.addListener(ended, filter);
  chrome.webRequest.onErrorOccurred.addListener(ended, filter);
}

/** Wires up reply notices; called at the worker's start so the browser can wake it for them. */
export function setUp(ask, looking) {
  listen(ask, looking);
  chrome.tabs.onRemoved.addListener((tabId) => {
    clearTimeout(tabs.get(tabId)?.settle);
    tabs.delete(tabId);
  });
  // Chrome closes the popup when it shows the permission prompt, so turning on happens here once they're granted,
  // and again whenever the worker starts in case it was missed.
  chrome.permissions.onAdded.addListener(() => void isOn().then((on) => on && turnOn(ask, looking)));
  void isOn().then((on) => on && turnOn(ask, looking));

  chrome.runtime.onMessage.addListener((message, sender) => {
    if (!message?.replies) return false;
    const tabId = sender.tab?.id;
    const url = sender.tab?.url ?? '';
    if (tabId === undefined) return false;
    // Chats open from before notices were turned off still have the script until they reload.
    void isOn().then((on) => {
      if (!on) return;
      if (message.seen) void remember(hostOf(url), 'visited');
      if (typeof message.busy === 'boolean') {
        tabState(tabId).page = message.busy;
        update(tabId, url, ask, looking);
      }
    });
    return false;
  });
}

/** Turns reply notices on once the browser has given the permissions: watch chats from now on, and those open now. */
export async function turnOn(ask, looking) {
  const registered = await chrome.scripting.getRegisteredContentScripts({ ids: [SCRIPT_ID] });
  listen(ask, looking);
  if (registered.length) return;
  // The popup and the permission event can both get here at once; the second finds it registered.
  const added = await chrome.scripting
    .registerContentScripts([
      { id: SCRIPT_ID, matches: ORIGINS, js: ['ai.js'], runAt: 'document_idle', persistAcrossSessions: true },
    ])
    .then(
      () => true,
      () => false,
    );
  if (!added) return;
  // Chats already open get it now; the ones opened later get it as they load.
  for (const tab of await chrome.tabs.query({ url: ORIGINS })) {
    await chrome.scripting.executeScript({ target: { tabId: tab.id }, files: ['ai.js'] }).catch(() => {});
  }
}

/** Turns them off and hands the permissions back. */
export async function turnOff() {
  await chrome.scripting.unregisterContentScripts({ ids: [SCRIPT_ID] }).catch(() => {});
  await chrome.permissions.remove(PERMISSIONS);
  for (const s of tabs.values()) clearTimeout(s.settle);
  tabs.clear();
}
