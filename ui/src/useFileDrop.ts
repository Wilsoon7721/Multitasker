import { getCurrentWebview } from '@tauri-apps/api/webview';
import { useEffect, useRef } from 'react';

/** Desktop file drops onto `data-drop` zones; the zone is passed to `onDrop`, or null outside every zone. */
export function useFileDrop(onDrop: (paths: string[], zone: string | null) => void) {
  const handler = useRef(onDrop);
  handler.current = onDrop;

  useEffect(() => {
    let current: Element | null = null;
    const highlight = (el: Element | null) => {
      if (el === current) return;
      current?.classList.remove('drop-over');
      el?.classList.add('drop-over');
      current = el;
    };

    let stop: (() => void) | undefined;
    let cancelled = false;
    void getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (payload.type === 'leave') return highlight(null);
        const el = zoneAt(payload.position.x, payload.position.y);
        if (payload.type === 'drop') {
          highlight(null);
          if (payload.paths.length) handler.current(payload.paths, el?.getAttribute('data-drop') ?? null);
        } else {
          highlight(el);
        }
      })
      .then((u) => (cancelled ? u() : (stop = u)));

    return () => {
      cancelled = true;
      highlight(null);
      stop?.();
    };
  }, []);
}

/** Drop positions arrive in physical pixels; the DOM works in CSS pixels. */
function zoneAt(x: number, y: number): Element | null {
  const r = window.devicePixelRatio || 1;
  return document.elementFromPoint(x / r, y / r)?.closest('[data-drop]') ?? null;
}

/** "task:<id>" → "<id>". */
export const taskZone = (zone: string | null) => (zone?.startsWith('task:') ? zone.slice(5) : null);

export const pathItems = (paths: string[]) => paths.map((path) => ({ type: 'path' as const, path }));
