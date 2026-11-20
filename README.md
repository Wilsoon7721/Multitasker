# MultiTasker

An always-on-top focus list for people who juggle many tasks at once.

You start something, switch to something else, and forget the first thing exists until tomorrow. MultiTasker keeps
every task you've started pinned in a small widget at the top-right of your screen, so nothing in progress slips out
of sight.

## Status

Early development. What works today:

- A task list (add, edit, complete, delete) stored locally in SQLite.
- Mark tasks **in progress**; they appear in an always-on-top widget pinned to the top-right of your screen, with how
  long they've been going.
- Attach files, folders and links to tasks (by picker, or by dragging files from the desktop onto a task in the main
  window or the widget). Files stay where they are; MultiTasker keeps a reference and opens them in their default app.
- Expand the widget to see your to-do list and drag a task up into In Progress to start it.
- **Activity detection** (Windows): link apps, files and folders to a task, and MultiTasker notices when you're on
  them — from the window in front and from file saves. Tasks in progress show as *active now*; if you're clearly
  working on a task you haven't started, the widget offers to start it. Everything stays on your computer; window
  titles are only read in memory. Turn it off from the tray ("Detect what I'm working on").
- **Sync between computers**, through a shared folder (point MultiTasker at a folder in OneDrive, Google Drive or
  Dropbox on each computer and their desktop apps carry it) or any S3-compatible bucket (AWS S3, Cloudflare R2,
  MinIO; the secret key stays in Windows Credential Manager). Tasks, what's in progress and attachments merge across
  computers; each computer only writes its own files, so nothing gets overwritten.
- Tray icon: closing the main window keeps MultiTasker running with the widget.

Planned:

- Signing in to Google Drive or OneDrive directly, without their desktop apps.
- More task providers (your own dashboard, Notion, Todoist...).
- A Chrome extension to link browser tabs (AI chats, docs) to tasks, and reopen them on any of your devices.
- Deliverables: back up finished files to your storage backend, with revisions.
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
crates/activity          activity detection: window in front, file saves, matching them to tasks
crates/sync              sync engine and storage backends (shared folder, S3)
src-tauri                desktop app: commands, windows, tray
ui                       React frontend (main window + widget)
```

### How sync works

Every change made on a computer (a task edited, started, an attachment added) is logged with a
[hybrid logical clock](https://cse.buffalo.edu/tech-reports/2014-04.pdf) stamp. Each computer uploads its changes as
numbered batches under `multitasker/v1/ops/<device>/`, and merges every other computer's batches. Conflicts resolve
field by field, newest stamp wins; deleting a task wins over edits to it. Because no file is ever written by two
computers, this works through file-sync clients that can't lock files.

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

Your data lives in the app data folder (on Windows, `%APPDATA%\io.github.wilsoon7721.multitasker`).
