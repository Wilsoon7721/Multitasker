# Browser extension

The extension in [`extension/`](../extension) links browser tabs, such as AI chats and docs, to MultiTasker tasks. It
works in Chrome, Edge and Brave.

- **Link a tab** to any open task from the toolbar popup, or make a new task from the page. The link is a normal
  attachment, so it opens from MultiTasker on any of your computers.
- **Start** a task from the popup.
- Only web pages (http and https) can be linked. The browser's own pages, such as `chrome://extensions`, only open
  inside the browser, so MultiTasker couldn't open them from a task.
- **See what's linked**: the toolbar icon shows ✓ on a page linked to an open task.
- **Activity detection**: while the browser is in front, the tab you're on counts like a linked file or app does. A
  linked page marks its task active, and a page under a linked address (say, another page of the same docs) suggests
  the task.
- **Picking a task back up**: when a task starts, the widget offers to open its files and pages. With the extension,
  the browser switches to pages that are already open (matching exactly, or else ignoring what follows `?`), moves them
  next to each other and opens the rest beside them.
- **Linking as you go**: stay on a page that isn't linked to any task for two minutes while a task is in progress on
  this computer, and the widget offers to link it to that task. Saying no stops it offering that page again until
  MultiTasker restarts.

## Reply notices

Turn on **Tell me when AI replies finish** in the popup, and when Claude, ChatGPT, Gemini, AI Studio, Perplexity,
Copilot, DeepSeek or Grok finishes a reply while you're on another tab or app, MultiTasker says so. The widget offers
to go back, switching to that tab and starting again the task that was in progress when you asked. Replies that take
under five seconds, or that you watched arrive, aren't announced.

It's off until you turn it on, and the browser asks for its permissions then. Turning it off hands them back.

How it tells a reply is being written, from three signals so a change to one site doesn't break it:

- **Text streaming in.** The conversation grows in bursts while a reply arrives and stops when it's done. This works
  on any site and in any language; typing in the prompt box doesn't count.
- **The Stop button.** Most chats show one while they write. It helps through pauses where nothing new appears.
- **The request carrying the reply.** The extension sees when requests to these sites start and finish (never what's
  in them), so a reply isn't called done while the model is still thinking. Requests alone never start a reply, so a
  site's background connections can't pass for one.

If a site you've used lately hasn't had a reply noticed for a few days, the popup says so: the site has probably
changed in a way the extension can't follow yet.

### Permissions, as explained to the Chrome Web Store

| Permission | Why |
| --- | --- |
| `nativeMessaging` | To talk to the MultiTasker app on the same computer. Nothing goes over the network. |
| `tabs` | To read the address and title of the tab in front, so it can be linked to a task, and to switch to a task's tabs when you pick the task up. |
| `storage` | To remember, on this computer, when reply notices last noticed a reply on each AI site. |
| `scripting` (optional) | Only after you turn on reply notices: to run the reply detector on the AI chat sites listed below. |
| `webRequest` (optional) | Only after you turn on reply notices: to see when requests to those AI chat sites start and finish, so the extension can tell when a reply is complete. It never reads, blocks or changes requests or their contents. |
| AI chat sites (optional) | `claude.ai`, `chatgpt.com`, `chat.openai.com`, `gemini.google.com`, `aistudio.google.com`, `perplexity.ai`, `copilot.microsoft.com`, `chat.deepseek.com` and `grok.com`, only after you turn on reply notices, and only to tell when a reply finishes. Nothing from these pages leaves your computer; MultiTasker learns only that a reply started and finished, and on which tab. |

## Setting it up

1. Open MultiTasker once. On start, it tells Chrome, Edge and Brave where to find it (see below).
2. In the browser, open `chrome://extensions` (or `edge://extensions`), turn on **Developer mode**, choose **Load
   unpacked** and pick the `extension` folder.
3. Pin MultiTasker to the toolbar.

If the popup says MultiTasker isn't set up in this browser, open MultiTasker once and try again.

## How it talks to MultiTasker

The extension uses the browser's native messaging, so no web page can reach MultiTasker and nothing listens on the
network.

- On start, MultiTasker writes `browser-host.json` to its data folder (`%APPDATA%\dev.wilsoon.multitasker`). It also
  registers that file under `HKEY_CURRENT_USER\Software\<browser>\NativeMessagingHosts\dev.wilsoon.multitasker` for
  Chrome, Edge and Brave. The file allows only this extension, `fldigbhidkmfhlgplcmgjmbhbgkdmebj`.
- The browser then runs a copy of MultiTasker kept in `browser-host\` in that folder, as a relay. A copy is used so
  the running relay never locks the app's own file while you rebuild it. The relay passes messages to the running
  app over a named pipe that only this computer can connect to.
- Choosing something in the popup starts MultiTasker if it isn't running. Switching tabs never does.

The extension's id comes from the public key in `extension/manifest.json`. Its private key, needed only to publish
the extension to a store, is kept out of the repository in `.secrets/extension-key.pem`.
