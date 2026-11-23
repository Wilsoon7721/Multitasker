// Tells the extension whether an AI chat looks busy writing a reply, from what the page itself shows.
(() => {
  // Turning reply notices on injects this into chats already open, which may also have it from loading.
  if (window.__multitaskerReplies) return;
  window.__multitaskerReplies = true;

  // Most chats show a Stop button for as long as a reply is being written.
  const STOP = [
    'button[data-testid="stop-button"]',
    'button[aria-label^="stop" i]',
    'button[title^="stop" i]',
    'button[data-testid*="stop" i]',
  ].join(',');
  const stopShown = () => [...document.querySelectorAll(STOP)].some((el) => el.offsetParent !== null);

  // Any chat, in any language, grows its conversation in small bursts while a reply streams in.
  const TICK = 500;
  const STREAMING_AFTER = 4;
  const STILL_AFTER = 6;
  let grew = false;
  const typedIn = (node) => {
    const el = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
    return !!el?.closest('[contenteditable=""], [contenteditable="true"], textarea, input');
  };
  new MutationObserver((records) => {
    if (grew) return;
    for (const r of records) {
      if (typedIn(r.target)) continue;
      const added =
        r.type === 'characterData' || [...r.addedNodes].some((n) => !typedIn(n) && (n.textContent ?? '').trim());
      if (added) {
        grew = true;
        return;
      }
    }
  }).observe(document.body, { subtree: true, childList: true, characterData: true });

  // A reply only starts after you've typed or clicked in the chat: loading a conversation, restoring tabs when the
  // browser starts and switching chats all fill the page the same way, but without you.
  const ASKED_WITHIN = 60_000;
  let lastTouched = 0;
  const touched = (e) => {
    if (e.isTrusted) lastTouched = Date.now();
  };
  document.addEventListener('keydown', touched, true);
  document.addEventListener('pointerdown', touched, true);

  let growing = 0;
  let still = 0;
  let streaming = false;
  let busy = false;
  let stopGone = 2;

  const tell = (message) => {
    try {
      void chrome.runtime.sendMessage({ replies: true, ...message }).catch(() => {});
    } catch {
      // The extension was updated or removed; this copy of the script has no one left to tell.
    }
  };
  tell({ seen: true, hidden: document.hidden });
  // Out of sight, the page stops drawing the reply, so the extension leans on the reply's request instead.
  document.addEventListener('visibilitychange', () => tell({ hidden: document.hidden }));

  setInterval(() => {
    if (grew) {
      growing += 1;
      still = 0;
    } else {
      still += 1;
      growing = 0;
    }
    grew = false;
    if (!streaming && growing >= STREAMING_AFTER) streaming = true;
    if (streaming && still >= STILL_AFTER) streaming = false;

    // The button can blink out between parts of a reply, so it has to stay gone for two looks in a row.
    stopGone = stopShown() ? 0 : stopGone + 1;
    const looksBusy = stopGone < 2 || streaming;
    // Once a reply has started it's followed to the end, however long it takes.
    const now = looksBusy && (busy || Date.now() - lastTouched < ASKED_WITHIN);
    if (now !== busy) {
      busy = now;
      tell({ busy });
    }
  }, TICK);
})();
