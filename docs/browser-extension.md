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
