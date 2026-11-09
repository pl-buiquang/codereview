import { useEffect, useRef, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import type { AgentSession } from "../lib/types";
import { Icon } from "./icons";
import { isWaiting } from "./SessionsView";

export function KebabMenu({
  title,
  label,
  disabled = false,
  className,
  children,
}: {
  title: string;
  /** Renders a labelled button instead of the icon-only kebab. */
  label?: ReactNode;
  disabled?: boolean;
  className?: string;
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  return (
    <div ref={ref} className={`kebab-menu${className ? ` ${className}` : ""}`}>
      {label != null ? (
        <button className="btn" title={title} disabled={disabled} onClick={() => setOpen((v) => !v)}>
          {label}
        </button>
      ) : (
        <button className="btn-icon" title={title} aria-label={title} disabled={disabled} onClick={() => setOpen((v) => !v)}>
          <Icon name="kebab" size={14} />
        </button>
      )}
      {open && (
        <div className="btn-split-menu card" role="menu">
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}

function sessionLabel(s: AgentSession): string {
  const status = isWaiting(s) ? `waiting: ${s.waiting_for ?? "input"}` : s.status;
  return `${s.name ?? s.session_id.slice(0, 8)}${status ? ` · ${status}` : ""}`;
}

export function AgentSessionItems({
  sessions,
  busy,
  disabledReason = null,
  close,
  onFocus,
  onOpenNew,
}: {
  sessions: AgentSession[];
  busy: boolean;
  disabledReason?: string | null;
  close: () => void;
  onFocus: (sessionId: string) => void;
  /** `harness` is set when the user picked one; otherwise the `sessions` profile decides. */
  onOpenNew: (harness?: string) => void;
}) {
  const disabled = busy || disabledReason != null;
  const harnesses = useQuery({ queryKey: ["ai-harnesses"], queryFn: () => api.aiHarnesses(), staleTime: 60_000 });
  const pickable = (harnesses.data ?? []).filter((h) => h.available && h.caps.interactive_tui);
  return (
    <>
      {sessions.map((s) => (
        <button
          key={s.session_id}
          role="menuitem"
          disabled={disabled}
          className="kebab-menu-session"
          title={`Focus agent session: ${sessionLabel(s)}\n${s.cwd}`}
          onClick={() => {
            close();
            onFocus(s.session_id);
          }}
        >
          {isWaiting(s) ? "Waiting" : "Focus"} agent: {s.name ?? s.session_id.slice(0, 8)}
        </button>
      ))}
      <button
        role="menuitem"
        disabled={disabled}
        title={disabledReason ?? "Open a new terminal running Claude in the chat worktree"}
        onClick={() => {
          close();
          onOpenNew();
        }}
      >
        New agent session
      </button>
      {pickable.length > 1 &&
        pickable.map((h) => (
          <button
            key={h.id}
            role="menuitem"
            disabled={disabled}
            title={`Open a new terminal running ${h.name} instead of the default`}
            onClick={() => {
              close();
              onOpenNew(h.id);
            }}
          >
            New {h.name} session
          </button>
        ))}
    </>
  );
}
