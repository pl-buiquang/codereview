import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { TOOLTIP_DELAY_MS } from "./Tooltip";

const STASH = "data-cr-title";
const GAP = 6;
const MARGIN = 4;

/** Replaces native `title` tooltips app-wide with a fast styled one. On hover
 *  the anchor's `title` is moved to `data-cr-title` (suppressing the slow OS
 *  tooltip) and restored on leave, so the DOM stays as React rendered it.
 *  Opt out with `data-native-title`; `data-cr-title-when="clipped"` only shows
 *  the tooltip when the anchor's text is truncated. */
export function TooltipLayer() {
  const [tip, setTip] = useState<{ text: string; rect: DOMRect } | null>(null);

  useEffect(() => {
    let anchor: HTMLElement | null = null;
    let timer: number | undefined;

    const release = () => {
      window.clearTimeout(timer);
      if (anchor) {
        const text = anchor.getAttribute(STASH);
        anchor.removeAttribute(STASH);
        // React may have re-set `title` while hovered; don't clobber it.
        if (text != null && !anchor.hasAttribute("title")) anchor.setAttribute("title", text);
        anchor = null;
      }
      setTip(null);
    };
    const hideTip = () => {
      window.clearTimeout(timer);
      setTip(null);
    };

    const onOver = (e: PointerEvent) => {
      const target = e.target as Element | null;
      if (anchor?.contains(target)) return;
      release();
      const el = target?.closest?.<HTMLElement>("[title]");
      if (!el || el.hasAttribute("data-native-title")) return;
      const text = el.getAttribute("title");
      if (!text) return;
      anchor = el;
      el.setAttribute(STASH, text);
      el.removeAttribute("title");
      timer = window.setTimeout(() => {
        if (anchor !== el || !el.isConnected) return;
        if (el.dataset.crTitleWhen === "clipped" && el.scrollWidth <= el.clientWidth) return;
        setTip({ text, rect: el.getBoundingClientRect() });
      }, TOOLTIP_DELAY_MS);
    };
    const onOut = (e: PointerEvent) => {
      if (anchor && !anchor.contains(e.relatedTarget as Node | null)) release();
    };

    document.addEventListener("pointerover", onOver);
    document.addEventListener("pointerout", onOut);
    document.addEventListener("pointerdown", hideTip, true);
    document.addEventListener("scroll", hideTip, true);
    window.addEventListener("blur", release);
    return () => {
      release();
      document.removeEventListener("pointerover", onOver);
      document.removeEventListener("pointerout", onOut);
      document.removeEventListener("pointerdown", hideTip, true);
      document.removeEventListener("scroll", hideTip, true);
      window.removeEventListener("blur", release);
    };
  }, []);

  return tip ? createPortal(<FloatingTip text={tip.text} rect={tip.rect} />, document.body) : null;
}

/** Below the anchor, centered; flips above and clamps horizontally to stay in the viewport. */
function FloatingTip({ text, rect }: { text: string; rect: DOMRect }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    let top = rect.bottom + GAP;
    if (top + height > window.innerHeight - MARGIN) top = Math.max(MARGIN, rect.top - GAP - height);
    const centered = rect.left + rect.width / 2 - width / 2;
    const left = Math.min(Math.max(MARGIN, centered), window.innerWidth - width - MARGIN);
    setPos({ top, left });
  }, [rect]);

  return (
    <div
      ref={ref}
      role="tooltip"
      className="cr-tooltip cr-tooltip-float"
      style={pos ? { top: pos.top, left: pos.left } : { top: 0, left: 0, visibility: "hidden" }}
    >
      {text}
    </div>
  );
}
