import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { useEffect, useRef, useState, type RefObject } from 'react';

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

export type Edge = 'West' | 'South' | 'SouthWest';

/** Keeps the window hugging the card, so its transparent edge never blocks clicks, within the size dragged to. */
export function useWidgetSize(header: RefObject<HTMLElement | null>, body: RefObject<HTMLElement | null>) {
  const [size, setSize] = useState<WidgetSize>(load);
  const sizeRef = useRef(size);
  sizeRef.current = size;
  // While an edge is being dragged, the window's size is the person's, not the content's.
  const resizing = useRef<Edge | null>(null);
  const settle = useRef<ReturnType<typeof setTimeout>>(undefined);

  const fit = () => {
    if (resizing.current !== null || !header.current || !body.current) return;
    const natural = header.current.offsetHeight + body.current.offsetHeight + 2 + MARGIN * 2;
    const { width, maxHeight } = sizeRef.current;
    void getCurrentWindow().setSize(new LogicalSize(width + MARGIN * 2, Math.min(maxHeight + MARGIN * 2, natural)));
  };

  useEffect(() => {
    const observer = new ResizeObserver(fit);
    if (header.current) observer.observe(header.current);
    if (body.current) observer.observe(body.current);
    return () => observer.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(fit, [size]);

  useEffect(() => {
    const win = getCurrentWindow();
    const stop = win.onResized(async ({ payload }) => {
      const edge = resizing.current;
      if (edge === null) return;
      const scale = await win.scaleFactor();
      // Only the edge being dragged changes its setting, so widening never caps the height at what's showing now.
      const next = {
        width:
          edge === 'South'
            ? sizeRef.current.width
            : clamp(payload.width / scale - MARGIN * 2, LIMITS.minWidth, LIMITS.maxWidth),
        maxHeight:
          edge === 'West'
            ? sizeRef.current.maxHeight
            : clamp(payload.height / scale - MARGIN * 2, LIMITS.minHeight, LIMITS.maxHeight),
      };
      clearTimeout(settle.current);
      // The drag has ended once the size stops changing; then the window hugs the content again.
      settle.current = setTimeout(() => {
        resizing.current = null;
        try {
          localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
        } catch {
          // Without storage the size lasts until the widget restarts.
        }
        setSize(next);
      }, 400);
    });
    return () => void stop.then((unlisten) => unlisten());
  }, []);

  const startResize = (edge: Edge) => {
    resizing.current = edge;
    // A press without a drag sends no resize at all, so the widget stops waiting for one after a moment.
    clearTimeout(settle.current);
    settle.current = setTimeout(() => {
      resizing.current = null;
      fit();
    }, 1500);
    void getCurrentWindow().startResizeDragging(edge);
  };

  return { size, startResize };
}
