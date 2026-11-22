import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

export interface MenuItem {
  label: string;
  icon: ReactNode;
  onSelect: () => void;
  danger?: boolean;
  /** A shortcut or note shown on the right, e.g. "Ctrl+Space". */
  hint?: string;
}

/** A menu's entries; `null` draws a line between groups. */
export type MenuEntry = MenuItem | null;

interface Open {
  x: number;
  y: number;
  entries: MenuEntry[];
}

const EDGE = 6;

/**
 * A right-click menu: `show` opens it at the pointer (or under a button), `menu` draws it. `onOpenChange` lets a window
 * that hugs its content make room while it's open.
 */
export function useContextMenu(onOpenChange?: (height: number | null) => void) {
  const [open, setOpen] = useState<Open | null>(null);

  const close = useCallback(() => {
    setOpen(null);
    onOpenChange?.(null);
  }, [onOpenChange]);

  const show = useCallback(
    (e: { clientX: number; clientY: number; preventDefault?: () => void }, entries: MenuEntry[]) => {
      e.preventDefault?.();
      setOpen({ x: e.clientX, y: e.clientY, entries });
    },
    [],
  );

  /** Opens it under a button, as a "more" button does. */
  const showFor = useCallback((el: HTMLElement, entries: MenuEntry[]) => {
    const r = el.getBoundingClientRect();
    setOpen({ x: r.right, y: r.bottom + 4, entries });
  }, []);

  const menu = open ? (
    <Menu {...open} onClose={close} grows={!!onOpenChange} onMeasure={(h) => onOpenChange?.(h)} />
  ) : null;
  return { show, showFor, close, menu, isOpen: !!open };
}

function Menu({
  x,
  y,
  entries,
  onClose,
  grows,
  onMeasure,
}: Open & { onClose: () => void; grows: boolean; onMeasure: (height: number) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y, ready: false });
  const items = entries.filter((e): e is MenuItem => !!e);
  const [active, setActive] = useState(-1);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    onMeasure(y + height + EDGE);
    // Flips to the other side of the pointer near an edge, and never leaves the window.
    const left = x + width + EDGE > window.innerWidth ? Math.max(EDGE, x - width) : x;
    // A window that grows to fit the menu has room below the pointer; any other keeps it inside.
    const bottom = grows ? Math.max(window.innerHeight, y + height + EDGE) : window.innerHeight;
    const top = y + height + EDGE > bottom ? Math.max(EDGE, Math.min(y - height, bottom - height - EDGE)) : y;
    setPos({ left, top, ready: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [x, y]);

  // It can only take the keyboard once it's visible.
  useEffect(() => {
    if (pos.ready) ref.current?.focus({ preventScroll: true });
  }, [pos.ready]);

  useEffect(() => {
    const away = (e: Event) => !ref.current?.contains(e.target as Node) && onClose();
    const escape = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      onClose();
    };
    document.addEventListener('keydown', escape, true);
    const gone = () => onClose();
    document.addEventListener('pointerdown', away, true);
    document.addEventListener('contextmenu', away, true);
    window.addEventListener('blur', gone);
    document.addEventListener('scroll', gone, true);
    return () => {
      document.removeEventListener('keydown', escape, true);
      document.removeEventListener('pointerdown', away, true);
      document.removeEventListener('contextmenu', away, true);
      window.removeEventListener('blur', gone);
      document.removeEventListener('scroll', gone, true);
    };
  }, [onClose]);

  const choose = (item: MenuItem) => {
    onClose();
    item.onSelect();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onClose();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const step = e.key === 'ArrowDown' ? 1 : -1;
      setActive((i) => (i + step + items.length) % items.length);
    } else if ((e.key === 'Enter' || e.key === ' ') && items[active]) {
      e.preventDefault();
      choose(items[active]);
    }
  };

  let index = -1;
  return createPortal(
    <div
      ref={ref}
      className="context-menu"
      role="menu"
      tabIndex={-1}
      style={{ left: pos.left, top: pos.top, visibility: pos.ready ? 'visible' : 'hidden' }}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {entries.map((entry, i) => {
        if (!entry) return <div key={`line-${i}`} className="context-menu-line" role="separator" />;
        index += 1;
        const mine = index;
        return (
          <button
            key={entry.label}
            role="menuitem"
            className={`context-menu-item${entry.danger ? ' danger' : ''}${active === mine ? ' active' : ''}`}
            onClick={() => choose(entry)}
            onPointerEnter={() => setActive(mine)}
          >
            <span className="context-menu-icon">{entry.icon}</span>
            <span className="context-menu-label">{entry.label}</span>
            {entry.hint && <span className="context-menu-hint">{entry.hint}</span>}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}

/** Text boxes keep the browser's own menu (copy, paste); everywhere else it's replaced by the app's, or nothing. */
export function quietBrowserMenu() {
  document.addEventListener('contextmenu', (e) => {
    const el = e.target as HTMLElement | null;
    const editable = el?.closest('input, textarea, [contenteditable="true"], [contenteditable=""]');
    // Shift+right-click still opens it, for inspecting the page while developing.
    if (!editable && !e.shiftKey) e.preventDefault();
  });
}
