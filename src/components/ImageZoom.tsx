import { useEffect, useRef, useState } from "react";

const MIN_SCALE = 0.02;
const MAX_SCALE = 40;

/** Full-window zoom and pan of one picture: wheel zooms around the cursor, drag pans, Esc or a
 *  double click on the backdrop closes. */
export function ImageZoom({ src, onClose }: { src: string; onClose: () => void }) {
  const [view, setView] = useState({ scale: 1, x: 0, y: 0 });
  const rootRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ px: number; py: number; x: number; y: number } | null>(null);

  useEffect(() => {
    // Capture phase: a pane underneath that also closes on Esc must not see this one.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopImmediatePropagation();
      onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  // Not a React onWheel: that one is passive and could not stop the page behind from scrolling.
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      setView((v) => {
        const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, v.scale * Math.exp(-e.deltaY * 0.0015)));
        const k = scale / v.scale;
        return { scale, x: cx - (cx - v.x) * k, y: cy - (cy - v.y) * k };
      });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const fit = (img: HTMLImageElement) => {
    const el = rootRef.current;
    if (!el || !img.naturalWidth) return;
    const { width, height } = el.getBoundingClientRect();
    const scale = Math.min((width * 0.9) / img.naturalWidth, (height * 0.9) / img.naturalHeight, 1);
    setView({
      scale,
      x: (width - img.naturalWidth * scale) / 2,
      y: Math.max(16, (height - img.naturalHeight * scale) / 2),
    });
  };

  return (
    <div
      ref={rootRef}
      className="image-zoom"
      role="dialog"
      aria-label="Image zoom"
      onDoubleClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onPointerDown={(e) => {
        drag.current = { px: e.clientX, py: e.clientY, x: view.x, y: view.y };
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onPointerMove={(e) => {
        const d = drag.current;
        if (d) setView((v) => ({ ...v, x: d.x + e.clientX - d.px, y: d.y + e.clientY - d.py }));
      }}
      onPointerUp={() => (drag.current = null)}
    >
      <img
        src={src}
        alt=""
        draggable={false}
        onLoad={(e) => fit(e.currentTarget)}
        style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})` }}
      />
      <div className="image-zoom-hint">Scroll: zoom · Drag: pan · Esc: close</div>
      <button className="btn-icon image-zoom-close" title="Close (Esc)" onClick={onClose}>
        ✕
      </button>
    </div>
  );
}
