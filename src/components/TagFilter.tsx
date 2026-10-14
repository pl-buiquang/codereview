import { useEffect, useId, useRef, useState } from "react";
import { Icon } from "./icons";

/** Tag autocomplete + chip filter for the board rail. Matching across
 *  selected tags within one instance is OR (include: card shows if it
 *  carries any selected tag; exclude: card is hidden if it carries any
 *  selected tag). `unavailable` hides tags already picked in a sibling
 *  filter (e.g. the exclude list hides tags already included). */
export function TagFilter({
  tags,
  selected,
  onChange,
  unavailable,
  title = "Tags",
  placeholder = "Filter by tag…",
  variant = "include",
}: {
  tags: [string, number][];
  selected: string[];
  onChange: (next: string[]) => void;
  unavailable?: string[];
  title?: string;
  placeholder?: string;
  variant?: "include" | "exclude";
}) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const listboxId = useId();

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  if (tags.length === 0 && selected.length === 0) return null;

  const q = query.trim().toLowerCase();
  const options = tags.filter(
    ([t]) => !selected.includes(t) && !unavailable?.includes(t) && (!q || t.toLowerCase().includes(q)),
  );
  const activeHighlight = Math.min(highlight, Math.max(options.length - 1, 0));

  const select = (tag: string) => {
    onChange([...selected, tag]);
    setQuery("");
    setHighlight(0);
    setOpen(false);
  };

  const remove = (tag: string) => onChange(selected.filter((t) => t !== tag));

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setOpen(true);
      setHighlight((h) => Math.min(h + 1, Math.max(options.length - 1, 0)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlight((h) => Math.max(h - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const opt = options[activeHighlight];
      if (opt) select(opt[0]);
    } else if (e.key === "Escape") {
      setOpen(false);
    } else if (e.key === "Backspace" && query === "" && selected.length > 0) {
      remove(selected[selected.length - 1]);
    }
  };

  return (
    <div className="cr-rail-group tag-filter" ref={rootRef}>
      <div className="cr-rail-h">
        <span>{title}</span>
        {selected.length > 0 && (
          <button className="cr-rail-clear" onClick={() => onChange([])}>
            clear
          </button>
        )}
      </div>
      <div className="tag-filter-combobox">
        <input
          className="input tag-filter-input"
          role="combobox"
          aria-expanded={open}
          aria-controls={listboxId}
          placeholder={placeholder}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setHighlight(0);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={onKeyDown}
        />
        {open && (
          <div className="tag-filter-dropdown card" id={listboxId} role="listbox">
            {options.length === 0 && <div className="tag-filter-empty faint">No matching tags</div>}
            {options.map(([tag, count], i) => (
              <button
                key={tag}
                type="button"
                role="option"
                aria-selected={i === activeHighlight}
                className={`tag-filter-option${i === activeHighlight ? " on" : ""}`}
                onMouseEnter={() => setHighlight(i)}
                onClick={() => select(tag)}
              >
                <span className="lbl">{tag}</span>
                <span className="count">{count}</span>
              </button>
            ))}
          </div>
        )}
      </div>
      {selected.length > 0 && (
        <div className="tag-filter-chips">
          {selected.map((tag) => (
            <span key={tag} className={`tag-chip${variant === "exclude" ? " tag-chip-exclude" : ""}`}>
              {tag}
              <button className="btn-icon" title={`Remove ${tag} filter`} onClick={() => remove(tag)}>
                <Icon name="x" size={10} />
              </button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
