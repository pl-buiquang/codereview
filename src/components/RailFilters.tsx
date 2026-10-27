import type { ReactNode } from "react";

export function MultiFilterList({
  title,
  entries,
  selected,
  onChange,
  renderLabel,
}: {
  title: string;
  entries: [string, number][];
  selected: string[];
  onChange: (next: string[]) => void;
  renderLabel?: (key: string) => ReactNode;
}) {
  if (entries.length === 0) return null;
  return (
    <div className="cr-rail-group">
      <div className="cr-rail-h">
        <span>{title}</span>
        {selected.length > 0 && (
          <button className="cr-rail-clear" onClick={() => onChange([])}>
            clear
          </button>
        )}
      </div>
      {entries.map(([key, count]) => {
        const on = selected.includes(key);
        return (
          <button
            key={key}
            className={`cr-rail-item${on ? " on" : ""}`}
            aria-pressed={on}
            onClick={() => onChange(on ? selected.filter((k) => k !== key) : [...selected, key])}
          >
            {renderLabel ? renderLabel(key) : <span className="lbl">{key}</span>}
            <span className="count">{count}</span>
          </button>
        );
      })}
    </div>
  );
}

export function FilterList({
  title,
  entries,
  selected,
  onSelect,
  ident,
  renderLabel,
}: {
  title: string;
  entries: [string, number][];
  selected: string | null;
  onSelect: (v: string | null) => void;
  ident?: boolean;
  renderLabel?: (key: string) => ReactNode;
}) {
  if (entries.length === 0) return null;
  return (
    <div className="cr-rail-group">
      <div className="cr-rail-h">
        <span>{title}</span>
        {selected && (
          <button className="cr-rail-clear" onClick={() => onSelect(null)}>
            clear
          </button>
        )}
      </div>
      {entries.map(([key, count]) => (
        <button
          key={key}
          className={`cr-rail-item${key === selected ? " on" : ""}`}
          onClick={() => onSelect(key === selected ? null : key)}
        >
          {renderLabel ? renderLabel(key) : <span className={ident ? "lbl ident" : "lbl"}>{key}</span>}
          <span className="count">{count}</span>
        </button>
      ))}
    </div>
  );
}
