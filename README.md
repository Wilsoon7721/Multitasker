# MultiTasker

An always-on-top focus list for people who juggle many tasks at once.

You start something, switch to something else, and forget the first thing exists until tomorrow. MultiTasker keeps
every task you've started pinned in a small widget at the top-right of your screen, so nothing in progress slips out
of sight.

## Status

Early development. What works today:

- A task list (add, edit, complete, delete) stored locally in SQLite, or taken from **Todoist** or a **Notion**
  database with your own token (see [docs/task-lists.md](docs/task-lists.md)). Builds made with a WilsoonID
  registration can also use a **JavaDashboard** task board
  (see [docs/cloud-sign-in.md](docs/cloud-sign-in.md#wilsoonid-and-javadashboard)).
- **Due dates with an optional time.** Timed tasks count down ("in 2h 15m") once they're within a day, and
  MultiTasker reminds you with a Windows notification beforehand (15 minutes by default) and when they're due.
- Mark tasks **in progress**; they appear in an always-on-top widget pinned to the top-right of your screen, with how
  long they've been going. It can stop staying on top while something is full screen or while chosen programs (say,
  PowerPoint) are in front, or stop staying on top altogether (Settings → Widget).
- Attach files, folders and links to tasks (by picker, or by dragging files from the desktop onto a task in the main
  window or the widget). Files stay where they are; MultiTasker keeps a reference and opens them in their default app.
- **Hand in** finished work: MultiTasker keeps a read-only copy of each file as it was, so you can open or save that
  version later even after the original changes. Handing the same file in again keeps every version. Finishing a task
  that has files attached offers to hand them in. Copies are kept in MultiTasker's data folder, and Sync carries them
  (up to 100 MB each) to your other computers. With Todoist or Notion, handed-in work is uploaded to the task itself.
  Links can be handed in too, such as a shared doc. Before handing in, MultiTasker checks each file isn't still open
  in another program (so unsaved changes aren't missed), empty, or not what its name says, and that each link opens
  for someone who isn't signed in. A file open elsewhere is handed in as soon as that program closes.
- Expand the widget to see your to-do list and drag a task up into In Progress to start it.
- A **browser extension** for Chrome, Edge and Brave links tabs, such as AI chats and docs, to tasks. While the
  browser is in front, the linked tab counts as working on its task (see
  [docs/browser-extension.md](docs/browser-extension.md)).
- **Activity detection** (Windows): link apps, files and folders to a task, and MultiTasker notices when you're on
  them — from the window in front and from file saves. Tasks in progress show as *active now*. Stay on another task's
  linked files or pages for half a minute and MultiTasker switches to it, pausing what you were on (Undo is one click
  away, or have it ask first in Settings). Time spent on a task adds up across pauses and computers. Everything stays
  on your computer; window titles are only read in memory. Turn it off from the tray ("Detect what I'm working on").
- **Pauses when you're away**: after 15 minutes with no keyboard or mouse, or while the computer sleeps or MultiTasker
  is closed, what's in progress on that computer pauses. The widget offers to resume when you're back. Change the wait
  or turn it off in Settings.
- **Getting back into a task**: when a task pauses or you switch away, the widget asks where you left off, and the
  note shows on the task until you replace it. Going back to a task shows that note and opens its linked pages, files
  and folders in one click ("Open all" in its details does the same).
- **Linking as you go**: stay on a page that isn't linked to anything for two minutes while a task is in progress, and
  the widget offers to link it to that task (needs the browser extension).
- **Installs like any app**: an installer that needs no admin rights, starts with Windows (quietly, to the tray and
  widget) and updates itself from GitHub releases. See [docs/releasing.md](docs/releasing.md) for making a release.
- **Sync between computers**, three ways:
  - sign in to **Google Drive** or **OneDrive**; MultiTasker only gets its own app folder (setting up the app
    registrations: [docs/cloud-sign-in.md](docs/cloud-sign-in.md));
  - a **shared folder** that a sync app (Dropbox and the like) carries while it's running;
  - an **S3-compatible bucket**: AWS S3, Cloudflare R2, Supabase Storage, Backblaze B2, Wasabi, DigitalOcean Spaces
    or MinIO.

  Sign-ins and secret keys stay in Windows Credential Manager. Tasks, what's in progress and attachments merge across
  computers, and each computer only writes its own files, so nothing gets overwritten. A task started on one
  computer is in progress on all of them.
- Tray icon: closing the main window keeps MultiTasker running with the widget.

Planned:

- Project shortcuts: detect a project's type and offer start/stop dev server, build and run.

## Architecture

MultiTasker has two plug-in points, both defined in [`crates/core`](crates/core):

- **`TaskProvider`** — where tasks come from. It only handles the small shared task model every task list can
  represent (title, description, category, due date, to-do/done).
- **`StateStore`** — where MultiTasker keeps what task lists have no field for: which tasks are in progress and on
  which device (and later links, deliverables and settings). Keeping this apart means "in progress" works the same
  with any provider.

[`crates/provider-sqlite`](crates/provider-sqlite) implements both for a local SQLite file. The desktop app in
[`src-tauri`](src-tauri) is built with [Tauri 2](https://tauri.app), with a React + Vite frontend in [`ui`](ui).

```
crates/core              domain model, TaskProvider and StateStore traits
crates/provider-sqlite   local SQLite provider + state store
crates/provider-javadashboard   tasks from JavaDashboard's API
crates/provider-todoist   tasks from Todoist's API, with a personal token
crates/provider-notion    tasks from a Notion database, with an integration token
crates/deliverables      copies of handed-in work, with every revision
crates/activity          activity detection: window in front, file saves, matching them to tasks
crates/sync              sync engine, sign-in, and storage backends (Google Drive, OneDrive, folder, S3)
src-tauri                desktop app: commands, windows, tray, browser bridge
extension                browser extension (Chrome, Edge, Brave)
ui                       React frontend (main window + widget)
```

### How sync works

Every change made on a computer (a task edited, started, an attachment added) is logged with a
[hybrid logical clock](https://cse.buffalo.edu/tech-reports/2014-04.pdf) stamp. Each computer uploads its changes as
numbered batches under `multitasker/v1/ops/<device>/`, and merges every other computer's batches. Conflicts resolve
field by field, newest stamp wins; deleting a task wins over edits to it. Because no file is ever written by two
computers, this works through file-sync clients that can't lock files.

## Privacy

MultiTasker has no server, accounts or tracking; see [PRIVACY.md](PRIVACY.md) for what it stores and where.

## Development

Prerequisites: [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) 20+, and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS (WebView2 ships with Windows 10/11).

```sh
npm install
npm run tauri dev       # run the app with hot reload
cargo test              # Rust tests
npm run typecheck       # frontend type check
npm run tauri build     # build an installer
```

### Browser preview

`npm run dev` on its own serves the UI at http://localhost:1420 (main window) and http://localhost:1420/widget.html
(widget). Outside Tauri, a mock backend with sample tasks stands in for Rust
([`ui/src/dev/mockBackend.ts`](ui/src/dev/mockBackend.ts)), so you can work on the UI in any browser. It only loads in
dev and never ships in builds.

Your data lives in the app data folder (on Windows, `%APPDATA%\dev.wilsoon.multitasker`).
