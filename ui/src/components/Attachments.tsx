import { File, FileImage, FileText, FileVideo, Folder, FolderOpen, Globe, Link2, Paperclip, X } from 'lucide-react';
import { useState, type FormEvent } from 'react';
import { api, type Attachment, type AttachmentKind, type NewAttachment } from '../api';
import type { Run } from '../useTasks';
import { pathItems } from '../useFileDrop';

const ICONS: Record<AttachmentKind, typeof File> = {
  document: FileText,
  pdf: FileText,
  image: FileImage,
  video: FileVideo,
  folder: Folder,
  file: File,
  link: Globe,
};

export function KindIcon({ kind, size = 16 }: { kind: AttachmentKind; size?: number }) {
  const Icon = ICONS[kind];
  return <Icon size={size} className={`kind kind-${kind}`} />;
}

/** A rough, display-only guess at a file's kind; the backend decides for real once it's attached. */
const GUESSES: [RegExp, AttachmentKind][] = [
  [/\.pdf$/i, 'pdf'],
  [/\.(docx?|odt|rtf|txt|md|xlsx?|ods|csv|pptx?|odp)$/i, 'document'],
  [/\.(png|jpe?g|gif|webp|bmp|svg|heic|avif|tiff?)$/i, 'image'],
  [/\.(mp4|mov|mkv|avi|webm|wmv|m4v)$/i, 'video'],
];

/** The name shown for a not-yet-saved attachment in the composer. */
export function pendingLabel(item: NewAttachment): { kind: AttachmentKind; name: string } {
  if (item.type === 'link') return { kind: 'link', name: item.name || item.url.replace(/^https?:\/\/(www\.)?/, '') };
  const name = item.path.split(/[\\/]/).filter(Boolean).pop() ?? item.path;
  return { kind: GUESSES.find(([re]) => re.test(name))?.[1] ?? 'file', name };
}

/** Folder a file sits in, shortened from the left: "…\School\GS\notes.pdf" → "…\School\GS". */
function where(a: Attachment, deviceId: string | null): string {
  if (a.kind === 'link') return a.target.replace(/^https?:\/\//, '');
  if (deviceId && a.device && a.device !== deviceId) return 'On another computer';
  const parts = a.target.split(/[\\/]/);
  parts.pop();
  const dir = parts.join('\\');
  return dir.length > 48 ? `…${dir.slice(-47)}` : dir;
}

/** The attachments section of the detail panel. */
export function AttachmentList(props: { taskId: string; attachments: Attachment[]; run: Run; deviceId: string | null }) {
  const { taskId, attachments, run, deviceId } = props;

  return (
    <section className="attachments" data-drop={`task:${taskId}`}>
      <header className="attachments-header">
        <h3>
          Attachments {attachments.length > 0 && <span className="count">{attachments.length}</span>}
        </h3>
        <AttachButtons onAdd={(items) => run(() => api.addAttachments(taskId, items))} run={run} />
      </header>

      {attachments.length === 0 ? (
        <p className="attachments-empty">Drop files or folders here, or attach a link.</p>
      ) : (
        <ul className="attachment-list">
          {attachments.map((a) => (
            <li key={a.id} className="attachment">
              <button className="attachment-open" onClick={() => void run(() => api.openAttachment(a.id))} title={`Open ${a.target}`}>
                <KindIcon kind={a.kind} size={18} />
                <span className="attachment-text">
                  <span className="attachment-name">{a.name}</span>
                  <span className="attachment-where">{where(a, deviceId)}</span>
                </span>
              </button>
              {a.kind !== 'link' && (
                <button
                  className="icon-btn"
                  onClick={() => void run(() => api.revealAttachment(a.id))}
                  aria-label={`Show ${a.name} in its folder`}
                  title="Show in folder"
                >
                  <FolderOpen size={15} />
                </button>
              )}
              <button
                className="icon-btn"
                onClick={() => void run(() => api.removeAttachment(a.id))}
                aria-label={`Remove ${a.name}`}
                title="Remove (the file itself is kept)"
              >
                <X size={15} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** "Files" and "Link" buttons; the link one turns into an inline field. */
export function AttachButtons({ onAdd, run, compact }: { onAdd: (items: NewAttachment[]) => unknown; run: Run; compact?: boolean }) {
  const [linking, setLinking] = useState(false);
  const [url, setUrl] = useState('');

  const pick = () =>
    void run(async () => {
      const paths = await api.pickFiles();
      if (paths.length) await onAdd(pathItems(paths));
    });

  const addLink = async (e: FormEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (!url.trim()) return;
    await onAdd([{ type: 'link', url: url.trim() }]);
    setUrl('');
    setLinking(false);
  };

  if (linking) {
    return (
      <span className="link-field">
        <Link2 size={14} />
        <input
          autoFocus
          placeholder="Paste a link"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void addLink(e);
            if (e.key === 'Escape') {
              e.stopPropagation();
              setLinking(false);
            }
          }}
          onBlur={() => !url && setLinking(false)}
          aria-label="Link to attach"
        />
        <button type="button" className="icon-btn" onClick={(e) => void addLink(e)} aria-label="Add link" disabled={!url.trim()}>
          <Paperclip size={14} />
        </button>
      </span>
    );
  }

  return (
    <span className="attach-buttons">
      <button type="button" className="ghost-pill" onClick={pick} title="Attach files">
        <Paperclip size={14} /> {compact ? null : 'Files'}
      </button>
      <button type="button" className="ghost-pill" onClick={() => setLinking(true)} title="Attach a link">
        <Link2 size={14} /> {compact ? null : 'Link'}
      </button>
    </span>
  );
}
