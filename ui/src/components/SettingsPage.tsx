import { CircleAlert, CloudOff, Folder, FolderSync, Laptop, LoaderCircle, Radar, RefreshCw } from 'lucide-react';
import { useEffect, useState, type ReactNode } from 'react';
import { api, onActivity, onSyncStatus, type FolderSuggestion, type SyncStatus, type SyncTarget } from '../api';
import { elapsed, useNow } from '../time';
import type { Run } from '../useTasks';

const ago = (iso: string, now: number) => {
  const e = elapsed(iso, now);
  return e === 'just now' ? e : `${e} ago`;
};

export function SettingsPage({ run }: { run: Run }) {
  return (
    <>
      <SyncSection run={run} />
      <DetectionSection run={run} />
    </>
  );
}

function Section({ icon, title, aside, children }: { icon: ReactNode; title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="settings-section">
      <header>
        {icon}
        <h2>{title}</h2>
        {aside}
      </header>
      {children}
    </section>
  );
}

function SyncSection({ run }: { run: Run }) {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [suggestions, setSuggestions] = useState<FolderSuggestion[]>([]);
  const [choosingFolder, setChoosingFolder] = useState(false);
  const now = useNow(15_000);

  useEffect(() => {
    void api.syncStatus().then(setStatus, () => {});
    void api.syncFolderSuggestions().then(setSuggestions, () => {});
    return onSyncStatus(setStatus);
  }, []);

  const target = status?.target ?? { kind: 'off' };
  const choose = (next: SyncTarget) => run(() => api.setSyncTarget(next));
  const pick = () =>
    void run(async () => {
      const path = await api.pickFolder();
      if (path) await api.setSyncTarget({ kind: 'folder', path });
    });

  const folderMode = target.kind === 'folder' || choosingFolder;

  return (
    <Section icon={<FolderSync size={18} />} title="Sync" aside={status && <SyncBadge status={status} now={now} />}>
      <p className="settings-lead">
        Keep tasks, what's in progress and attachments the same on all your computers. Each computer only writes its own
        files, so they never overwrite each other.
      </p>

      <div className="choice-list" role="radiogroup" aria-label="Where to sync">
        <Choice
          selected={!folderMode}
          onSelect={() => {
            setChoosingFolder(false);
            if (target.kind !== 'off') void choose({ kind: 'off' });
          }}
          icon={<CloudOff size={18} />}
          title="Off"
          detail="Tasks stay on this computer only."
        />
        <Choice
          selected={folderMode}
          onSelect={() => setChoosingFolder(true)}
          icon={<Folder size={18} />}
          title="Shared folder"
          detail="Pick a folder in OneDrive, Google Drive or Dropbox; their desktop apps carry it between computers."
        />
      </div>

      {folderMode && (
        <div className="folder-setup">
          {target.kind === 'folder' ? (
            <div className="folder-current">
              <Folder size={16} className="kind kind-folder" />
              <span className="folder-path" title={target.path}>
                {target.path}
              </span>
              <button className="secondary" onClick={pick}>
                Change…
              </button>
            </div>
          ) : (
            <>
              {suggestions.length > 0 && (
                <div className="suggestions">
                  <span className="muted">Found on this computer:</span>
                  {suggestions.map((s) => (
                    <button key={s.path} className="secondary" onClick={() => void choose({ kind: 'folder', path: s.path })} title={s.path}>
                      <Folder size={14} /> {s.label}
                    </button>
                  ))}
                </div>
              )}
              <button className="primary" onClick={pick}>
                Choose folder…
              </button>
            </>
          )}
          <p className="settings-hint">
            MultiTasker keeps its files in a <code>multitasker</code> folder inside the one you choose. Use the same folder
            on each computer.
          </p>
        </div>
      )}

      {status && status.state !== 'off' && (
        <>
          {status.error && (
            <p className="settings-error">
              <CircleAlert size={15} /> {status.error}
            </p>
          )}
          <div className="sync-actions">
            <button className="secondary" onClick={() => void run(api.syncNow)} disabled={status.state === 'syncing'}>
              <RefreshCw size={14} /> Sync now
            </button>
          </div>

          {status.devices.length > 0 && (
            <>
              <h3 className="settings-subhead">Computers</h3>
              <ul className="device-list">
                {status.devices.map((d) => (
                  <li key={d.id}>
                    <Laptop size={16} />
                    <span className="device-name">{d.name}</span>
                    <span className="muted">{d.this_device ? 'This computer' : `Last synced ${ago(d.last_seen, now)}`}</span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </>
      )}
    </Section>
  );
}

function SyncBadge({ status, now }: { status: SyncStatus; now: number }) {
  if (status.state === 'off') return <span className="badge">Off</span>;
  if (status.state === 'error') return <span className="badge error">Needs attention</span>;
  if (status.state === 'syncing' || !status.last_synced) {
    return (
      <span className="badge">
        <LoaderCircle size={12} className="spin" /> Syncing
      </span>
    );
  }
  return <span className="badge ok">Synced {ago(status.last_synced, now)}</span>;
}

function Choice(props: { selected: boolean; onSelect: () => void; icon: ReactNode; title: string; detail: string }) {
  return (
    <button className={`choice${props.selected ? ' selected' : ''}`} role="radio" aria-checked={props.selected} onClick={props.onSelect}>
      <span className="choice-icon">{props.icon}</span>
      <span className="choice-text">
        <span className="choice-title">{props.title}</span>
        <span className="choice-detail">{props.detail}</span>
      </span>
      <span className="choice-dot" aria-hidden />
    </button>
  );
}

function DetectionSection({ run }: { run: Run }) {
  const [enabled, setEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    void api.detectionEnabled().then(setEnabled, () => {});
    return onActivity((s) => setEnabled(s.enabled));
  }, []);

  return (
    <Section icon={<Radar size={18} />} title="Activity detection">
      <label className="toggle-row">
        <span>
          <span className="choice-title">Detect what I'm working on</span>
          <span className="choice-detail">
            Watches the window in front and linked files to mark tasks active and suggest ones to start. Nothing leaves
            this computer.
          </span>
        </span>
        <input
          type="checkbox"
          role="switch"
          className="switch"
          checked={!!enabled}
          disabled={enabled === null}
          onChange={(e) => void run(() => api.setDetection(e.target.checked))}
        />
      </label>
    </Section>
  );
}
