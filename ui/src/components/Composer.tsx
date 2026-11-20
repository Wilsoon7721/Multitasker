import { CalendarDays, CornerDownLeft, Plus, Tag, X } from 'lucide-react';
import { useRef, useState, type Dispatch, type FormEvent, type KeyboardEvent, type SetStateAction } from 'react';
import { api, type NewAttachment } from '../api';
import type { Run } from '../useTasks';
import { AttachButtons, KindIcon, pendingLabel } from './Attachments';
import { AutoTextarea } from './AutoTextarea';

/** The add-task card; files dropped on it wait in `pending` until the task is created. */
export function Composer(props: {
  run: Run;
  categories: string[];
  pending: NewAttachment[];
  setPending: Dispatch<SetStateAction<NewAttachment[]>>;
}) {
  const { run, categories, pending, setPending } = props;
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [category, setCategory] = useState('');
  const [due, setDue] = useState('');
  const [focused, setFocused] = useState(false);
  const form = useRef<HTMLFormElement>(null);

  // The category sticks between tasks (handy for adding several), so it alone doesn't keep the card open.
  const open = focused || !!(title || description || due || pending.length);

  const reset = () => {
    setTitle('');
    setDescription('');
    setDue('');
    setPending([]);
  };

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!title.trim()) return;
    const ok = await run(async () => {
      const task = await api.create({ title, description: description || null, category: category || null, due_date: due || null });
      reset();
      if (pending.length) await api.addAttachments(task.id, pending);
    });
    if (ok) form.current?.querySelector<HTMLInputElement>('.composer-title input')?.focus();
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

  const addPending = (items: NewAttachment[]) =>
    setPending((cur) => [...cur, ...items.filter((i) => !cur.some((c) => JSON.stringify(c) === JSON.stringify(i)))]);

  return (
    <form
      ref={form}
      className={`composer${open ? ' open' : ''}`}
      data-drop="composer"
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
            placeholder="Details, notes… (optional)"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            minRows={2}
            aria-label="Details"
          />

          {pending.length > 0 && (
            <ul className="pending">
              {pending.map((item, i) => {
                const { kind, name } = pendingLabel(item);
                return (
                  <li key={i} className="chip">
                    <KindIcon kind={kind} size={13} />
                    <span>{name}</span>
                    <button
                      type="button"
                      onClick={() => setPending((cur) => cur.filter((_, j) => j !== i))}
                      aria-label={`Remove ${name}`}
                    >
                      <X size={12} />
                    </button>
                  </li>
                );
              })}
            </ul>
          )}

          <div className="composer-bar">
            <AttachButtons onAdd={addPending} run={run} />
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
