import { createContext } from "react";
import type { ReactNode } from "react";

const RailIdContext = createContext("");

/** The filter rail of a view. `id` scopes per-section state to the view. */
export function FilterRail({ id, children }: { id: string; children: ReactNode }) {
  return (
    <RailIdContext.Provider value={id}>
      <aside className="cr-rail">{children}</aside>
    </RailIdContext.Provider>
  );
}

/** One titled section of a filter rail, with a "clear" action when `onClear` is given. */
export function RailSection({
  title,
  onClear,
  className,
  children,
}: {
  title: string;
  onClear?: () => void;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div className={`cr-rail-group${className ? ` ${className}` : ""}`}>
      <div className="cr-rail-h">
        <span>{title}</span>
        {onClear && (
          <button className="cr-rail-clear" onClick={onClear}>
            clear
          </button>
        )}
      </div>
      {children}
    </div>
  );
}

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
    <RailSection title={title} onClear={selected.length > 0 ? () => onChange([]) : undefined}>
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
    </RailSection>
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
    <RailSection title={title} onClear={selected ? () => onSelect(null) : undefined}>
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
    </RailSection>
  );
}
