import { useMemo } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { timeAgo } from "../lib/timeAgo";
import { useSettingsStore } from "../lib/settings";
import { useUIStore } from "../store";
import type { CardSession, ClaudeSession, SessionFocusState } from "../lib/types";
import { Icon } from "./icons";

const FOCUS_DISABLED_REASON: Record<Exclude<SessionFocusState, "ready">, string> = {
  detached: "No terminal is attached to this tmux session",
  background: "Background session — no terminal",
  no_terminal: "This session has no terminal",
  not_live: "Not running",
};

export function tildePath(path: string): string {
  return path.replace(/^\/(Users|home)\/[^/]+(?=\/|$)/, "~");
}

function shellQuote(s: string): string {
  return /^[\w./~@%+=:-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`;
}

export function attachCommand(s: Pick<ClaudeSession, "session_id" | "job_id">): string {
  return `claude attach ${s.job_id ?? s.session_id.slice(0, 8)}`;
}

export function isWaiting(s: Pick<ClaudeSession, "live" | "status" | "waiting_for">): boolean {
  return s.live && (s.status === "waiting" || s.waiting_for != null);
}

export function resumeCommand(s: Pick<ClaudeSession, "cwd" | "session_id">): string {
  return `cd ${shellQuote(s.cwd)}; claude --resume ${s.session_id}`;
}

export function SessionsView() {
  const historyDays = useSettingsStore((s) => s.sessionHistoryDays);
  const showBoardCard = useUIStore((s) => s.showBoardCard);
  const queryClient = useQueryClient();

  const sessionsQuery = useQuery({
    queryKey: ["claude-sessions", historyDays],
    queryFn: () => api.listClaudeSessions(historyDays),
    refetchInterval: 3_000,
  });
  const cardSessionsQuery = useQuery({
    queryKey: ["board-card-sessions"],
    queryFn: api.boardCardSessions,
    refetchInterval: 3_000,
  });

  const focus = useMutation({
    mutationFn: api.focusClaudeSession,
    onError: (e) => toast.error(`Could not focus terminal:\n${String(e)}`),
  });
  const stop = useMutation({
    mutationFn: api.stopClaudeSession,
    onSuccess: () => {
      toast.success("Background session stopped");
      queryClient.invalidateQueries({ queryKey: ["claude-sessions"] });
    },
    onError: (e) => toast.error(`Could not stop session:\n${String(e)}`),
  });
  const confirmStop = async (s: ClaudeSession) => {
    if (
      await confirmDialog({
        title: "Stop background session",
        message: `Stop "${s.name ?? s.session_id}"? Its conversation is kept; resume it later with ${attachCommand(s)}.`,
        confirmLabel: "Stop",
        danger: true,
      })
    )
      stop.mutate(s.session_id);
  };

  const cardsBySession = useMemo(() => {
    const m = new Map<string, CardSession[]>();
    for (const link of cardSessionsQuery.data ?? []) {
      m.set(link.session_id, [...(m.get(link.session_id) ?? []), link]);
    }
    return m;
  }, [cardSessionsQuery.data]);

  const sessions = sessionsQuery.data ?? [];
  const live = sessions.filter((s) => s.live);
  const history = sessions.filter((s) => !s.live);

  const copyCommand = (command: string, what: string) =>
    navigator.clipboard.writeText(command).then(
      () => toast.success(`${what} command copied to clipboard`),
      (e) => toast.error(`Could not copy: ${String(e)}`),
    );

  return (
    <section className="cr-main">
      <header className="cr-pagehead">
        <div>
          <h1 className="cr-h1">Sessions</h1>
          <p className="cr-sub">
            {live.length} live · {history.length} in the last {historyDays} day{historyDays === 1 ? "" : "s"}
          </p>
        </div>
      </header>

      <div className="cr-list sessions-list">
        {sessionsQuery.isLoading && <p className="muted">Loading…</p>}
        {sessionsQuery.isError && <p className="muted">Could not list sessions: {String(sessionsQuery.error)}</p>}

        {!sessionsQuery.isLoading && (
          <>
            <h2 className="sessions-group-head">Live</h2>
            {live.length === 0 && <p className="muted faint">No running Claude sessions.</p>}
            {live.map((s) => (
              <SessionRow
                key={s.session_id}
                session={s}
                cards={cardsBySession.get(s.session_id) ?? []}
                onShowCard={showBoardCard}
                action={
                  <>
                    {s.kind === "bg" && s.job_id && (
                      <button
                        className="btn btn-sm btn-ghost"
                        disabled={stop.isPending}
                        title={`claude stop ${s.job_id}`}
                        onClick={() => confirmStop(s)}
                      >
                        Stop
                      </button>
                    )}
                    {s.focus === "background" ? (
                      <BackgroundOpenButton
                        session={s}
                        busy={focus.isPending}
                        onFocus={() => focus.mutate(s.session_id)}
                        onCopy={() => copyCommand(attachCommand(s), "Attach")}
                      />
                    ) : (
                      <button
                        className="btn btn-sm"
                        disabled={s.focus !== "ready" || focus.isPending}
                        title={s.focus === "ready" ? "Focus this session's terminal" : FOCUS_DISABLED_REASON[s.focus]}
                        onClick={() => focus.mutate(s.session_id)}
                      >
                        <Icon name="terminal" size={13} /> Focus
                      </button>
                    )}
                  </>
                }
              />
            ))}

            {historyDays > 0 && <h2 className="sessions-group-head">History</h2>}
            {historyDays > 0 && history.length === 0 && <p className="muted faint">No recent sessions.</p>}
            {history.map((s) => (
              <SessionRow
                key={s.session_id}
                session={s}
                cards={[]}
                onShowCard={showBoardCard}
                action={
                  <button className="btn btn-sm" title={resumeCommand(s)} onClick={() => copyCommand(resumeCommand(s), "Resume")}>
                    <Icon name="copy" size={13} /> Copy resume
                  </button>
                }
              />
            ))}
          </>
        )}
      </div>
    </section>
  );
}

/** Background sessions nobody is attached to: jump to an agent view to open it, else copy attach. */
function BackgroundOpenButton({
  session: s,
  busy,
  onFocus,
  onCopy,
}: {
  session: ClaudeSession;
  busy: boolean;
  onFocus: () => void;
  onCopy: () => void;
}) {
  const view = s.agent_view;
  if (view?.focus !== "ready") {
    return (
      <button className="btn btn-sm" title={`${attachCommand(s)} — open this background session in a terminal`} onClick={onCopy}>
        <Icon name="copy" size={13} /> Copy attach
      </button>
    );
  }
  const where = view.tmux?.label ? `tmux ${view.tmux.label}` : view.tty;
  return (
    <div className="btn-split">
      <button
        className="btn btn-sm"
        disabled={busy}
        title={`Focus the agent view (${where}${view.cwd ? `, ${tildePath(view.cwd)}` : ""}) — open "${s.name ?? s.job_id}" from there`}
        onClick={onFocus}
      >
        <Icon name="terminal" size={13} /> Agent view
      </button>
      <button className="btn btn-sm" title={`Copy: ${attachCommand(s)}`} aria-label="Copy attach command" onClick={onCopy}>
        <Icon name="copy" size={12} />
      </button>
    </div>
  );
}

function statusClass(s: ClaudeSession): string {
  if (!s.live) return "ended";
  if (isWaiting(s)) return "waiting";
  return s.status === "busy" ? "busy" : "idle";
}

function SessionRow({
  session: s,
  cards,
  onShowCard,
  action,
}: {
  session: ClaudeSession;
  cards: CardSession[];
  onShowCard: (cardId: string) => void;
  action: React.ReactNode;
}) {
  const status = statusClass(s);
  return (
    <div className="card sessions-row">
      <span className={`sessions-dot sessions-dot-${status}`} title={status} />
      <div className="sessions-main">
        <div className="sessions-title">
          <span className="sessions-name">{s.name ?? s.session_id}</span>
          {s.kind === "bg" && (
            <span className="badge badge-pending" title={s.attached ? "Background session viewed via claude attach" : undefined}>
              {s.attached ? "attached" : "background"}
            </span>
          )}
          {s.focus === "detached" && <span className="badge badge-draft">detached</span>}
          {isWaiting(s) && <span className="badge badge-review">{s.waiting_for ?? "waiting"}</span>}
          {cards.map((c) => (
            <button
              key={c.card_id}
              className="badge badge-pr badge-toggle"
              title="Show on the Board"
              onClick={() => onShowCard(c.card_id)}
            >
              {c.repo.split("/").pop()}#{c.number}
            </button>
          ))}
        </div>
        <div className="sessions-meta">
          <span className="chip" title={s.cwd}>
            <Icon name="folder" size={11} /> {tildePath(s.cwd)}
          </span>
          {s.git_branch && (
            <span className="chip">
              <Icon name="branch" size={11} /> {s.git_branch}
            </span>
          )}
          {s.tmux?.label && (
            <span className="chip" title={`tmux pane ${s.tmux.pane}`}>
              <Icon name="terminal" size={11} /> {s.tmux.label}
            </span>
          )}
          <span className="chip">{timeAgo(s.updated_at)}</span>
        </div>
      </div>
      <div className="sessions-actions">{action}</div>
    </div>
  );
}
