import { ChevronDown, ExternalLink, Play, Square, SquareTerminal } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { api, onProjectRuns, type ProjectCommand, type RunView, type TaskView } from '../api';
import type { Run } from '../useTasks';

/** How often running commands are read again, for their output. */
const REFRESH_MS = 1500;
/** Commands shown before "More". */
const SHOWN = 4;

/** Dev servers, builds and tests a task's linked folders can run, started and stopped from here. */
export function ProjectSection({ task, run }: { task: TaskView; run: Run }) {
  const [commands, setCommands] = useState<ProjectCommand[]>([]);
  const [runs, setRuns] = useState<RunView[]>([]);
  const [autoruns, setAutoruns] = useState<string[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const [more, setMore] = useState(false);
  const folders = task.attachments.filter((a) => a.kind === 'folder').length;

  useEffect(() => {
    if (!folders) return setCommands([]);
    void api.projectCommands(task.id).then(setCommands, () => setCommands([]));
    void api.projectAutoruns(task.id).then(setAutoruns, () => {});
  }, [task.id, folders]);

  const anyRunning = runs.some((r) => r.running && r.task_id === task.id);
  useEffect(() => {
    const load = () => void api.projectRuns().then(setRuns, () => {});
    load();
    const stop = onProjectRuns(load);
    const timer = anyRunning ? setInterval(load, REFRESH_MS) : undefined;
    return () => {
      stop();
      clearInterval(timer);
    };
  }, [anyRunning]);

  const byProject = useMemo(() => {
    const groups = new Map<string, ProjectCommand[]>();
    for (const c of commands) groups.set(c.folder, [...(groups.get(c.folder) ?? []), c]);
    return [...groups.values()];
  }, [commands]);

  if (!commands.length) return null;

  const toggleAutorun = (c: ProjectCommand, on: boolean) => {
    setAutoruns((cur) => (on ? [...cur, c.id] : cur.filter((id) => id !== c.id)));
    void run(() => api.setProjectAutorun(task.id, c.id, on));
  };

  return (
    <section className="attachments project-section">
      <header className="attachments-header">
        <h3>Project</h3>
      </header>
      {byProject.map((group) => {
        const shown = more ? group : group.slice(0, SHOWN);
        return (
          <div key={group[0].folder} className="project-group">
            {byProject.length > 1 && <span className="project-name">{group[0].project}</span>}
            <ul className="project-commands">
              {shown.map((c) => {
                const r = runs.find((x) => x.command_id === c.id);
                const running = !!r?.running;
                return (
                  <li key={c.id} className={`project-command${running ? ' running' : ''}`}>
                    <div className="project-row">
                      <span className={`dot${running ? ' on' : ''}`} />
                      <span className="project-text">
                        <span className="project-label">{c.label}</span>
                        <code className="project-line">{c.command}</code>
                      </span>
                      {r?.url && running && (
                        <button
                          className="link-button"
                          onClick={() => void run(() => api.openProjectUrl(c.id))}
                          title="Open in the browser"
                        >
                          {r.url.replace(/^https?:\/\//, '')} <ExternalLink size={11} />
                        </button>
                      )}
                      {r && !running && r.exit !== null && r.exit !== -1 && (
                        <span className={`project-exit${r.exit === 0 ? '' : ' failed'}`}>
                          {r.exit === 0 ? 'Finished' : `Failed (${r.exit})`}
                        </span>
                      )}
                      <label
                        className="project-with-task"
                        title="Start it when the task starts, and stop it when the task pauses"
                      >
                        <input
                          type="checkbox"
                          checked={autoruns.includes(c.id)}
                          onChange={(e) => toggleAutorun(c, e.target.checked)}
                        />
                        With task
                      </label>
                      {r && (
                        <button
                          className={`icon-btn${open === c.id ? ' on' : ''}`}
                          onClick={() => setOpen(open === c.id ? null : c.id)}
                          aria-label={`Output of ${c.command}`}
                          title="Output"
                        >
                          <SquareTerminal size={14} />
                        </button>
                      )}
                      {running ? (
                        <button
                          className="secondary small"
                          onClick={() => void run(() => api.stopProjectCommand(c.id))}
                        >
                          <Square size={11} /> Stop
                        </button>
                      ) : (
                        <button
                          className="secondary small"
                          onClick={() => {
                            setOpen(c.id);
                            void run(() => api.runProjectCommand(task.id, c.id));
                          }}
                        >
                          <Play size={11} /> Run
                        </button>
                      )}
                    </div>
                    {open === c.id && r && (
                      <pre className="project-output">{r.output.slice(-60).join('\n') || 'No output yet.'}</pre>
                    )}
                  </li>
                );
              })}
            </ul>
            {group.length > SHOWN && (
              <button className="link-button" onClick={() => setMore((m) => !m)}>
                <ChevronDown size={12} /> {more ? 'Fewer' : `${group.length - SHOWN} more`}
              </button>
            )}
          </div>
        );
      })}
    </section>
  );
}
