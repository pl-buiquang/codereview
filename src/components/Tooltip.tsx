import { useEffect, useRef, useState, type ReactElement } from "react";
import { createPortal } from "react-dom";

const DELAY_MS = 120;

/** Fast hover tooltip, portaled to `body` so it isn't clipped by overflow
 *  containers. Positioned to the right of the wrapped element. */
export function Tooltip({ text, children }: { text: string; children: ReactElement }) {
  const wrapRef = useRef<HTMLSpanElement>(null);
  const timer = useRef<number | undefined>(undefined);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const show = () => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      const r = wrapRef.current?.getBoundingClientRect();
      if (r) setPos({ top: r.top + r.height / 2, left: r.right + 8 });
    }, DELAY_MS);
  };
  const hide = () => {
    window.clearTimeout(timer.current);
    setPos(null);
  };

  return (
    <span
      ref={wrapRef}
      className="cr-tooltip-anchor"
      onMouseEnter={show}
      onMouseLeave={hide}
      onMouseDown={hide}
    >
      {children}
      {pos &&
        createPortal(
          <div role="tooltip" className="cr-tooltip" style={{ top: pos.top, left: pos.left }}>
            {text}
          </div>,
          document.body,
        )}
    </span>
  );
}
