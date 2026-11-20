import { useLayoutEffect, useRef, type TextareaHTMLAttributes } from 'react';

/** A textarea that grows with its content instead of scrolling, up to `maxRows`. */
export function AutoTextarea({
  minRows = 1,
  maxRows = 12,
  value,
  ...props
}: TextareaHTMLAttributes<HTMLTextAreaElement> & { minRows?: number; maxRows?: number; value: string }) {
  const ref = useRef<HTMLTextAreaElement>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const line = parseFloat(getComputedStyle(el).lineHeight) || 20;
    const pad = el.offsetHeight - el.clientHeight + parseFloat(getComputedStyle(el).paddingTop) * 2;
    el.style.height = 'auto';
    el.style.height = `${Math.min(Math.max(el.scrollHeight, line * minRows + pad), line * maxRows + pad)}px`;
  }, [value, minRows, maxRows]);

  return <textarea ref={ref} rows={minRows} value={value} {...props} />;
}
