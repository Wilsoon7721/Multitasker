import { Check, X } from 'lucide-react';
import { useState } from 'react';
import { api, type TaskView } from '../api';
import type { Run } from '../useTasks';

/** A paused task's time spent, which can be clicked to correct it when it was counted wrongly. */
export function TimeCorrection({ task, label, run }: { task: TaskView; label: string; run: Run }) {
  const [editing, setEditing] = useState(false);
  const [hours, setHours] = useState('');
  const [minutes, setMinutes] = useState('');

  const open = () => {
    setHours(String(Math.floor(task.worked_secs / 3600)));
    setMinutes(String(Math.round((task.worked_secs % 3600) / 60)));
    setEditing(true);
  };

  const total = (Number(hours) || 0) * 3600 + (Number(minutes) || 0) * 60;
  const valid = /^\d*$/.test(hours) && /^\d*$/.test(minutes) && (hours !== '' || minutes !== '');

  const save = () => {
    if (!valid) return;
    setEditing(false);
    if (total !== task.worked_secs) void run(() => api.correctWorked(task.id, total));
  };

  if (!editing) {
    return (
      <button className="time-spent" onClick={open} title="Correct the time spent, if it was counted wrongly">
        {label}
      </button>
    );
  }

  return (
    <form
      className="time-correction"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          setEditing(false);
        }
      }}
    >
      <input
        autoFocus
        inputMode="numeric"
        value={hours}
        onChange={(e) => setHours(e.target.value.trim())}
        aria-label="Hours spent"
      />
      h
      <input
        inputMode="numeric"
        value={minutes}
        onChange={(e) => setMinutes(e.target.value.trim())}
        aria-label="Minutes spent"
      />
      m
      <button type="submit" className="icon-btn" disabled={!valid} aria-label="Save the time spent" title="Save">
        <Check size={14} />
      </button>
      <button type="button" className="icon-btn" onClick={() => setEditing(false)} aria-label="Cancel" title="Cancel">
        <X size={14} />
      </button>
    </form>
  );
}
