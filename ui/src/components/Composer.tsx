import { CalendarDays, CornerDownLeft, Plus, Tag } from 'lucide-react';
import { useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import { api } from '../api';
import type { Run } from '../useTasks';
import { AutoTextarea } from './AutoTextarea';

/** The add-task card: a title line that opens up into details, category and due date while you're using it. */
export function Composer({ run, categories }: { run: Run; categories: string[] }) {
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [category, setCategory] = useState('');
  const [due, setDue] = useState('');
  const [focused, setFocused] = useState(false);
  const form = useRef<HTMLFormElement>(null);

  // The category sticks between tasks (handy for adding several), so it alone doesn't keep the card open.
  const open = focused || !!(title || description || due);

  const reset = () => {
    setTitle('');
    setDescription('');
    setDue('');
  };

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!title.trim()) return;
    const ok = await run(() =>
      api.create({ title, description: description || null, category: category || null, due_date: due || null }),
    );
    if (ok) reset();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void submit();
    } else if (e.key === 'Escape') {
      reset();
      setCategory('');
      (document.activeElement as HTMLElement | null)?.blur();
    }
  };

  return (
    <form
      ref={form}
      className={`composer${open ? ' open' : ''}`}
      onSubmit={submit}
      onKeyDown={onKeyDown}
      onFocus={() => setFocused(true)}
      onBlur={(e) => {
        if (!form.current?.contains(e.relatedTarget as Node | null)) setFocused(false);
      }}
    >
      <div className="composer-title">
        <Plus size={18} className="composer-plus" />
        <input
          placeholder="Add a task…"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.ctrlKey) {
              e.preventDefault();
              void submit();
            }
          }}
          maxLength={200}
          aria-label="Task title"
        />
      </div>

      {open && (
        <>
          <AutoTextarea
            className="composer-details"
            placeholder="Details, notes, links… (optional)"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            minRows={2}
            aria-label="Details"
          />

          <div className="composer-bar">
            <label className="field-pill" title="Category">
              <Tag size={14} />
              <input
                placeholder="Category"
                list="categories"
                value={category}
                onChange={(e) => setCategory(e.target.value)}
                maxLength={60}
                aria-label="Category"
              />
            </label>
            <label className={`field-pill${due ? '' : ' empty'}`} title="Due date">
              <CalendarDays size={14} />
              <input type="date" value={due} onChange={(e) => setDue(e.target.value)} aria-label="Due date" />
            </label>

            <span className="spacer" />
            <span className="hint">
              Ctrl <CornerDownLeft size={11} />
            </span>
            <button type="submit" className="primary" disabled={!title.trim()}>
              Add task
            </button>
          </div>
        </>
      )}

      <datalist id="categories">
        {categories.map((c) => (
          <option key={c} value={c} />
        ))}
      </datalist>
    </form>
  );
}
