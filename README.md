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
- Tray icon: closing the main window keeps MultiTasker running with the widget.

Planned:

- Syncing MultiTasker's state and settings through a storage backend of your choice (Google Drive, OneDrive, S3).
- More task providers (your own dashboard, Notion, Todoist...).
- Detecting which task you're working on from file saves and the active window.
- A Chrome extension to link browser tabs (AI chats, docs) to tasks, and reopen them on any of your devices.
- Deliverables: attach finished files to a task, with revisions.
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
src-tauri                desktop app: commands, windows, tray
ui                       React frontend (main window + widget)
```

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
