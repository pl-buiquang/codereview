import { useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { toast } from "../lib/toast";

/** Split button for a GitHub PR: the main button opens the PR in the default
 *  browser; the caret reveals a menu whose secondary action copies the URL.
 *  Stops click propagation so it can sit inside a clickable row. */
export function OpenPrButton({ url, size }: { url: string; size?: "xs" }) {
  const [menuOpen, setMenuOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  const openInBrowser = async () => {
    try {
      await api.openUrl(url);
    } catch (e) {
      toast.error(`Could not open PR:\n${String(e)}`);
    }
  };

  const copyUrl = async () => {
    setMenuOpen(false);
    try {
      await navigator.clipboard.writeText(url);
      toast.success("PR URL copied to clipboard");
    } catch (e) {
      toast.error(`Could not copy URL:\n${String(e)}`);
    }
  };

  const cls = size === "xs" ? "btn-xs" : "";

  return (
    <div ref={ref} className="split-button" onClick={(e) => e.stopPropagation()}>
      <button
        className={`split-button-main ${cls}`}
        title="Open this PR in your browser"
        onClick={openInBrowser}
      >
        Open PR ↗
      </button>
      <button
        className={`split-button-toggle ${cls}`}
        title="More PR actions"
        aria-label="More PR actions"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onClick={() => setMenuOpen((o) => !o)}
      >
        ▾
      </button>
      {menuOpen && (
        <div className="split-button-menu" role="menu">
          <button role="menuitem" onClick={copyUrl}>
            Copy URL
          </button>
        </div>
      )}
    </div>
  );
}
