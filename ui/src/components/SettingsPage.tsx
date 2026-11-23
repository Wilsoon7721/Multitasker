import {
  BellRing,
  PictureInPicture2,
  HardDrive,
  LayoutDashboard,
  ListChecks,
  ListTodo,
  NotebookText,
  CircleAlert,
  Cloud,
  CloudOff,
  LogIn,
  LogOut,
  Database,
  Folder,
  FolderSync,
  KeyRound,
  Laptop,
  LoaderCircle,
  Radar,
  RefreshCw,
  Plus,
  X,
  AppWindow,
} from 'lucide-react';
import { useEffect, useState, type FormEvent, type ReactNode } from 'react';
import {
  type CloudService,
  type FolderSuggestion,
  type NotionDatabases,
  type OnTop,
  type ReminderSettings,
  type S3Settings,
  type SettingsSync,
  type SyncStatus,
  type SyncTarget,
  type TaskSourceView,
  type UpdateInfo,
  api,
  onActivity,
  onSettingsSynced,
  onSyncStatus,
  onTasksChanged,
  onUpdateReady,
} from '../api';
import { AppPicker } from './AppPicker';
import { STILL_ON_CHOICES, STILL_ON_KEY, stillOnHours } from './StillOn';
import { guessProvider, providerById, S3_PROVIDERS } from '../s3Providers';
import { elapsed, useNow } from '../time';
import type { Run } from '../useTasks';
import { useTaskSource } from '../useTaskSource';

const ago = (iso: string, now: number) => {
  const e = elapsed(iso, now);
  return e === 'just now' ? e : `${e} ago`;
};

export function SettingsPage({ run }: { run: Run }) {
  const source = useTaskSource();
  // An online task list keeps in-progress marks and attachments itself, so there's nothing to sync.
  const syncs = source?.active === 'sqlite';
  // Settings that arrive from another computer show by reading them again.
  const [synced, setSynced] = useState(0);
  useEffect(() => onSettingsSynced(() => setSynced((n) => n + 1)), []);
  return (
    <>
      <AppSection key={`app-${synced}`} run={run} />
      <TaskListSection run={run} />
      {syncs && <SyncSection run={run} />}
      <WidgetSection key={`widget-${synced}`} run={run} />
      <RemindersSection key={`reminders-${synced}`} run={run} />
      <DetectionSection key={`detection-${synced}`} run={run} />
    </>
  );
}

