import { Copy, ShieldAlert, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { api } from '../api';

const DONE_KEY = 'windhawk-notice';
const PROGRAMS = ['multitasker.exe', 'multitasker-host.exe'];

/** Windhawk loads its mods into every program unless told not to, which can stop the browser extension's relay. */
export function WindhawkNote() {
  const [show, setShow] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let done = false;
    try {
      done = localStorage.getItem(DONE_KEY) === 'done';
    } catch {
      // Without storage it asks again next time, which is harmless.
    }
    if (!done) void api.windhawkPresent().then(setShow, () => {});
  }, []);

  if (!show) return null;

  const finish = () => {
    try {
      localStorage.setItem(DONE_KEY, 'done');
    } catch {
      // Without storage it shows again next time.
    }
    setShow(false);
  };

  const copy = () =>
    void navigator.clipboard.writeText(PROGRAMS.join('\n')).then(
      () => setCopied(true),
      () => {},
    );

  return (
    <div className="notice-card" role="status">
      <ShieldAlert size={18} className="notice-card-icon" aria-hidden />
      <div className="notice-card-text">
        <strong>Windhawk is on this computer</strong>
        <p>
          Its mods load into every program, and that can stop the browser extension from reaching MultiTasker. In
          Windhawk, open <b>Settings → Advanced settings → More advanced settings → Process exclusion list</b> and add:
        </p>
        <p className="notice-card-programs">
          {PROGRAMS.map((p) => (
            <code key={p}>{p}</code>
          ))}
          <button className="link-btn" onClick={copy}>
            <Copy size={12} /> {copied ? 'Copied' : 'Copy both'}
          </button>
        </p>
        <div className="notice-card-actions">
          <button className="primary small" onClick={finish}>
            Done
          </button>
          <button className="secondary small" onClick={() => setShow(false)}>
            Not now
          </button>
        </div>
      </div>
      <button className="icon-btn" onClick={() => setShow(false)} aria-label="Not now" title="Not now">
        <X size={14} />
      </button>
    </div>
  );
}
