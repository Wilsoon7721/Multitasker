import { CircleAlert, CircleCheck, Link2, LoaderCircle, PackageCheck, PackagePlus } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { api, type FileCheck, type LinkCheck, type TaskView } from '../api';
import { elapsed } from '../time';
import type { Run } from '../useTasks';
import { KindIcon, pendingLabel } from './Attachments';
import { handInCandidates, sizeText } from './Deliverables';

/** How often files open in another program are checked again, to hand them in once it lets go. */
const RECHECK_MS = 2000;

interface LinkItem {
  url: string;
  check: LinkCheck | null;
}

/** Hands in work, after checking each file is saved and sound, and each link opens for others. */
export function HandInDialog(props: {
  task: TaskView;
  deviceId: string | null;
  run: Run;
  onClose: () => void;
  /** Files to offer instead of the task's attachments, such as ones just dropped in. */
  paths?: string[];
}) {
  const { task, deviceId, run, onClose } = props;
  const candidates = props.paths ? [] : handInCandidates(task, deviceId);
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(candidates.map((a) => a.target)));
  const [extra, setExtra] = useState<string[]>(props.paths ?? []);
  const [checks, setChecks] = useState<Map<string, FileCheck>>(new Map());
  const [anyway, setAnyway] = useState<Set<string>>(new Set());
  const [links, setLinks] = useState<LinkItem[]>([]);
  const [linkText, setLinkText] = useState('');
  const [busy, setBusy] = useState(false);
  // Files left to hand in once the program holding them closes.
  const [waiting, setWaiting] = useState<string[]>([]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const paths = useMemo(() => [...chosen, ...extra.filter((p) => !chosen.has(p))], [chosen, extra]);
  const pathsKey = paths.join('\n');

  // Checks are re-run while anything is still open elsewhere, so it's noticed the moment it closes.
  const anyOpen = [...checks.values()].some((c) => c.open_in.length > 0);
  useEffect(() => {
    let stop = false;
    const check = () =>
      void api.checkHandIn(task.id, paths).then(
        (list) => !stop && setChecks(new Map(list.map((c) => [c.path, c]))),
        () => {},
      );
    check();
    const timer = anyOpen ? setInterval(check, RECHECK_MS) : undefined;
    return () => {
      stop = true;
      clearInterval(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task.id, pathsKey, anyOpen]);

  const state = (path: string): 'ready' | 'open' | 'problem' | 'checking' => {
    const c = checks.get(path);
    if (!c) return 'checking';
    if (anyway.has(path)) return 'ready';
    if (c.problem) return 'problem';
    if (c.open_in.length) return 'open';
    return 'ready';
  };
  const linkReady = (l: LinkItem) => l.check && (l.check.verdict === 'ok' || anyway.has(l.url));
  const ready = paths.filter((p) => state(p) === 'ready');
  const open = paths.filter((p) => state(p) === 'open');
  const readyLinks = links.filter(linkReady);

  // Files waited on are handed in as soon as nothing holds them any more.
  useEffect(() => {
    const free = waiting.filter((p) => state(p) === 'ready');
    if (!free.length) return;
    setWaiting((cur) => cur.filter((p) => !free.includes(p)));
    void run(() => api.handIn(task.id, free)).then((ok) => ok && waiting.length === free.length && onClose());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [checks, anyway]);

  const toggle = (path: string) =>
    setChosen((cur) => {
      const next = new Set(cur);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const addLink = () => {
    const url = linkText.trim();
    if (!url || links.some((l) => l.url === url)) return;
    setLinkText('');
    setLinks((cur) => [...cur, { url, check: null }]);
    void api.checkLink(url).then(
      (check) => setLinks((cur) => cur.map((l) => (l.url === url ? { ...l, check } : l))),
      (e) =>
        setLinks((cur) =>
          cur.map((l) =>
            l.url === url ? { ...l, check: { url, title: null, verdict: 'unreachable', message: String(e) } } : l,
          ),
        ),
    );
  };

  const submit = async () => {
    setBusy(true);
    const ok = await run(async () => {
      if (ready.length) await api.handIn(task.id, ready);
      for (const l of readyLinks) await api.handInLink(task.id, l.check?.url ?? l.url, l.check?.title ?? null);
    });
    setBusy(false);
    if (!ok) return;
    if (open.length) setWaiting(open);
    else onClose();
  };

  const count = ready.length + readyLinks.length;
  const label = waiting.length
    ? 'Waiting…'
    : open.length && count
      ? `Hand in ${count} now, ${open.length} when closed`
      : open.length
        ? `Hand in when ${open.length === 1 ? "it's" : "they're"} closed`
        : count === 1
          ? 'Hand in 1 item'
          : `Hand in ${count} items`;

  return (
    <div className="dialog-scrim" onClick={onClose}>
      <div
        className="dialog hand-in-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="hand-in-title"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="hand-in-title">
          <PackageCheck size={18} /> Hand in work for “{task.title}”
        </h3>
        <p>
          MultiTasker keeps a copy of each file as it is now, so you can get this version back later, even after the
          files change. Handing in a task's work doesn't mark it done.
        </p>

        <ul className="hand-in-list">
          {paths.map((path) => {
            const attachment = candidates.find((a) => a.target === path);
            const label = pendingLabel({ type: 'path', path });
            const c = checks.get(path);
            const s = state(path);
            const isWaiting = waiting.includes(path);
            return (
              <li key={path} className={`hand-in-item ${s}`}>
                <label>
                  <input
                    type="checkbox"
                    checked={chosen.has(path) || extra.includes(path)}
                    disabled={isWaiting}
                    onChange={() => (attachment ? toggle(path) : setExtra((cur) => cur.filter((p) => p !== path)))}
                  />
                  <KindIcon kind={attachment?.kind ?? label.kind} />
                  <span className="hand-in-text">
                    <span className="hand-in-name">{attachment?.name ?? label.name}</span>
                    <FileStatus check={c} waiting={isWaiting} excused={anyway.has(path)} />
                  </span>
                </label>
                {(s === 'open' || s === 'problem') && (
                  <button
                    type="button"
                    className="link-button"
                    onClick={() => setAnyway((cur) => new Set(cur).add(path))}
                  >
                    Hand in anyway
                  </button>
                )}
              </li>
            );
          })}
          {links.map((l) => (
            <li key={l.url} className={`hand-in-item ${linkReady(l) ? 'ready' : l.check ? 'problem' : 'checking'}`}>
              <label>
                <input type="checkbox" checked onChange={() => setLinks((cur) => cur.filter((x) => x.url !== l.url))} />
                <Link2 size={16} className="kind kind-link" />
                <span className="hand-in-text">
                  <span className="hand-in-name">{l.check?.title ?? l.url.replace(/^https?:\/\/(www\.)?/, '')}</span>
                  <LinkStatus check={l.check} excused={anyway.has(l.url)} />
                </span>
              </label>
              {l.check && l.check.verdict !== 'ok' && !anyway.has(l.url) && (
                <button
                  type="button"
                  className="link-button"
                  onClick={() => setAnyway((cur) => new Set(cur).add(l.url))}
                >
                  Hand in anyway
                </button>
              )}
            </li>
          ))}
        </ul>

        <div className="hand-in-add">
          <button
            type="button"
            className="ghost-pill"
            onClick={() =>
              void api.pickFiles('Hand in files').then((picked) => setExtra((cur) => [...new Set([...cur, ...picked])]))
            }
          >
            <PackagePlus size={14} /> Other files…
          </button>
          <form
            className="hand-in-link"
            onSubmit={(e) => {
              e.preventDefault();
              addLink();
            }}
          >
            <input
              value={linkText}
              onChange={(e) => setLinkText(e.target.value)}
              placeholder="Or paste a link, like a shared doc"
              aria-label="Link to hand in"
            />
            <button type="submit" className="secondary small" disabled={!linkText.trim()}>
              Check
            </button>
          </form>
        </div>

        <div className="dialog-actions">
          <button className="secondary" onClick={onClose}>
            {waiting.length ? 'Stop waiting' : 'Not now'}
          </button>
          <button
            className="primary"
            disabled={busy || waiting.length > 0 || (count === 0 && open.length === 0)}
            onClick={() => void submit()}
          >
            {label}
          </button>
        </div>
      </div>
    </div>
  );
}

function FileStatus({ check, waiting, excused }: { check?: FileCheck; waiting: boolean; excused: boolean }) {
  if (!check)
    return (
      <span className="hand-in-status">
        <LoaderCircle size={12} className="spin" /> Checking…
      </span>
    );
  if (check.problem && !excused)
    return (
      <span className="hand-in-status bad">
        <CircleAlert size={12} /> {check.problem}
      </span>
    );
  if (check.open_in.length && !excused) {
    const who = check.open_in.join(', ');
    return (
      <span className="hand-in-status bad">
        <CircleAlert size={12} />{' '}
        {waiting ? `Waiting for ${who} to close…` : `Open in ${who}, so unsaved changes won't be included`}
      </span>
    );
  }
  if (check.same_as_last) return <span className="hand-in-status">Same as what you handed in last time</span>;
  const saved = check.modified ? elapsed(check.modified) : null;
  return (
    <span className="hand-in-status">
      {saved ? (saved === 'just now' ? 'Saved just now' : `Saved ${saved} ago`) : ''}
      {saved ? ' · ' : ''}
      {sizeText(check.size)}
    </span>
  );
}

function LinkStatus({ check, excused }: { check: LinkCheck | null; excused: boolean }) {
  if (!check)
    return (
      <span className="hand-in-status">
        <LoaderCircle size={12} className="spin" /> Checking it opens for others…
      </span>
    );
  if (check.verdict === 'ok')
    return (
      <span className="hand-in-status good">
        <CircleCheck size={12} /> {check.message}
      </span>
    );
  return (
    <span className={`hand-in-status ${excused ? '' : check.verdict === 'unchecked' ? 'warn' : 'bad'}`}>
      <CircleAlert size={12} /> {check.message}
    </span>
  );
}