/** Where settings travel between computers, and whether that's working. */
function SettingsSyncBadge() {
  const [status, setStatus] = useState<SettingsSync | null>(null);
  const now = useNow(15_000);

  useEffect(() => {
    const load = () => void api.settingsSync().then(setStatus, () => {});
    load();
    const timer = setInterval(load, 15_000);
    const stop = onSettingsSynced(load);
    return () => {
      clearInterval(timer);
      stop();
    };
  }, []);

  if (!status) return null;
  if (status.error) {
    return (
      <span className="badge warn" title={status.error}>
        Settings not shared
      </span>
    );
  }
  if (!status.last_synced) return null;
  return (
    <span className="badge ok" title={`Your settings follow you to every computer through ${status.via}.`}>
      Settings shared {ago(status.last_synced, now)}
    </span>
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
  const [question, setQuestion] = useState<{ where: string | null; resolve: (ok: boolean) => void } | null>(null);

  // Leaving a location that's already been synced to splits the tasks between the two, so ask first.
  const confirmMove = (where: string | null) =>
    new Promise<boolean>((resolve) => {
      if (!status || status.target.kind === 'off' || !status.last_synced) return resolve(true);
      setQuestion({ where, resolve });
    });

  const choose = async (next: SyncTarget, where: string | null) => {
    if (await confirmMove(where)) await run(() => api.setSyncTarget(next));
  };
  const pick = () =>
    void run(async () => {
      const path = await api.pickFolder();
      if (
        path &&
        path !== (target.kind === 'folder' ? target.path : null) &&
        (await confirmMove('the folder you picked'))
      ) {
        await api.setSyncTarget({ kind: 'folder', path });
      }
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
            if (target.kind !== 'off') void choose({ kind: 'off' }, null);
          }}
          icon={<CloudOff size={18} />}
          title="Off"
          detail="Tasks stay on this computer only."
        />
        <Choice
          selected={mode === 'google_drive'}
          onSelect={() => setChoosing('google_drive')}
          icon={<Cloud size={18} />}
          title="Google Drive"
          detail="Sign in with Google. Kept in a hidden app folder, so nothing shows up in your Drive."
        />
        <Choice
          selected={mode === 'one_drive'}
          onSelect={() => setChoosing('one_drive')}
          icon={<Cloud size={18} />}
          title="OneDrive"
          detail="Sign in with a personal Microsoft account. Kept in its own Apps/MultiTasker folder."
        />
        <Choice
          selected={mode === 'folder'}
          onSelect={() => setChoosing('folder')}
          icon={<Folder size={18} />}
          title="Shared folder"
          detail="A folder in Dropbox or another sync app; that app carries it between computers while it's running."
        />
        <Choice
          selected={mode === 's3'}
          onSelect={() => setChoosing('s3')}
          icon={<Database size={18} />}
          title="S3-compatible storage"
          detail="AWS S3, Cloudflare R2, MinIO or any service with the S3 API."
        />
      </div>

      {(mode === 'google_drive' || mode === 'one_drive') && (
        <CloudSetup
          service={mode === 'google_drive' ? 'google' : 'microsoft'}
          signedInAs={target.kind === mode ? target.account : null}
          run={run}
          onDone={() => setChoosing(null)}
          onSignOut={() => {
            setChoosing(null);
            void choose({ kind: 'off' }, null);
          }}
          confirm={() =>
            confirmMove(
              target.kind === mode
                ? `a different ${mode === 'google_drive' ? 'Google' : 'Microsoft'} account`
                : mode === 'google_drive'
                  ? 'Google Drive'
                  : 'OneDrive',
            )
          }
        />
      )}

      {mode === 's3' && (
        <S3Form
          current={target.kind === 's3' ? target : null}
          onSave={(settings, secret) =>
            run(async () => {
              const same =
                target.kind === 's3' &&
                target.bucket === settings.bucket &&
                (target.endpoint ?? null) === settings.endpoint &&
                (target.prefix ?? null) === settings.prefix;
              if (!same && !(await confirmMove(`the ${settings.bucket} bucket`))) return;
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
                      onClick={() => void choose({ kind: 'folder', path: s.path }, `your ${s.label} folder`)}
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

      {status && status.state !== 'off' && mode === target.kind && (
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
      {question && status && (
        <MoveDialog
          current={describeTarget(status.target)}
          where={question.where}
          others={status.devices.filter((d) => !d.this_device).map((d) => d.name)}
          onAnswer={(ok) => {
            question.resolve(ok);
            setQuestion(null);
          }}
        />
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

type SourceKind = TaskSourceView['kind'];

function TaskListSection({ run }: { run: Run }) {
  const [source, setSource] = useState<TaskSourceView | null>(null);
  const [choosing, setChoosing] = useState<SourceKind | null>(null);
  const [waiting, setWaiting] = useState(false);

  useEffect(() => {
    const load = () => void api.taskSource().then(setSource, () => {});
    load();
    return onTasksChanged(load);
  }, []);

  if (!source) return null;

  const mode = choosing ?? source.kind;
  const connected = source.kind !== 'local' && source.active !== 'sqlite';
  const account = source.kind !== 'local' ? source.account : null;
  // Only builds made with a WilsoonID registration can use JavaDashboard.
  const offerDashboard = source.wilsoon_available || source.kind === 'java_dashboard';

  const signIn = async () => {
    setWaiting(true);
    const ok = await run(api.signInWilsoon);
    setWaiting(false);
    if (ok) setChoosing(null);
  };

  const signedIn = (icon: ReactNode, text: string) => (
    <div className="folder-current">
      {icon}
      <span className="folder-path account">{text}</span>
      <button className="secondary" onClick={() => void run(api.useLocalTasks)}>
        <LogOut size={14} /> Use tasks on this computer
      </button>
    </div>
  );
  const lost = (what: string) =>
    source.kind === mode &&
    !connected && (
      <p className="settings-warning">
        <CircleAlert size={15} /> {what} has gone, so tasks on this computer are showing for now.
      </p>
    );

  return (
    <Section icon={<ListChecks size={18} />} title="Task list" aside={<SettingsSyncBadge />}>
      <div className="choice-list" role="radiogroup" aria-label="Where tasks come from">
        <Choice
          selected={mode === 'local'}
          onSelect={() => {
            setChoosing(null);
            if (source.kind !== 'local') void run(api.useLocalTasks);
          }}
          icon={<HardDrive size={18} />}
          title="On this computer"
          detail="Kept by MultiTasker; sync carries them to your other computers."
        />
        <Choice
          selected={mode === 'todoist'}
          onSelect={() => setChoosing('todoist')}
          icon={<ListTodo size={18} />}
          title="Todoist"
          detail="Your Todoist tasks, connected with your own API token."
        />
        <Choice
          selected={mode === 'notion'}
          onSelect={() => setChoosing('notion')}
          icon={<NotebookText size={18} />}
          title="Notion"
          detail="A task database in Notion, connected with an integration token."
        />
        {offerDashboard && (
          <Choice
            selected={mode === 'java_dashboard'}
            onSelect={() => setChoosing('java_dashboard')}
            icon={<LayoutDashboard size={18} />}
            title="JavaDashboard"
            detail="Your dashboard's task board, signed in with WilsoonID."
          />
        )}
      </div>

      {mode === 'todoist' && (
        <div className="cloud-setup">
          {lost('The Todoist token')}
          {source.kind === 'todoist' && connected ? (
            signedIn(<ListTodo size={16} className="kind kind-link" />, `Connected as ${account}`)
          ) : (
            <TodoistForm run={run} onConnected={() => setChoosing(null)} />
          )}
          <p className="settings-hint">
            Projects show as categories. Starting a task adds an <code>in-progress</code> label, and attachments are
            kept as comments on the task, so your other computers see them too. Handed-in work is attached to a comment.
          </p>
        </div>
      )}

      {mode === 'notion' && (
        <div className="cloud-setup">
          {lost('The Notion token')}
          {source.kind === 'notion' && connected ? (
            signedIn(<NotebookText size={16} className="kind kind-link" />, `${source.name} in ${source.account}`)
          ) : (
            <NotionForm run={run} onConnected={() => setChoosing(null)} />
          )}
          <p className="settings-hint">
            MultiTasker uses the database's Status column for in progress and done (or a Done checkbox), its date for
            the due date and a select for the category. It adds any of those it lacks, plus Started, Started on and an
            Attachments column, which keeps files as links and remembers which computer they're on. Handed-in work is
            uploaded into the task's page.
          </p>
        </div>
      )}

      {mode === 'java_dashboard' && (
        <div className="cloud-setup">
          {!waiting && lost('The WilsoonID sign-in')}
          {source.kind === 'java_dashboard' && connected && !waiting ? (
            signedIn(<LayoutDashboard size={16} className="kind kind-link" />, `Signed in as ${account}`)
          ) : waiting ? (
            <div className="sign-in-wait">
              <LoaderCircle size={16} className="spin" />
              <span>Finish signing in to WilsoonID in your browser…</span>
              <button className="secondary" onClick={() => void api.cancelSignIn()}>
                Cancel
              </button>
            </div>
          ) : (
            <button
              className="primary sign-in"
              onClick={() => void signIn()}
              disabled={!source.wilsoon_available}
              title={source.wilsoon_available ? undefined : 'This build has no WilsoonID registration.'}
            >
              <LogIn size={15} /> Sign in with WilsoonID
            </button>
          )}
          <p className="settings-hint">
            Tasks on this computer stay saved while you use JavaDashboard. JavaDashboard tasks have due dates but no
            times, so they don't get timed reminders.
          </p>
        </div>
      )}
    </Section>
  );
}

function NotionForm({ run, onConnected }: { run: Run; onConnected: () => void }) {
  const [token, setToken] = useState('');
  const [found, setFound] = useState<NotionDatabases | null>(null);
  const [database, setDatabase] = useState('');
  const [busy, setBusy] = useState(false);

  const look = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    let result: NotionDatabases | null = null;
    await run(async () => {
      result = await api.notionDatabases(token);
    });
    setBusy(false);
    setFound(result);
    setDatabase((result as NotionDatabases | null)?.databases[0]?.id ?? '');
  };

  const connect = async () => {
    setBusy(true);
    const ok = await run(() => api.connectNotion(token, database));
    setBusy(false);
    if (ok) {
      setToken('');
      setFound(null);
      onConnected();
    }
  };

  return (
    <form className="s3-form" onSubmit={look}>
      <label className="field wide">
        <span>Integration token</span>
        <input
          type="password"
          value={token}
          onChange={(e) => {
            setToken(e.target.value);
            setFound(null);
          }}
          autoComplete="off"
          spellCheck={false}
          required
        />
      </label>
      <p className="settings-hint wide">
        <KeyRound size={13} /> In Notion, open Settings → Connections → Develop or manage integrations, make an internal
        integration and copy its token. Then open your task database, choose ••• → Connections and add the integration
        so MultiTasker can see it. The token is kept in Windows Credential Manager.
      </p>
      {found && found.databases.length === 0 && (
        <p className="settings-warning wide">
          <CircleAlert size={15} /> {found.workspace} hasn't shared any databases with this integration yet. Add it
          under ••• → Connections on your task database (or the page it's in), then look again.
        </p>
      )}
      {found && found.databases.length > 0 && (
        <label className="field wide">
          <span>Task database in {found.workspace}</span>
          <select value={database} onChange={(e) => setDatabase(e.target.value)}>
            {found.databases.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="wide">
        {found && found.databases.length > 0 ? (
          <button type="button" className="primary" disabled={!database || busy} onClick={() => void connect()}>
            {busy && <LoaderCircle size={14} className="spin" />}
            Connect
          </button>
        ) : (
          <button type="submit" className="primary" disabled={!token.trim() || busy}>
            {busy && <LoaderCircle size={14} className="spin" />}
            {found ? 'Look again' : 'Find databases'}
          </button>
        )}
      </div>
    </form>
  );
}

function TodoistForm({ run, onConnected }: { run: Run; onConnected: () => void }) {
  const [token, setToken] = useState('');
  const [saving, setSaving] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    const ok = await run(() => api.connectTodoist(token));
    setSaving(false);
    if (ok) {
      setToken('');
      onConnected();
    }
  };

  return (
    <form className="s3-form" onSubmit={submit}>
      <label className="field wide">
        <span>API token</span>
        <input
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          autoComplete="off"
          spellCheck={false}
          required
        />
      </label>
      <p className="settings-hint wide">
        <KeyRound size={13} /> In Todoist, open Settings → Integrations → Developer and copy your API token. It's kept
        in Windows Credential Manager, never in MultiTasker's files. Tasks on this computer stay saved while you use
        Todoist.
      </p>
      <div className="wide">
        <button type="submit" className="primary" disabled={!token.trim() || saving}>
          {saving && <LoaderCircle size={14} className="spin" />}
          Connect
        </button>
      </div>
    </form>
  );
}

const SHORTCUTS = ['Ctrl+Alt+Space', 'Ctrl+Shift+Space', 'Ctrl+Alt+K', 'Ctrl+Shift+K', null];

function AppSection({ run }: { run: Run }) {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [startup, setStartup] = useState<boolean | null>(null);
  const [checking, setChecking] = useState(false);
  const [upToDate, setUpToDate] = useState(false);
  const [shortcut, setShortcut] = useState<string | null | undefined>(undefined);
  const [shortcutProblem, setShortcutProblem] = useState<string | null>(null);

  useEffect(() => {
    const load = () => void api.updateInfo().then(setInfo, () => {});
    load();
    void api.startWithWindows().then(setStartup, () => {});
    void api.switcherShortcut().then(setShortcut, () => {});
    void api.switcherProblem().then(setShortcutProblem, () => {});
    return onUpdateReady(load);
  }, []);

  const check = async () => {
    setChecking(true);
    setUpToDate(false);
    const ok = await run(async () => {
      const ready = await api.checkForUpdates();
      setUpToDate(!ready);
      setInfo(await api.updateInfo());
    });
    if (!ok) setUpToDate(false);
    setChecking(false);
  };

  return (
    <Section icon={<AppWindow size={18} />} title="MultiTasker">
      <label className="toggle-row">
        <span>
          <span className="choice-title">Start with Windows</span>
          <span className="choice-detail">
            {info && !info.installed
              ? 'Only for installed copies; this development build never starts on its own.'
              : 'Opens quietly to the tray and the widget when you sign in.'}
          </span>
        </span>
        <input
          type="checkbox"
          role="switch"
          className="switch"
          checked={!!startup}
          disabled={startup === null || !info?.installed}
          onChange={(e) => {
            const on = e.target.checked;
            setStartup(on);
            void run(() => api.setStartWithWindows(on));
          }}
        />
      </label>
      <label className="toggle-row reminder-row">
        <span>
          <span className="choice-title">Quick switcher</span>
          <span className="choice-detail">
            A hotkey that works anywhere: find a task to switch to, start alongside or pause, or add a new one.
          </span>
          {shortcutProblem && <span className="choice-detail error-text">{shortcutProblem}</span>}
        </span>
        <select
          className="inline-select"
          value={shortcut ?? ''}
          disabled={shortcut === undefined}
          onChange={(e) => {
            const next = e.target.value || null;
            const before = shortcut;
            setShortcut(next);
            void run(() => api.setSwitcherShortcut(next)).then((ok) => {
              if (!ok) setShortcut(before);
              else setShortcutProblem(null);
            });
          }}
        >
          {SHORTCUTS.map((s) => (
            <option key={s ?? 'off'} value={s ?? ''}>
              {s ?? 'Off'}
            </option>
          ))}
        </select>
      </label>
      <div className="toggle-row reminder-row">
        <span>
          <span className="choice-title">Version {info?.current ?? '…'}</span>
          <span className="choice-detail">
            {info?.ready
              ? `Version ${info.ready} is downloaded. Restarting installs it.`
              : upToDate
                ? "You're up to date."
                : info?.private
                  ? "This private build doesn't update itself; install new versions by hand."
                  : info && !info.enabled
                    ? "Development builds don't update themselves."
                    : 'Updates download in the background, and install when you restart.'}
          </span>
        </span>
        {info?.ready ? (
          <button className="primary small" onClick={() => void run(api.installUpdate)}>
            <RefreshCw size={13} /> Restart to update
          </button>
        ) : (
          <button className="secondary small" disabled={checking || !info?.enabled} onClick={() => void check()}>
            {checking ? <LoaderCircle size={13} className="spin" /> : <RefreshCw size={13} />} Check for updates
          </button>
        )}
      </div>
    </Section>
  );
}

function WidgetSection({ run }: { run: Run }) {
  const [opacity, setOpacity] = useState<number | null>(null);
  const [onTop, setOnTop] = useState<OnTop | null>(null);
  const [picking, setPicking] = useState(false);

  useEffect(() => {
    void api.widgetOpacity().then(setOpacity, () => {});
    void api.widgetOnTop().then(setOnTop, () => {});
  }, []);

  const changeOnTop = (next: OnTop) => {
    setOnTop(next);
    void run(() => api.setWidgetOnTop(next));
  };

  // Dragging previews on the widget as it moves; letting go saves.
  const preview = (value: number) => {
    setOpacity(value);
    void api.setWidgetOpacity(value, false);
  };
  const save = () => opacity !== null && void run(() => api.setWidgetOpacity(opacity, true));

  return (
    <Section icon={<PictureInPicture2 size={18} />} title="Widget">
      <label className="toggle-row">
        <span>
          <span className="choice-title">Background opacity</span>
          <span className="choice-detail">Lower lets more of what's behind show through. Text stays solid.</span>
        </span>
        <span className="slider-field">
          <input
            type="range"
            min={30}
            max={100}
            step={1}
            value={opacity ?? 94}
            disabled={opacity === null}
            onChange={(e) => preview(Number(e.target.value))}
            onPointerUp={save}
            onKeyUp={save}
            aria-label="Widget background opacity"
          />
          <output>{opacity ?? 94}%</output>
        </span>
      </label>
      <label className="toggle-row reminder-row">
        <span>
          <span className="choice-title">Always on top</span>
          <span className="choice-detail">Keeps the widget above other windows, except where you say below.</span>
        </span>
        <input
          type="checkbox"
          role="switch"
          className="switch"
          checked={!!onTop?.enabled}
          disabled={!onTop}
          onChange={(e) => onTop && changeOnTop({ ...onTop, enabled: e.target.checked })}
        />
      </label>
      {onTop?.enabled && (
        <div className="on-top-exceptions">
          <span className="choice-title">Except</span>
          <label className="check-row">
            <input
              type="checkbox"
              checked={onTop.except_fullscreen}
              onChange={(e) => changeOnTop({ ...onTop, except_fullscreen: e.target.checked })}
            />
            When something is full screen, like a game, a presentation or a film
          </label>
          <ul className="exception-list">
            {onTop.exceptions.map((app) => (
              <li key={app.exe}>
                <span>
                  {app.name} <span className="muted">{app.exe}</span>
                </span>
                <button
                  className="icon-btn"
                  onClick={() =>
                    changeOnTop({ ...onTop, exceptions: onTop.exceptions.filter((e) => e.exe !== app.exe) })
                  }
                  aria-label={`Stop stepping aside for ${app.name}`}
                  title="Remove"
                >
                  <X size={14} />
                </button>
              </li>
            ))}
          </ul>
          <div className="exception-add">
            <button className="secondary small" onClick={() => setPicking((p) => !p)}>
              <Plus size={13} /> Add a program
            </button>
            {picking && (
              <AppPicker
                onClose={() => setPicking(false)}
                onPick={(app) => {
                  setPicking(false);
                  if (onTop.exceptions.some((e) => e.exe.toLowerCase() === app.exe.toLowerCase())) return;
                  changeOnTop({ ...onTop, exceptions: [...onTop.exceptions, { exe: app.exe, name: app.name }] });
                }}
              />
            )}
          </div>
        </div>
      )}
    </Section>
  );
}

const EARLY_OPTIONS = [null, 5, 10, 15, 30, 60];

function RemindersSection({ run }: { run: Run }) {
  const [prefs, setPrefs] = useState<ReminderSettings | null>(null);

  useEffect(() => {
    void api.reminderSettings().then(setPrefs, () => {});
  }, []);

  const change = (next: ReminderSettings) => {
    setPrefs(next);
    void run(() => api.setReminderSettings(next));
  };

  return (
    <Section icon={<BellRing size={18} />} title="Reminders">
      <p className="settings-lead">
        For tasks with a due time. They show as Windows notifications while MultiTasker is running.
      </p>
      <label className="toggle-row">
        <span>
          <span className="choice-title">Before it's due</span>
          <span className="choice-detail">A heads-up so there's time to wrap up.</span>
        </span>
        <select
          className="inline-select"
          value={prefs?.minutes_before ?? ''}
          disabled={!prefs}
          onChange={(e) =>
            prefs && change({ ...prefs, minutes_before: e.target.value ? Number(e.target.value) : null })
          }
        >
          {EARLY_OPTIONS.map((m) => (
            <option key={m ?? 'off'} value={m ?? ''}>
              {m === null ? 'Off' : m === 60 ? '1 hour before' : `${m} minutes before`}
            </option>
          ))}
        </select>
      </label>
      <label className="toggle-row reminder-row">
        <span>
          <span className="choice-title">When it's due</span>
          <span className="choice-detail">Another reminder at the due time itself.</span>
        </span>
        <input
          type="checkbox"
          role="switch"
          className="switch"
          checked={!!prefs?.at_due_time}
          disabled={!prefs}
          onChange={(e) => prefs && change({ ...prefs, at_due_time: e.target.checked })}
        />
      </label>
    </Section>
  );
}

const START_AFTER = [5, 15, 30, 60, 120, 300];

const AWAY_OPTIONS = [5, 10, 15, 30, 60, null];

function DetectionSection({ run }: { run: Run }) {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [away, setAway] = useState<number | null | undefined>(undefined);
  const [autoSwitch, setAutoSwitch] = useState<boolean | null>(null);
  const [startAfter, setStartAfter] = useState<number | null>(null);
  const [stillOn, setStillOn] = useState(stillOnHours);

  useEffect(() => {
    void api.pauseAfterAway().then(setAway, () => {});
    void api.autoSwitch().then(setAutoSwitch, () => {});
    void api.autoStartAfter().then(setStartAfter, () => {});
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
      <label className="toggle-row reminder-row">
        <span>
          <span className="choice-title">Auto-start tasks when a related file or process opens</span>
          <span className="choice-detail">
            Starts the task whose linked file, page or app you've stayed on, pausing what you were on, with Undo in the
            widget. Turn this off to be asked first.
          </span>
        </span>
        <input
          type="checkbox"
          role="switch"
          className="switch"
          checked={!!autoSwitch}
          disabled={autoSwitch === null || !enabled}
          onChange={(e) => {
            const on = e.target.checked;
            setAutoSwitch(on);
            void run(() => api.setAutoSwitch(on));
          }}
        />
      </label>
      {autoSwitch && enabled && (
        <label className="toggle-row reminder-row">
          <span>
            <span className="choice-title">How long before auto-starting</span>
            <span className="choice-detail">How long the related file or process has to stay open first.</span>
          </span>
          <select
            className="inline-select"
            value={startAfter ?? 30}
            disabled={startAfter === null}
            onChange={(e) => {
              const secs = Number(e.target.value);
              setStartAfter(secs);
              void run(() => api.setAutoStartAfter(secs));
            }}
          >
            {START_AFTER.map((s) => (
              <option key={s} value={s}>
                {s < 60 ? `${s} seconds` : s === 60 ? '1 minute' : `${s / 60} minutes`}
              </option>
            ))}
          </select>
        </label>
      )}
      <label className="toggle-row reminder-row">
        <span>
          <span className="choice-title">Ask if I'm still on a task</span>
          <span className="choice-detail">
            When something's been in progress here this long with none of its files, pages or apps in front.
          </span>
        </span>
        <select
          className="inline-select"
          value={stillOn}
          onChange={(e) => {
            const hours = Number(e.target.value);
            setStillOn(hours);
            try {
              localStorage.setItem(STILL_ON_KEY, String(hours));
            } catch {
              // Without storage the default stays.
            }
          }}
        >
          {STILL_ON_CHOICES.map((h) => (
            <option key={h} value={h}>
              {h === 0 ? 'Never' : h === 1 ? 'After 1 hour' : `After ${h} hours`}
            </option>
          ))}
        </select>
      </label>
      <label className="toggle-row reminder-row">
        <span>
          <span className="choice-title">Pause when I'm away</span>
          <span className="choice-detail">
            Pauses what's in progress on this computer after a while with no keyboard or mouse, or while it sleeps.
            Coming back offers to resume.
          </span>
        </span>
        <select
          className="inline-select"
          value={away ?? ''}
          disabled={away === undefined}
          onChange={(e) => {
            const minutes = e.target.value ? Number(e.target.value) : null;
            setAway(minutes);
            void run(() => api.setPauseAfterAway(minutes));
          }}
        >
          {AWAY_OPTIONS.map((m) => (
            <option key={m ?? 'never'} value={m ?? ''}>
              {m === null ? 'Never' : m === 60 ? 'After 1 hour' : `After ${m} minutes`}
            </option>
          ))}
        </select>
      </label>
    </Section>
  );
}

function S3Form(props: { current: S3Settings | null; onSave: (s: S3Settings, secret: string) => Promise<boolean> }) {
  const { current, onSave } = props;
  const initial = guessProvider(current?.provider, current?.endpoint);
  const [providerId, setProviderId] = useState(initial.id);
  const provider = providerById(providerId);
  const [param, setParam] = useState(
    () => (current?.endpoint && initial.param?.pattern.exec(current.endpoint)?.[1]) || '',
  );
  const [endpoint, setEndpoint] = useState(current?.endpoint ?? '');
  const [region, setRegion] = useState(current?.region ?? initial.region.initial);
  const [bucket, setBucket] = useState(current?.bucket ?? '');
  const [prefix, setPrefix] = useState(current?.prefix ?? '');
  const [keyId, setKeyId] = useState(current?.access_key_id ?? '');
  const [secret, setSecret] = useState('');
  const [saving, setSaving] = useState(false);

  const chooseProvider = (id: string) => {
    const next = providerById(id);
    setProviderId(id);
    setParam('');
    if (id !== 'custom') setEndpoint('');
    setRegion(current?.provider === id ? current.region : next.region.initial);
  };

  const builtEndpoint =
    provider.id === 'custom' ? endpoint.trim() || null : provider.endpoint(param.trim(), region.trim());
  const sameKey = current?.access_key_id === keyId.trim() && (current?.endpoint ?? null) === builtEndpoint;
  const ready =
    bucket.trim() &&
    region.trim() &&
    keyId.trim() &&
    (!provider.param || param.trim()) &&
    (provider.id !== 'custom' || endpoint.trim()) &&
    (secret || sameKey);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    await onSave(
      {
        provider: provider.id,
        endpoint: builtEndpoint,
        region: region.trim(),
        bucket: bucket.trim(),
        prefix: prefix.trim() || null,
        access_key_id: keyId.trim(),
      },
      secret,
    );
    setSaving(false);
    setSecret('');
  };

  return (
    <form className="s3-form" onSubmit={submit}>
      <label className="field wide">
        <span>Service</span>
        <select value={providerId} onChange={(e) => chooseProvider(e.target.value)}>
          {S3_PROVIDERS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
      </label>
      {provider.param && (
        <label className="field wide">
          <span>{provider.param.label}</span>
          <input value={param} onChange={(e) => setParam(e.target.value)} placeholder={provider.param.placeholder} />
        </label>
      )}
      {provider.id === 'custom' && (
        <label className="field wide">
          <span>Endpoint</span>
          <input
            value={endpoint}
            onChange={(e) => setEndpoint(e.target.value)}
            placeholder="e.g. http://nas.local:9000"
          />
        </label>
      )}
      <label className={`field${provider.region.fixed ? ' wide' : ''}`}>
        <span>Bucket</span>
        <input value={bucket} onChange={(e) => setBucket(e.target.value)} required />
      </label>
      {!provider.region.fixed && (
        <label className="field">
          <span>Region</span>
          <input value={region} onChange={(e) => setRegion(e.target.value)} placeholder={provider.region.placeholder} />
        </label>
      )}
      <label className="field wide">
        <span>Folder in bucket (optional)</span>
        <input value={prefix} onChange={(e) => setPrefix(e.target.value)} placeholder="e.g. multitasker-sync" />
      </label>
      <label className="field">
        <span>Access key ID</span>
        <input
          value={keyId}
          onChange={(e) => setKeyId(e.target.value)}
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
        <KeyRound size={13} /> {provider.keys} The secret is kept in Windows Credential Manager, never in MultiTasker's
        files.
      </p>
      {builtEndpoint && provider.id !== 'custom' && (!provider.param || param.trim()) && region.trim() && (
        <p className="settings-hint wide endpoint-preview">Endpoint: {builtEndpoint}</p>
      )}
      <div className="wide">
        <button type="submit" className="primary" disabled={!ready || saving}>
          {saving && <LoaderCircle size={14} className="spin" />}
          {current ? 'Save and reconnect' : 'Connect'}
        </button>
      </div>
    </form>
  );
}

const SERVICE = {
  google: { name: 'Google', storage: 'Google Drive', secret: true },
  microsoft: { name: 'Microsoft', storage: 'OneDrive', secret: false },
} as const;

function CloudSetup(props: {
  service: CloudService;
  signedInAs: string | null;
  run: Run;
  onDone: () => void;
  onSignOut: () => void;
  /** Asks before signing in moves sync somewhere else; resolves false to stop. */
  confirm: () => Promise<boolean>;
}) {
  const { service, signedInAs, run, onDone, onSignOut, confirm } = props;
  const info = SERVICE[service];
  const [builtIn, setBuiltIn] = useState<boolean | null>(null);
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [waiting, setWaiting] = useState(false);

  useEffect(() => {
    void api.builtInClients().then(
      (c) => setBuiltIn(c[service]),
      () => setBuiltIn(false),
    );
  }, [service]);

  const signIn = async () => {
    if (!(await confirm())) return;
    setWaiting(true);
    const ok = await run(() => api.signInCloud(service, clientId.trim(), clientSecret.trim()));
    setWaiting(false);
    if (ok) onDone();
  };

  const needsClient = builtIn === false && !signedInAs;
  const ready = !needsClient || (clientId.trim() && (!info.secret || clientSecret.trim()));

  return (
    <div className="cloud-setup">
      {signedInAs && !waiting && (
        <div className="folder-current">
          <Cloud size={16} className="kind kind-link" />
          <span className="folder-path account">Signed in as {signedInAs}</span>
          <button className="secondary" onClick={() => void signIn()}>
            Switch account
          </button>
          <button className="secondary" onClick={onSignOut}>
            <LogOut size={14} /> Sign out
          </button>
        </div>
      )}

      {needsClient && !waiting && (
        <div className="s3-form">
          <p className="settings-hint wide">
            This copy of MultiTasker wasn't built with a {info.name} app registration. Paste the client ID
            {info.secret ? ' and secret' : ''} from your own app registration.
          </p>
          <label className={`field${info.secret ? '' : ' wide'}`}>
            <span>Client ID</span>
            <input value={clientId} onChange={(e) => setClientId(e.target.value)} spellCheck={false} />
          </label>
          {info.secret && (
            <label className="field">
              <span>Client secret</span>
              <input type="password" value={clientSecret} onChange={(e) => setClientSecret(e.target.value)} />
            </label>
          )}
        </div>
      )}

      {waiting ? (
        <div className="sign-in-wait">
          <LoaderCircle size={16} className="spin" />
          <span>Finish signing in to {info.name} in your browser…</span>
          <button className="secondary" onClick={() => void api.cancelSignIn()}>
            Cancel
          </button>
        </div>
      ) : (
        !signedInAs && (
          <button className="primary sign-in" onClick={() => void signIn()} disabled={!ready || builtIn === null}>
            <LogIn size={15} /> Sign in with {info.name}
          </button>
        )
      )}

      <p className="settings-hint">
        MultiTasker only gets access to its own {info.storage} folder, never your other files. The sign-in is kept in
        Windows Credential Manager.
      </p>
    </div>
  );
}

function describeTarget(target: SyncTarget): string {
  switch (target.kind) {
    case 'folder':
      return 'the shared folder';
    case 's3':
      return `the ${target.bucket} bucket`;
    case 'google_drive':
      return `Google Drive (${target.account})`;
    case 'one_drive':
      return `OneDrive (${target.account})`;
    default:
      return 'the current location';
  }
}

const capitalize = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

function MoveDialog(props: {
  current: string;
  where: string | null;
  others: string[];
  onAnswer: (ok: boolean) => void;
}) {
  const { current, where, others, onAnswer } = props;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onAnswer(false);
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onAnswer]);

  const names = others.join(', ');
  return (
    <div className="dialog-scrim" onClick={() => onAnswer(false)}>
      <div
        className="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="move-title"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="move-title">{where ? `Move sync to ${where}?` : 'Turn off sync?'}</h3>
        <p>
          {where
            ? `Your tasks will be copied to ${where}, and this computer will sync there from now on.`
            : 'This computer will stop syncing. Your tasks stay here.'}{' '}
          {capitalize(current)} keeps its copy as it is now, but won't get this computer's changes.
        </p>
        {others.length > 0 && (
          <p className="settings-warning">
            <CircleAlert size={15} />
            <span>
              {where
                ? `${names} ${others.length === 1 ? 'still syncs' : 'still sync'} with ${current}. Move ${
                    others.length === 1 ? 'it' : 'them'
                  } to ${where} too, or the computers will drift apart.`
                : `${names} will keep syncing with ${current}, but changes made here won't reach ${
                    others.length === 1 ? 'it' : 'them'
                  }.`}
            </span>
          </p>
        )}
        <div className="dialog-actions">
          <button className="secondary" onClick={() => onAnswer(false)}>
            Cancel
          </button>
          <button className="primary" onClick={() => onAnswer(true)} autoFocus>
            {where ? 'Move sync' : 'Turn off sync'}
          </button>
        </div>
      </div>
    </div>
  );
}
