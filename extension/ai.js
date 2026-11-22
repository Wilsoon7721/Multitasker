// Tells MultiTasker when an AI chat starts and finishes writing a reply, so it can say when one is ready.
(() => {
  const SITES = {
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
  const site = SITES[location.hostname] ?? location.hostname;

  // Every one of these chats shows a Stop button for as long as a reply is being written.
  const STOP = [
    'button[data-testid="stop-button"]',
    'button[aria-label^="stop" i]',
    'button[title^="stop" i]',
    'button[data-testid*="stop" i]',
  ].join(',');

  const writing = () => [...document.querySelectorAll(STOP)].some((el) => el.offsetParent !== null);

  let busy = false;
  // The button can blink out between parts of a reply, so it has to stay gone for two looks in a row.
  let gone = 0;

  const tell = (ai) => {
    try {
      void chrome.runtime.sendMessage({ ai, site }).catch(() => {});
    } catch {
      // The extension was updated or removed; this page's copy of the script no longer has anyone to tell.
    }
  };

  setInterval(() => {
    if (writing()) {
      gone = 0;
      if (!busy) {
        busy = true;
        tell('started');
      }
    } else if (busy && ++gone >= 2) {
      busy = false;
      tell('done');
    }
  }, 800);
})();
