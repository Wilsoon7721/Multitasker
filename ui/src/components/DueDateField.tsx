import { CalendarDays } from 'lucide-react';
import { useRef } from 'react';

/** A date field that says "Due date" while empty, where the browser would only show "dd/mm/yyyy". */
export function DueDateField({ value, onChange }: { value: string; onChange: (date: string) => void }) {
  const input = useRef<HTMLInputElement>(null);

  // Clicking the label opens the calendar straight away, rather than just focusing the day part of the date.
  const openPicker = () => {
    try {
      input.current?.showPicker();
    } catch {
      input.current?.focus();
    }
  };

  return (
    <label className={`field-pill due-field${value ? '' : ' empty'}`} title="Due date">
      <CalendarDays size={14} />
      {!value && (
        <span className="due-placeholder" onClick={openPicker} aria-hidden>
          Due date
        </span>
      )}
      <input ref={input} type="date" value={value} onChange={(e) => onChange(e.target.value)} aria-label="Due date" />
    </label>
  );
}
