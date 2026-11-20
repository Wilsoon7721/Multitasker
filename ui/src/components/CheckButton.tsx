import { Circle, CircleCheck } from 'lucide-react';

/** A round "done" toggle: hollow while open, filled when finished. */
export function CheckButton({ done, title, onToggle }: { done: boolean; title: string; onToggle: () => void }) {
  return (
    <button
      className={`check${done ? ' done' : ''}`}
      onClick={(e) => {
        e.stopPropagation();
        onToggle();
      }}
      role="checkbox"
      aria-checked={done}
      aria-label={done ? `Reopen "${title}"` : `Mark "${title}" as done`}
      title={done ? 'Reopen' : 'Mark as done'}
    >
      {done ? <CircleCheck size={18} /> : <Circle size={18} />}
    </button>
  );
}
