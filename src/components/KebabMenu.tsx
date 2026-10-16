import { useEffect, useRef, useState, type ReactNode } from "react";
import type { ClaudeSession } from "../lib/types";
import { Icon } from "./icons";
import { isWaiting } from "./SessionsView";

export function KebabMenu({
  title,
  disabled = false,
  className,
  children,
}: {
  title: string;
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
      <button className="btn-icon" title={title} aria-label={title} disabled={disabled} onClick={() => setOpen((v) => !v)}>
        <Icon name="kebab" size={14} />
      </button>
      {open && (
        <div className="btn-split-menu card" role="menu">
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}

function sessionLabel(s: ClaudeSession): string {
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
  sessions: ClaudeSession[];
  busy: boolean;
  disabledReason?: string | null;
  close: () => void;
  onFocus: (sessionId: string) => void;
  onOpenNew: () => void;
}) {
  const disabled = busy || disabledReason != null;
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
    </>
  );
}
