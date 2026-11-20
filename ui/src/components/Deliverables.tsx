import { save } from '@tauri-apps/plugin-dialog';
import { ChevronDown, Download, ExternalLink, PackageCheck, PackagePlus, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, onTasksChanged, type Attachment, type Deliverable, type Revision, type TaskView } from '../api';
import { elapsed } from '../time';
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

async function saveCopy(run: Run, name: string, revision: Revision) {
  await run(async () => {
    const dest = await save({ defaultPath: name, title: `Save a copy of ${name}` });
    if (dest) await api.exportDeliverable(revision.id, dest);
  });
}

/** The work handed in for a task: a copy of each file as it was, with every revision. */
export function DeliverableList({ task, run, now }: { task: TaskView; run: Run; now: number }) {
  const [items, setItems] = useState<Deliverable[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);

  useEffect(() => {
    const load = () => void api.listDeliverables(task.id).then(setItems, () => setItems([]));
    load();
    return onTasksChanged(load);
  }, [task.id]);

  const handIn = () =>
    void run(async () => {
      const paths = await api.pickFiles('Hand in files');
      if (paths.length) await api.handIn(task.id, paths);
    });

  return (
    <section className="attachments deliverables" data-drop={`deliver:${task.id}`}>
      <header className="attachments-header">
        <h3>Handed in {items.length > 0 && <span className="count">{items.length}</span>}</h3>
        <span className="attach-buttons">
          <button type="button" className="ghost-pill" onClick={handIn} title="Keep a copy of finished work">
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
            return (
              <li key={d.id} className="deliverable">
                <div className="attachment">
                  <button
                    className="attachment-open"
                    onClick={() => void run(() => api.openDeliverable(latest.id))}
                    title={`Open a read-only copy of ${d.name}`}
                  >
                    <KindIcon kind={pendingLabel({ type: 'path', path: d.name }).kind} size={18} />
                    <span className="attachment-text">
                      <span className="attachment-name">{d.name}</span>
                      <span className="attachment-where">
                        {d.revisions.length > 1 ? `Version ${latest.number} · ` : ''}
                        {elapsed(latest.added_at, now) === 'just now'
                          ? 'just now'
                          : `${elapsed(latest.added_at, now)} ago`}
                        {' · '}
                        {sizeText(latest.size)}
                      </span>
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
                        title="Save a copy"
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
                        </span>
                        <button
                          className="icon-btn"
                          onClick={() => void run(() => api.openDeliverable(r.id))}
                          aria-label={`Open version ${r.number}`}
                          title="Open a read-only copy"
                        >
                          <ExternalLink size={14} />
                        </button>
                        <button
                          className="icon-btn"
                          onClick={() => void saveCopy(run, d.name, r)}
                          aria-label={`Save a copy of version ${r.number}`}
                          title="Save a copy"
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

/** Asked after finishing a task that has files attached: keep copies of them as handed-in work? */
export function HandInDialog(props: { task: TaskView; deviceId: string | null; run: Run; onClose: () => void }) {
  const { task, deviceId, run, onClose } = props;
  const candidates = handInCandidates(task, deviceId);
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(candidates.map((a) => a.target)));
  const [extra, setExtra] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const paths = [...chosen, ...extra];
  const toggle = (path: string) =>
    setChosen((cur) => {
      const next = new Set(cur);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const submit = async () => {
    setBusy(true);
    const ok = await run(() => api.handIn(task.id, paths));
    setBusy(false);
    if (ok) onClose();
  };

  return (
    <div className="dialog-scrim" onClick={onClose}>
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="hand-in-title"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="hand-in-title">
          <PackageCheck size={18} /> Hand in your work?
        </h3>
        <p>
          MultiTasker keeps a copy of each file as it is now, so you can get this version of "{task.title}" back later,
          even after the files change.
        </p>
        <ul className="hand-in-list">
          {candidates.map((a) => (
            <li key={a.id}>
              <label>
                <input type="checkbox" checked={chosen.has(a.target)} onChange={() => toggle(a.target)} />
                <KindIcon kind={a.kind} />
                <span>{a.name}</span>
              </label>
            </li>
          ))}
          {extra.map((path) => (
            <li key={path}>
              <label>
                <input type="checkbox" checked onChange={() => setExtra((cur) => cur.filter((p) => p !== path))} />
                <KindIcon kind={pendingLabel({ type: 'path', path }).kind} />
                <span>{pendingLabel({ type: 'path', path }).name}</span>
              </label>
            </li>
          ))}
        </ul>
        <button
          type="button"
          className="ghost-pill"
          onClick={() =>
            void api.pickFiles('Hand in files').then((picked) => setExtra((cur) => [...new Set([...cur, ...picked])]))
          }
        >
          <PackagePlus size={14} /> Other files…
        </button>
        <div className="dialog-actions">
          <button className="secondary" onClick={onClose}>
            Not now
          </button>
          <button className="primary" disabled={!paths.length || busy} onClick={() => void submit()}>
            {paths.length === 1 ? 'Hand in 1 file' : `Hand in ${paths.length} files`}
          </button>
        </div>
      </div>
    </div>
  );
}
