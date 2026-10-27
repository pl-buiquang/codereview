import { createContext, useContext } from "react";
import type { ReactNode } from "react";
import { useUIStore } from "../store";
import { Icon } from "./icons";

const RailIdContext = createContext("");

/** The filter rail of a view. `id` scopes per-section state to the view. */
export function FilterRail({ id, children }: { id: string; children: ReactNode }) {
  return (
    <RailIdContext.Provider value={id}>
      <aside className="cr-rail">{children}</aside>
    </RailIdContext.Provider>
  );
}

/** One titled, collapsible section of a filter rail, with a "clear" action when `onClear` is
 *  given. The fold state is persisted per rail + title; `summary` stands in for the hidden
 *  selection while folded. */
export function RailSection({
  title,
  summary,
  onClear,
  className,
  children,
}: {
  title: string;
  summary?: ReactNode;
  onClear?: () => void;
  className?: string;
  children: ReactNode;
}) {
  const railId = useContext(RailIdContext);
  const key = `${railId}:${title}`;
  const collapsed = useUIStore((s) => s.collapsedRailSections.includes(key));
  const toggle = useUIStore((s) => s.toggleRailSection);
  return (
    <div className={`cr-rail-group${className ? ` ${className}` : ""}`}>
      <div className="cr-rail-h">
        <button
          className={`cr-rail-toggle${collapsed ? " collapsed" : ""}`}
          aria-expanded={!collapsed}
          title={`${collapsed ? "Expand" : "Collapse"} ${title}`}
          onClick={() => toggle(key)}
        >
          <Icon name="chev" size={10} className="cr-rail-chev" />
          <span className="cr-rail-title">{title}</span>
          {collapsed && summary != null && <span className="cr-rail-summary">· {summary}</span>}
        </button>
        {onClear && (
          <button className="cr-rail-clear" onClick={onClear}>
            clear
          </button>
        )}
      </div>
      {!collapsed && children}
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
    <RailSection
      title={title}
      summary={selected.length > 0 ? selected.length : undefined}
      onClear={selected.length > 0 ? () => onChange([]) : undefined}
    >
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
    <RailSection
      title={title}
      summary={selected ? (renderLabel ? renderLabel(selected) : selected) : undefined}
      onClear={selected ? () => onSelect(null) : undefined}
    >
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
