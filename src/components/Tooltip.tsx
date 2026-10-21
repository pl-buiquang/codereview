import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

const DELAY_MS = 120;

/** Fast hover tooltip, portaled to `body` so it isn't clipped by overflow
 *  containers. Positioned to the right of the wrapped element. */
export function Tooltip({
  content,
  children,
  className = "cr-tooltip-anchor",
  variant = "text",
  when,
}: {
  content: ReactNode;
  children: ReactNode;
  /** Class of the anchor wrapper (the element the tooltip is positioned against). */
  className?: string;
  /** "card" renders a bordered surface that can hold rich content. */
  variant?: "text" | "card";
  /** Checked on hover; the tooltip is skipped when it returns false. */
  when?: (anchor: HTMLElement) => boolean;
}) {
  const wrapRef = useRef<HTMLSpanElement>(null);
  const timer = useRef<number | undefined>(undefined);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const show = () => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      const el = wrapRef.current;
      if (!el || (when && !when(el))) return;
      const r = el.getBoundingClientRect();
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
      className={className}
      onMouseEnter={show}
      onMouseLeave={hide}
      onMouseDown={hide}
    >
      {children}
      {pos &&
        createPortal(
          <div role="tooltip" className={`cr-tooltip${variant === "card" ? " cr-tooltip-card" : ""}`} style={{ top: pos.top, left: pos.left }}>
            {content}
          </div>,
          document.body,
        )}
    </span>
  );
}
