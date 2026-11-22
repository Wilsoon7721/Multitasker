import { FolderOpen, Link2, NotebookPen, Undo2, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { api, type ActivitySnapshot, type TaskView } from '../api';
import type { Run } from '../useTasks';

/** How long a prompt nobody touched stays up. */
const HIDE_AFTER_MS = 30_000;

/** Calls `onHide` once the prompt has sat untouched for a while. */
function useHideWhenIgnored(onHide: () => void, touched: boolean) {
  const hide = useRef(onHide);
  hide.current = onHide;
  useEffect(() => {
    if (touched) return;
    const timer = setTimeout(() => hide.current(), HIDE_AFTER_MS);
    return () => clearTimeout(timer);
  }, [touched]);
}

/** Asks where work on a just-paused task stopped, so picking it up later is quick. */
export function LeftOffPrompt(props: { task: { id: string; title: string }; run: Run; onClose: () => void }) {
  const { task, run, onClose } = props;
  const [text, setText] = useState('');
  const [focused, setFocused] = useState(false);
  useHideWhenIgnored(onClose, focused || text.length > 0);

  const save = () => {
    if (text.trim()) void run(() => api.setLeftOff(task.id, text));
    onClose();
  };

  return (
    <form
      className="suggestion left-off-prompt"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <NotebookPen size={14} className="away-icon" aria-hidden />
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onKeyDown={(e) => e.key === 'Escape' && onClose()}
        placeholder={`Leave a note for where you stopped "${task.title}" at?`}
        aria-label={`Where you left off “${task.title}”`}
        maxLength={500}
      />
      <button type="button" className="icon-btn" onClick={onClose} aria-label="Skip" title="Skip">
        <X size={14} />
      </button>
    </form>
  );
}

/** Things a task can reopen on this computer: its pages, apps, and the files and folders attached here. */
export function reopenable(task: TaskView, deviceId: string | null): number {
  return task.attachments.filter((a) => a.kind === 'link' || a.kind === 'app' || a.device === deviceId).length;
}

/** Back on a task: shows where it was left and offers to open its things again. */
export function PickUpBar(props: {
  task: TaskView;
  deviceId: string | null;
  /** Only show where it was left, not the offer to open its things. */
  noteOnly?: boolean;
  run: Run;
  onClose: () => void;
}) {
  const { task, deviceId, noteOnly, run, onClose } = props;
  const [hovered, setHovered] = useState(false);
  useHideWhenIgnored(onClose, hovered);
  const count = noteOnly ? 0 : reopenable(task, deviceId);
  const note = task.left_off?.text;
  if (!note && !count) return null;

  return (
    <div
      className="suggestion pick-up"
      role="status"
      onPointerEnter={() => setHovered(true)}
      onPointerLeave={() => setHovered(false)}
    >
      <Undo2 size={14} className="away-icon" aria-hidden />
      <div className="suggestion-text">
        <span className="suggestion-title" title={task.title}>
          Back to “{task.title}”
        </span>
        <span className="suggestion-reason" title={note}>
          {note ? `Left off: ${note}` : 'Pick up where you were'}
        </span>
      </div>
      {count > 0 && (
        <button
          className="note-action"
          onClick={() => {
            onClose();
            void run(async () => {
              const r = await api.openTaskThings(task.id);
              if (r.skipped.length) throw r.skipped.join(' ');
            });
          }}
          aria-label={`Open its ${count} linked ${count === 1 ? 'item' : 'items'}`}
          title={`Open its ${count} linked ${count === 1 ? 'item' : 'items'}`}
        >
          <FolderOpen size={14} />
        </button>
      )}
      <button className="icon-btn" onClick={onClose} aria-label="Close" title="Close">
        <X size={14} />
      </button>
    </div>
  );
}

/** Offers to link a page that's been in front a while to the task in progress. */
export function LinkOfferNote(props: { offer: NonNullable<ActivitySnapshot['link_offer']>; run: Run }) {
  const { offer, run } = props;
  const page = offer.title || offer.url.replace(/^https?:\/\/(www\.)?/, '');
  return (
    <div className="suggestion link-offer" role="status">
      <Link2 size={14} className="away-icon" aria-hidden />
      <div className="suggestion-text">
        <span className="suggestion-title" title={page}>
          Link “{page}”?
        </span>
        <span className="suggestion-reason" title={offer.task_title}>
          To “{offer.task_title}”, so it counts as working on it
        </span>
      </div>
      <button
        className="note-action"
        onClick={() => void run(api.acceptLinkOffer)}
        aria-label="Link this page"
        title="Link this page"
      >
        <Link2 size={14} />
      </button>
      <button
        className="icon-btn"
        onClick={() => void run(api.refuseLinkOffer)}
        aria-label="Don't link"
        title="Don't link this page"
      >
        <X size={14} />
      </button>
    </div>
  );
}
