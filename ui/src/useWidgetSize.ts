import { getCurrentWindow, LogicalPosition, LogicalSize } from '@tauri-apps/api/window';
import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from 'react';

/** The card's margin on every side; the window includes it so the shadow isn't clipped. */
export const MARGIN = 8;
const STORAGE_KEY = 'widget-size';

/** How wide the card can be, and how tall it may grow before its contents scroll. */
export const LIMITS = { minWidth: 300, maxWidth: 560, minHeight: 180, maxHeight: 900 };
const DEFAULT = { width: 360, maxHeight: 560 };

export interface WidgetSize {
  width: number;
  maxHeight: number;
}

export type Edge = 'West' | 'South' | 'SouthWest';

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, Math.round(v)));

function load(): WidgetSize {
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null') as WidgetSize | null;
    if (saved && typeof saved.width === 'number' && typeof saved.maxHeight === 'number') {
      return {
        width: clamp(saved.width, LIMITS.minWidth, LIMITS.maxWidth),
        maxHeight: clamp(saved.maxHeight, LIMITS.minHeight, LIMITS.maxHeight),
      };
    }
  } catch {
    // Storage unavailable: the widget just starts at its default size.
  }
  return DEFAULT;
}

interface Drag {
  edge: Edge;
  x: number;
  y: number;
  start: WidgetSize;
}

/**
 * Keeps the window hugging the card, so its transparent edge never blocks clicks. Dragging an edge resizes only the
 * card inside a window opened to the largest size, because resizing a transparent webview window live repaints badly.
 */
export function useWidgetSize(header: RefObject<HTMLElement | null>, body: RefObject<HTMLElement | null>) {
  const [size, setSize] = useState<WidgetSize>(load);
  const sizeRef = useRef(size);
  sizeRef.current = size;
  const drag = useRef<Drag | null>(null);
  const [dragging, setDragging] = useState(false);
  // Room a menu needs below the window's top while it's open, which can be more than the card itself.
  const room = useRef(0);

  /** Sizes the window to the card, keeping its right edge where it is, since the widget is pinned top-right. */
  const fit = async () => {
    if (drag.current || !header.current || !body.current) return;
    const win = getCurrentWindow();
    const natural = header.current.offsetHeight + body.current.offsetHeight + 2 + MARGIN * 2;
    const { width, maxHeight } = sizeRef.current;
    const w = width + MARGIN * 2;
    const h = Math.max(Math.min(maxHeight + MARGIN * 2, natural), room.current);
    const scale = await win.scaleFactor();
    const [pos, outer] = await Promise.all([win.outerPosition(), win.outerSize()]);
    const right = (pos.x + outer.width) / scale;
    if (Math.abs(outer.width / scale - w) > 0.5) await win.setPosition(new LogicalPosition(right - w, pos.y / scale));
    await win.setSize(new LogicalSize(w, h));
  };

  useEffect(() => {
    const observer = new ResizeObserver(() => void fit());
    if (header.current) observer.observe(header.current);
    if (body.current) observer.observe(body.current);
    return () => observer.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!dragging) void fit();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [size, dragging]);

  const start = async (edge: Edge, e: ReactPointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const target = e.currentTarget;
    target.setPointerCapture(e.pointerId);
    // Dragging the bottom starts from the height showing now, so the edge follows the pointer from the first move.
    const card = target.closest<HTMLElement>('.widget-card');
    const showing = card ? Math.round(card.getBoundingClientRect().height) : sizeRef.current.maxHeight;
    const from = {
      ...sizeRef.current,
      maxHeight: Math.min(sizeRef.current.maxHeight, Math.max(LIMITS.minHeight, showing)),
    };
    drag.current = { edge, x: e.screenX, y: e.screenY, start: from };
    setDragging(true);
    // Room for the card to grow into: the largest size, still anchored at the top-right corner.
    const win = getCurrentWindow();
    const scale = await win.scaleFactor();
    const [pos, outer] = await Promise.all([win.outerPosition(), win.outerSize()]);
    const right = (pos.x + outer.width) / scale;
    const w = LIMITS.maxWidth + MARGIN * 2;
    await win.setPosition(new LogicalPosition(right - w, pos.y / scale));
    await win.setSize(new LogicalSize(w, LIMITS.maxHeight + MARGIN * 2));
  };

  const move = (e: ReactPointerEvent<HTMLElement>) => {
    const d = drag.current;
    if (!d) return;
    const width = d.edge === 'South' ? d.start.width : d.start.width + (d.x - e.screenX);
    const maxHeight = d.edge === 'West' ? d.start.maxHeight : d.start.maxHeight + (e.screenY - d.y);
    const next = {
      width: clamp(width, LIMITS.minWidth, LIMITS.maxWidth),
      maxHeight: clamp(maxHeight, LIMITS.minHeight, LIMITS.maxHeight),
    };
    // Kept at once, so letting go saves the last position even before the screen has caught up.
    sizeRef.current = next;
    setSize(next);
  };

  const end = () => {
    if (!drag.current) return;
    drag.current = null;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(sizeRef.current));
    } catch {
      // Without storage the size lasts until the widget restarts.
    }
    setDragging(false);
  };

  /** Makes room for a menu reaching `height` pixels down the window, or gives it back with `null`. */
  const reserve = (height: number | null) => {
    room.current = height ?? 0;
    void fit();
  };

  return { size, dragging, grip: { start, move, end }, reserve };
}
