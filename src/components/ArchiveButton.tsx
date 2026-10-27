import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons";

/** Split button for a New inbox item: the primary action archives it (it comes back on a new
 *  reason); the menu archives and forgets it (it never comes back). */
export function ArchiveButton({
  busy,
  onArchive,
  onForget,
}: {
  busy: boolean;
  onArchive: () => void;
  onForget: () => void;
}) {
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

  return (
    <div ref={ref} className="btn-split">
      <button
        className="btn btn-sm btn-ghost"
        disabled={busy}
        onClick={onArchive}
        title="Archive — hide from New; comes back only if GitHub gives it a new reason"
      >
        Archive
      </button>
      <button
        className="btn btn-sm btn-ghost"
        disabled={busy}
        title="More archive options"
        aria-label="More archive options"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onClick={() => setMenuOpen((o) => !o)}
      >
        <Icon name="chev" size={11} />
      </button>
      {menuOpen && (
        <div className="btn-split-menu card" role="menu">
          <button
            role="menuitem"
            title="Never comes back to New, even with a new reason or activity"
            onClick={() => {
              setMenuOpen(false);
              onForget();
            }}
          >
            Archive and forget
          </button>
        </div>
      )}
    </div>
  );
}
