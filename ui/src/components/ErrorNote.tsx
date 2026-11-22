import { RotateCw, X } from 'lucide-react';
import { isTransient } from '../useTasks';

/** An error with a way to try again: changes that failed are made again, and the list is read again. */
export function ErrorNote(props: {
  error: string;
  unsaved: number;
  retrying: boolean;
  onRetry: () => void;
  onDismiss?: () => void;
  className: string;
}) {
  const { error, unsaved, retrying, onRetry, onDismiss, className } = props;
  const detail = unsaved
    ? `${unsaved} ${unsaved === 1 ? 'change' : 'changes'} not saved yet`
    : isTransient(error)
      ? 'Trying again on its own'
      : null;

  return (
    <div className={`error-note ${className}`} role="alert">
      <span className="error-note-text">
        {error}
        {detail && <span className="error-note-detail">{detail}</span>}
      </span>
      <button
        className="icon-btn error-note-retry"
        onClick={onRetry}
        disabled={retrying}
        aria-label={unsaved ? 'Retry the changes that failed' : 'Retry'}
        title={unsaved ? 'Make the changes that failed again' : 'Try again now'}
      >
        <RotateCw size={14} className={retrying ? 'spin' : undefined} />
      </button>
      {onDismiss && (
        <button
          className="icon-btn"
          onClick={onDismiss}
          aria-label="Dismiss"
          title={unsaved ? 'Dismiss and drop the changes that failed' : 'Dismiss'}
        >
          <X size={14} />
        </button>
      )}
    </div>
  );
}
