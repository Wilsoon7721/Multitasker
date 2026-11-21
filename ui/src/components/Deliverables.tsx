import { save } from '@tauri-apps/plugin-dialog';
import { ChevronDown, Download, ExternalLink, PackagePlus, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, onTasksChanged, type Attachment, type Deliverable, type Revision, type TaskView } from '../api';
import { elapsed } from '../time';
import { useDevices } from '../useDevices';
import { useTaskSource } from '../useTaskSource';
import type { Run } from '../useTasks';
import { KindIcon, pendingLabel } from './Attachments';

export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/** Copies bigger than this stay on the computer they were handed in on. */
const MAX_SYNC_SIZE = 100 * 1024 * 1024;

/** Why a version can't be opened here yet, if it can't. */
function notHere(r: Revision, nameOf: (id: string) => string, online: boolean): string | null {
  // An online task list holds the file itself, so opening it downloads it.
  if (r.available || (online && r.size <= MAX_SYNC_SIZE)) return null;
  const where = r.device ? nameOf(r.device) : 'another computer';
  return r.size > MAX_SYNC_SIZE ? `Over 100 MB, so it stays on ${where}` : `Arriving from ${where} with the next sync`;
}

/** The task's files on this computer that could be handed in. */
export function handInCandidates(task: TaskView, deviceId: string | null): Attachment[] {
  return task.attachments.filter(
    (a) =>
      !a.pending &&
      a.kind !== 'link' &&
      a.kind !== 'app' &&
      a.kind !== 'folder' &&
      (!a.device || !deviceId || a.device === deviceId),
  );
}

/** Handed-in links are kept as internet shortcuts. */
const isLink = (name: string) => /\.url$/i.test(name);
const shownName = (name: string) => (isLink(name) ? name.replace(/\.url$/i, '') : name);

async function saveCopy(run: Run, name: string, revision: Revision) {
  await run(async () => {
    const dest = await save({ defaultPath: name, title: `Save a copy of ${name}` });
    if (dest) await api.exportDeliverable(revision.id, dest);
  });
}

/** The work handed in for a task: a copy of each file as it was, with every revision. */
export function DeliverableList(props: { task: TaskView; run: Run; now: number; onHandIn: () => void }) {
  const { task, run, now, onHandIn } = props;
  const [items, setItems] = useState<Deliverable[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const { nameOf } = useDevices();
  const online = (useTaskSource()?.active ?? 'sqlite') !== 'sqlite';

  useEffect(() => {
    const load = () => void api.listDeliverables(task.id).then(setItems, () => setItems([]));
    load();
    return onTasksChanged(load);
  }, [task.id]);

  return (
    <section className="attachments deliverables" data-drop={`deliver:${task.id}`}>
      <header className="attachments-header">
        <h3>Handed in {items.length > 0 && <span className="count">{items.length}</span>}</h3>
        <span className="attach-buttons">
          <button type="button" className="ghost-pill" onClick={onHandIn} title="Keep a copy of finished work">
            <PackagePlus size={14} /> Hand in
          </button>
        </span>
      </header>

      {items.length === 0 ? (
        <p className="attachments-empty">
          Drop finished work here to keep a copy of it as it is now. Handing in the same file again keeps each version.
        </p>
      ) : (
        <ul className="attachment-list">
          {items.map((d) => {
            const [latest, ...older] = d.revisions;
            const expanded = open === d.id;
            const away = notHere(latest, nameOf, online);
            return (
              <li key={d.id} className="deliverable">
                <div className="attachment">
                  <button
                    className="attachment-open"
                    onClick={() => void run(() => api.openDeliverable(latest.id))}
                    title={away ?? (isLink(d.name) ? 'Open the link' : `Open a read-only copy of ${d.name}`)}
                    disabled={!!away}
                  >
                    <KindIcon
                      kind={isLink(d.name) ? 'link' : pendingLabel({ type: 'path', path: d.name }).kind}
                      size={18}
                    />
                    <span className="attachment-text">
                      <span className="attachment-name">{shownName(d.name)}</span>
                      <span className="attachment-where">
                        {d.revisions.length > 1 ? `Version ${latest.number} · ` : ''}
                        {elapsed(latest.added_at, now) === 'just now'
                          ? 'just now'
                          : `${elapsed(latest.added_at, now)} ago`}
                        {!isLink(d.name) && ` · ${sizeText(latest.size)}`}
                      </span>
                      {away && <span className="attachment-where">{away}</span>}
                    </span>
                  </button>
                  {confirm === d.id ? (
                    <>
                      <button className="secondary small" onClick={() => setConfirm(null)}>
                        Keep
                      </button>
                      <button
                        className="danger small"
                        onClick={() => void run(() => api.removeDeliverable(d.id)).then(() => setConfirm(null))}
                      >
                        Delete {d.revisions.length > 1 ? `all ${d.revisions.length}` : 'copy'}
                      </button>
                    </>
                  ) : (
                    <>
                      <button
                        className="icon-btn"
                        onClick={() => void saveCopy(run, d.name, latest)}
                        aria-label={`Save a copy of ${d.name}`}
                        title={away ?? 'Save a copy'}
                        disabled={!!away}
                      >
                        <Download size={15} />
                      </button>
                      {older.length > 0 && (
                        <button
                          className={`icon-btn${expanded ? ' on' : ''}`}
                          onClick={() => setOpen(expanded ? null : d.id)}
                          aria-expanded={expanded}
                          aria-label={`Earlier versions of ${d.name}`}
                          title="Earlier versions"
                        >
                          <ChevronDown size={15} />
                        </button>
                      )}
                      <button
                        className="icon-btn"
                        onClick={() => setConfirm(d.id)}
                        aria-label={`Delete the kept copies of ${d.name}`}
                        title="Delete the kept copies (the original file isn't touched)"
                      >
                        <Trash2 size={15} />
                      </button>
                    </>
                  )}
                </div>
                {expanded && (
                  <ul className="revision-list">
                    {older.map((r) => (
                      <li key={r.id} className="revision">
                        <span className="revision-text">
                          Version {r.number} · {new Date(r.added_at).toLocaleString()} · {sizeText(r.size)}
                          {notHere(r, nameOf, online) && ` · ${notHere(r, nameOf, online)}`}
                        </span>
                        <button
                          className="icon-btn"
                          onClick={() => void run(() => api.openDeliverable(r.id))}
                          aria-label={`Open version ${r.number}`}
                          title={notHere(r, nameOf, online) ?? 'Open a read-only copy'}
                          disabled={!!notHere(r, nameOf, online)}
                        >
                          <ExternalLink size={14} />
                        </button>
                        <button
                          className="icon-btn"
                          onClick={() => void saveCopy(run, d.name, r)}
                          aria-label={`Save a copy of version ${r.number}`}
                          title={notHere(r, nameOf, online) ?? 'Save a copy'}
                          disabled={!!notHere(r, nameOf, online)}
                        >
                          <Download size={14} />
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
