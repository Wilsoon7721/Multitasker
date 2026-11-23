import { Copy, PlugZap, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api, onRelayProblem, type RelayProblem } from '../api';

const PROGRAMS = ['multitasker.exe', 'multitasker-host.exe'];

/** The browser's relay started but got stuck, with the programs that had put their code into it. */
export function RelayProblemNote() {
  const [problem, setProblem] = useState<RelayProblem | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    const load = () => void api.relayProblem().then(setProblem, () => {});
    load();
    return onRelayProblem(load);
  }, []);

  if (!problem) return null;

  const windhawk = problem.suspects.some((s) => /windhawk/i.test(s.name));
  const copy = () =>
    void navigator.clipboard.writeText(PROGRAMS.join('\n')).then(
      () => setCopied(true),
      () => {},
    );

  return (
    <div className="notice-card" role="alert">
      <PlugZap size={18} className="notice-card-icon" aria-hidden />
      <div className="notice-card-text">
        <strong>The browser extension can't reach MultiTasker</strong>
        {problem.suspects.length > 0 ? (
          <>
            <p>
              Its relay started but got stuck before it could connect. These programs had loaded their own code into it,
              and one of them is the likely cause:
            </p>
            <ul className="notice-card-list">
              {problem.suspects.map((s) => (
                <li key={s.name}>
                  <b>{s.name}</b> <span className="notice-card-files">{s.files.join(', ')}</span>
                </li>
              ))}
            </ul>
            <p>
              {windhawk ? (
                <>
                  In Windhawk, open{' '}
                  <b>Settings → Advanced settings → More advanced settings → Process exclusion list</b> and add both
                  programs below. For anything else, look in its settings for programs to leave alone.
                </>
              ) : (
                'Look in their settings for a list of programs to leave alone, and add both programs below.'
              )}
            </p>
            <p className="notice-card-programs">
              {PROGRAMS.map((p) => (
                <code key={p}>{p}</code>
              ))}
              <button className="link-btn" onClick={copy}>
                <Copy size={12} /> {copied ? 'Copied' : 'Copy both'}
              </button>
            </p>
          </>
        ) : (
          <p>
            Its relay started but got stuck before it could connect, and nothing else had loaded into it. Restarting the
            browser usually sorts it out.
          </p>
        )}
        <p className="notice-card-status">
          {problem.gave_up
            ? 'It kept getting stuck, so MultiTasker stopped restarting it.'
            : problem.restarts > 0
              ? 'MultiTasker stopped the stuck relay; the browser starts a fresh one with your next click.'
              : null}
        </p>
      </div>
      <button className="icon-btn" onClick={() => void api.dismissRelayProblem()} aria-label="Dismiss" title="Dismiss">
        <X size={14} />
      </button>
    </div>
  );
}
