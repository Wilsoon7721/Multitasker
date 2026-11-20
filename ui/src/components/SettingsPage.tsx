import {
  CircleAlert,
  CloudOff,
  Database,
  Folder,
  FolderSync,
  KeyRound,
  Laptop,
  LoaderCircle,
  Radar,
  RefreshCw,
} from 'lucide-react';
import { useEffect, useState, type FormEvent, type ReactNode } from 'react';
import {
  api,
  onActivity,
  onSyncStatus,
  type FolderSuggestion,
  type S3Settings,
  type SyncStatus,
  type SyncTarget,
} from '../api';
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

function Section({
  icon,
  title,
  aside,
  children,
}: {
  icon: ReactNode;
  title: string;
  aside?: ReactNode;
  children: ReactNode;
}) {
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
  const [choosing, setChoosing] = useState<SyncTarget['kind'] | null>(null);
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

  const mode = choosing ?? target.kind;

  return (
    <Section icon={<FolderSync size={18} />} title="Sync" aside={status && <SyncBadge status={status} now={now} />}>
      <p className="settings-lead">
        Keep tasks, what's in progress and attachments the same on all your computers. Each computer only writes its own
        files, so they never overwrite each other.
      </p>

      <div className="choice-list" role="radiogroup" aria-label="Where to sync">
        <Choice
          selected={mode === 'off'}
          onSelect={() => {
            setChoosing(null);
            if (target.kind !== 'off') void choose({ kind: 'off' });
          }}
          icon={<CloudOff size={18} />}
          title="Off"
          detail="Tasks stay on this computer only."
        />
        <Choice
          selected={mode === 'folder'}
          onSelect={() => setChoosing('folder')}
          icon={<Folder size={18} />}
          title="Shared folder"
          detail="Pick a folder in OneDrive, Google Drive or Dropbox; their desktop apps carry it between computers."
        />
        <Choice
          selected={mode === 's3'}
          onSelect={() => setChoosing('s3')}
          icon={<Database size={18} />}
          title="S3-compatible storage"
          detail="AWS S3, Cloudflare R2, MinIO or any service with the S3 API."
        />
      </div>

      {mode === 's3' && (
        <S3Form
          current={target.kind === 's3' ? target : null}
          onSave={(settings, secret) =>
            run(async () => {
              await api.setSyncTarget({ kind: 's3', ...settings }, secret);
              setChoosing(null);
            })
          }
        />
      )}

      {mode === 'folder' && (
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
                    <button
                      key={s.path}
                      className="secondary"
                      onClick={() => void choose({ kind: 'folder', path: s.path })}
                      title={s.path}
                    >
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
            MultiTasker keeps its files in a <code>multitasker</code> folder inside the one you choose. Use the same
            folder on each computer.
          </p>
        </div>
      )}

      {status && status.state !== 'off' && (
        <>
          {status.client?.running === false && (
            <p className="settings-warning">
              <CircleAlert size={15} /> {status.client.name} isn't running, so changes are only saved in the folder on
              this computer. Start {status.client.name} and they'll go out.
            </p>
          )}
          {status.target.kind === 'folder' && status.client?.running !== false && (
            <p className="settings-hint">
              Changes reach your other computers once {status.client?.name ?? 'your sync app'} uploads them; a computer
              listed below with a recent "last seen" shows that round trip works.
            </p>
          )}
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
                    <span className="muted">
                      {d.this_device ? 'This computer' : `Last seen ${ago(d.last_seen, now)}`}
                    </span>
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

// For a shared folder MultiTasker only knows its files reached the folder; the sync app does the rest.
function SyncBadge({ status, now }: { status: SyncStatus; now: number }) {
  if (status.state === 'off') return <span className="badge">Off</span>;
  if (status.state === 'error') return <span className="badge error">Needs attention</span>;
  if (status.client?.running === false) return <span className="badge warn">Not leaving this computer</span>;
  if (status.state === 'syncing' || !status.last_synced) {
    return (
      <span className="badge">
        <LoaderCircle size={12} className="spin" /> Syncing
      </span>
    );
  }
  const when = ago(status.last_synced, now);
  if (status.target.kind === 'folder') return <span className="badge ok">Saved to folder {when}</span>;
  return <span className="badge ok">Synced {when}</span>;
}

function Choice(props: { selected: boolean; onSelect: () => void; icon: ReactNode; title: string; detail: string }) {
  return (
    <button
      className={`choice${props.selected ? ' selected' : ''}`}
      role="radio"
      aria-checked={props.selected}
      onClick={props.onSelect}
    >
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

const EMPTY_S3: S3Settings = { endpoint: '', region: 'us-east-1', bucket: '', prefix: '', access_key_id: '' };

function S3Form(props: { current: S3Settings | null; onSave: (s: S3Settings, secret: string) => Promise<boolean> }) {
  const { current, onSave } = props;
  const [form, setForm] = useState<S3Settings>(() => ({ ...EMPTY_S3, ...current }));
  const [secret, setSecret] = useState('');
  const [saving, setSaving] = useState(false);
  const set = (key: keyof S3Settings) => (e: { target: { value: string } }) =>
    setForm((f) => ({ ...f, [key]: e.target.value }));
  const sameKey = current?.access_key_id === form.access_key_id && (current?.endpoint ?? '') === (form.endpoint ?? '');
  const ready = form.bucket.trim() && form.region.trim() && form.access_key_id.trim() && (secret || sameKey);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    const clean = (v: string | null) => (v?.trim() ? v.trim() : null);
    await onSave(
      {
        endpoint: clean(form.endpoint),
        region: form.region.trim(),
        bucket: form.bucket.trim(),
        prefix: clean(form.prefix),
        access_key_id: form.access_key_id.trim(),
      },
      secret,
    );
    setSaving(false);
    setSecret('');
  };

  return (
    <form className="s3-form" onSubmit={submit}>
      <label className="field wide">
        <span>Endpoint</span>
        <input
          value={form.endpoint ?? ''}
          onChange={set('endpoint')}
          placeholder="Leave empty for AWS, e.g. https://ACCOUNT.r2.cloudflarestorage.com"
        />
      </label>
      <label className="field">
        <span>Bucket</span>
        <input value={form.bucket} onChange={set('bucket')} required />
      </label>
      <label className="field">
        <span>Region</span>
        <input value={form.region} onChange={set('region')} placeholder="us-east-1, or auto for R2" required />
      </label>
      <label className="field wide">
        <span>Folder in bucket (optional)</span>
        <input value={form.prefix ?? ''} onChange={set('prefix')} placeholder="e.g. multitasker-sync" />
      </label>
      <label className="field">
        <span>Access key ID</span>
        <input
          value={form.access_key_id}
          onChange={set('access_key_id')}
          autoComplete="off"
          spellCheck={false}
          required
        />
      </label>
      <label className="field">
        <span>Secret access key</span>
        <input
          type="password"
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
          placeholder={sameKey && current ? 'Saved, leave empty to keep it' : ''}
          autoComplete="off"
        />
      </label>
      <p className="settings-hint wide">
        <KeyRound size={13} /> The secret is kept in Windows Credential Manager, never in MultiTasker's files. Use a key
        that can only reach this bucket.
      </p>
      <div className="wide">
        <button type="submit" className="primary" disabled={!ready || saving}>
          {saving && <LoaderCircle size={14} className="spin" />}
          {current ? 'Save and reconnect' : 'Connect'}
        </button>
      </div>
    </form>
  );
}
