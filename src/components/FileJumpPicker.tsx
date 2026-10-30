import { useEffect, useMemo, useRef, useState } from "react";

export interface FileEntry {
  path: string;
  add: number;
  del: number;
}

/** Sticky "jump to file" combobox: type to filter, arrows + Enter (or a click) to pick. */
export function FileJumpPicker({
  files,
  onPick,
  autoFocus,
  onDismiss,
}: {
  files: FileEntry[];
  onPick: (path: string) => void;
  autoFocus?: boolean;
  /** Called after a pick, or on Esc with nothing to clear; lets an overlay close itself. */
  onDismiss?: () => void;
}) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(0);
  const listRef = useRef<HTMLUListElement>(null);

  const matches = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? files.filter((f) => f.path.toLowerCase().includes(q)) : files;
  }, [files, query]);

  useEffect(() => {
    listRef.current?.children[cursor]?.scrollIntoView({ block: "nearest" });
  }, [cursor, open]);

  const pick = (path: string) => {
    onPick(path);
    setQuery("");
    setOpen(false);
    onDismiss?.();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      setOpen(true);
      const step = e.key === "ArrowDown" ? 1 : -1;
      setCursor((c) => (matches.length ? (c + step + matches.length) % matches.length : 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (matches[cursor]) pick(matches[cursor].path);
    } else if (e.key === "Escape") {
      if (open || query) {
        // Closes the list first; a second Esc closes the pane.
        e.preventDefault();
        setQuery("");
        setOpen(false);
      } else if (onDismiss) {
        e.preventDefault();
        onDismiss();
      }
    }
  };

  return (
    <div className="quick-view-filebar">
      <div className="quick-view-select">
        <input
          className="input"
          autoFocus={autoFocus}
          role="combobox"
          aria-expanded={open}
          aria-controls="quick-view-file-list"
          placeholder={`Jump to file… (${files.length} changed)`}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setCursor(0);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          // The input keeps focus after a pick or Esc, so a click must reopen the list too.
          onClick={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onKeyDown={onKeyDown}
        />
        {open && (
          <ul className="quick-view-options" id="quick-view-file-list" role="listbox" ref={listRef}>
            {matches.length === 0 && <li className="muted quick-view-empty">No matching file</li>}
            {matches.map((f, i) => (
              <li
                key={f.path}
                role="option"
                aria-selected={i === cursor}
                className={i === cursor ? "active" : ""}
                onMouseEnter={() => setCursor(i)}
                // mousedown, not click: the input's blur would close the list first.
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(f.path);
                }}
              >
                <span className="quick-view-option-path">{f.path}</span>
                <span className="add">+{f.add}</span>
                <span className="del">−{f.del}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
