import { useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { Icon } from "./icons";

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

  const cls = size === "xs" ? "btn btn-sm" : "btn";

  return (
    <div ref={ref} className="btn-split" onClick={(e) => e.stopPropagation()}>
      <button className={cls} title="Open this PR in your browser" onClick={openInBrowser}>
        Open PR <Icon name="ext" size={11} />
      </button>
      <button
        className={cls}
        title="More PR actions"
        aria-label="More PR actions"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onClick={() => setMenuOpen((o) => !o)}
      >
        <Icon name="chev" size={11} />
      </button>
      {menuOpen && (
        <div className="btn-split-menu card" role="menu">
          <button role="menuitem" onClick={copyUrl}>
            Copy URL
          </button>
        </div>
      )}
    </div>
  );
}
