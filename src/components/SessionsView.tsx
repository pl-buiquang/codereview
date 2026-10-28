import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useRefetchOnActivate } from "../lib/useRefetchOnActivate";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { timeAgo } from "../lib/timeAgo";
import { isWaiting, useWaitingSessions } from "../lib/sessionCounts";
import { STATUS_LABEL, useActiveRoutineRuns, useUnseenRoutineProblems } from "../lib/routines";
import { useAiDisabledReason, useSettingsStore } from "../lib/settings";
import { useUIStore, type SessionsTab } from "../store";
import type { CardSession, ClaudeSession, SessionFocusState } from "../lib/types";
import { Icon } from "./icons";
import { Markdown } from "./Markdown";
import { RoutinesConfigTab } from "./routines/RoutinesConfigTab";
import { RoutinesRunsTab } from "./routines/RoutinesRunsTab";

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

export { isWaiting };

export function resumeCommand(s: Pick<ClaudeSession, "cwd" | "session_id">): string {
  return `cd ${shellQuote(s.cwd)}; claude --resume ${s.session_id}`;
}

/** Every whitespace-separated word of `q` must appear (case-insensitive) in the session's text. */
export function matchesSessionSearch(s: ClaudeSession, q: string): boolean {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const haystack = [
    s.name,
    s.original_name,
    s.session_id,
    s.cwd,
    tildePath(s.cwd),
    s.git_branch,
    s.repo,
    ...(s.summaries ?? []).map((sum) => sum.body),
  ]
    .filter(Boolean)
    .join("\n")
    .toLowerCase();
  return words.every((w) => haystack.includes(w));
}

function TerminalSessions() {
  const historyDays = useSettingsStore((s) => s.sessionHistoryDays);
  const summaryDir = useSettingsStore((s) => s.sessionSummaryDir);
  const defaultTerminal = useSettingsStore((s) => s.defaultTerminal);
  const tmuxTarget = useSettingsStore((s) => s.tmuxTarget);
  const tmuxSessionName = useSettingsStore((s) => s.tmuxSessionName);
  const showBoardCard = useUIStore((s) => s.showBoardCard);
  const homeActive = useUIStore((s) => s.activeTabId === "home");
  const queryClient = useQueryClient();

  const sessionsQuery = useQuery({
    queryKey: ["claude-sessions", historyDays, summaryDir],
    queryFn: () => api.listClaudeSessions(historyDays, summaryDir),
    // The home tab stays mounted (hidden) behind review tabs.
    refetchInterval: homeActive ? 3_000 : false,
    refetchOnWindowFocus: true,
  });
  useRefetchOnActivate(homeActive, sessionsQuery.refetch);
  const cardSessions = sessionsQuery.data?.card_sessions;

  const focus = useMutation({
    mutationFn: api.focusClaudeSession,
    onError: (e) => toast.error(`Could not focus terminal:\n${String(e)}`),
  });
  const resume = useMutation({
    mutationFn: (s: ClaudeSession) =>
      api.resumeClaudeSession(s.session_id, s.cwd, s.repo ?? null, defaultTerminal, tmuxTarget, tmuxSessionName),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["claude-sessions"] }),
    onError: (e) => toast.error(`Could not resume session:\n${String(e)}`),
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
    for (const link of cardSessions ?? []) {
      m.set(link.session_id, [...(m.get(link.session_id) ?? []), link]);
    }
    return m;
  }, [cardSessions]);

  const sessions = sessionsQuery.data?.sessions ?? [];
  const live = sessions.filter((s) => s.live);
  const history = sessions.filter((s) => !s.live);
  const [historySearch, setHistorySearch] = useState("");
  const filteredHistory = history.filter((s) => matchesSessionSearch(s, historySearch));
  const filtering = historySearch.trim() !== "";

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

            {historyDays > 0 && (
              <h2 className="sessions-group-head">
                History{filtering && ` · ${filteredHistory.length} of ${history.length}`}
              </h2>
            )}
            {historyDays > 0 && history.length === 0 && <p className="muted faint">No recent sessions.</p>}
            {historyDays > 0 && history.length > 0 && (
              <input
                type="search"
                className="input archive-search"
                aria-label="Filter history"
                placeholder="Filter history by name, folder, branch, repo, or summary…"
                value={historySearch}
                onChange={(e) => setHistorySearch(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setHistorySearch("");
                }}
              />
            )}
            {filtering && filteredHistory.length === 0 && history.length > 0 && (
              <p className="muted faint">No sessions match “{historySearch.trim()}”.</p>
            )}
            {filteredHistory.map((s) => (
              <SessionRow
                key={s.session_id}
                session={s}
                cards={[]}
                onShowCard={showBoardCard}
                action={
                  <div className="btn-split">
                    <button
                      className="btn btn-sm"
                      disabled={resume.isPending}
                      title={`Open a new terminal running: ${resumeCommand(s)}`}
                      onClick={() => resume.mutate(s)}
                    >
                      <Icon name="terminal" size={13} /> Resume
                    </button>
                    <button
                      className="btn btn-sm"
                      title={`Copy: ${resumeCommand(s)}`}
                      aria-label="Copy resume command"
                      onClick={() => copyCommand(resumeCommand(s), "Resume")}
                    >
                      <Icon name="copy" size={12} />
                    </button>
                  </div>
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
  const [expanded, setExpanded] = useState(false);
  const queryClient = useQueryClient();
  const fork = useSettingsStore((st) => st.sessionSummaryFork);
  const summaries = s.summaries ?? [];
  const canExpand = s.live || summaries.length > 0;
  const aiOff = useAiDisabledReason();
  const generate = useMutation({
    mutationFn: () => api.generateLiveSessionSummary(s.session_id, fork),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["claude-sessions"] }),
    onError: (e) => toast.error(`Could not summarize session:\n${String(e)}`),
  });
  return (
    <div className="card sessions-card">
      <div className="sessions-row-head">
        <span className={`sessions-dot sessions-dot-${status}`} title={status} />
        <div className="sessions-main">
          <div className="sessions-title">
            {canExpand ? (
              <button
                className="sessions-name-toggle"
                aria-expanded={expanded}
                title={s.original_name ?? undefined}
                onClick={() => setExpanded((v) => !v)}
              >
                <span className="sessions-name">{s.name ?? s.session_id}</span>
                <span
                  className={`sessions-expand${expanded ? "" : " sessions-expand-closed"}`}
                  aria-label={expanded ? "Hide summary" : "Show summary"}
                  role="img"
                >
                  <Icon name="chev" size={11} />
                </span>
              </button>
            ) : (
              <span className="sessions-name" title={s.original_name ?? undefined}>
                {s.name ?? s.session_id}
              </span>
            )}
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
      {expanded && (
        <div className="sessions-summaries">
          {summaries.map((sum, i) => (
            <div key={i} className="sessions-summary">
              {sum.generated && sum.date ? (
                <div className="sessions-summary-date">App summary · {timeAgo(sum.date)}</div>
              ) : (
                summaries.length > 1 && sum.date && <div className="sessions-summary-date">{sum.date}</div>
              )}
              <Markdown source={sum.body} />
            </div>
          ))}
          {s.live && (
            <button
              className="btn btn-sm"
              disabled={generate.isPending || aiOff != null}
              title={aiOff ?? "Summarize this session so far; stored separately from the end-of-session summaries"}
              onClick={() => generate.mutate()}
            >
              {generate.isPending ? "Summarizing…" : summaries.length > 0 ? "Regenerate summary" : "Generate summary"}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

const SESSION_TABS: { key: SessionsTab; label: string }[] = [
  { key: "terminal", label: "Terminal sessions" },
  { key: "routines", label: "Routines" },
  { key: "config", label: "Config" },
];

/** Sessions section: live Claude terminals, routine runs, and routine definitions. */
export function SessionsView() {
  const tab = useUIStore((s) => s.sessionsTab);
  const setTab = useUIStore((s) => s.setSessionsTab);
  const waiting = useWaitingSessions();
  const { unseen, markSeen } = useUnseenRoutineProblems();
  const activeRuns = [...useActiveRoutineRuns().values()].flat();
  // Looking at the runs is what clears the badge.
  useEffect(() => {
    if (tab === "routines" && unseen.length > 0) markSeen();
  }, [tab, unseen.length, markSeen]);
  const badges: Partial<Record<SessionsTab, { count: number; tone: "attention" | "error" | "info"; title: string }>> = {
    terminal: {
      count: waiting.length,
      tone: "attention",
      title: `Waiting for input:\n${waiting.map((s) => s.name ?? s.session_id.slice(0, 8)).join("\n")}`,
    },
    // Problems you haven't seen outrank the plain count of runs still going.
    routines:
      unseen.length > 0
        ? {
            count: unseen.length,
            tone: unseen.some((r) => r.status !== "needs_attention") ? "error" : "attention",
            title: `Since you last looked:\n${unseen.map((r) => `${r.routine_name} · ${r.context_label}: ${STATUS_LABEL[r.status]}`).join("\n")}`,
          }
        : {
            count: activeRuns.length,
            tone: "info",
            title: `Active:\n${activeRuns.map((r) => `${r.routine_name} · ${r.context_label}`).join("\n")}`,
    },
  };
  return (
    <div className="cr-main sessions-shell">
      <div className="cr-tabs" role="tablist">
        {SESSION_TABS.map((t) => (
          <button
            key={t.key}
            type="button"
            role="tab"
            aria-selected={t.key === tab}
            className={`cr-tab${t.key === tab ? " active" : ""}`}
            onClick={() => setTab(t.key)}
          >
            {t.label}
            {(badges[t.key]?.count ?? 0) > 0 && (
              <span className={`n n-${badges[t.key]!.tone}`} title={badges[t.key]!.title}>
                {badges[t.key]!.count > 99 ? "99+" : badges[t.key]!.count}
              </span>
            )}
          </button>
        ))}
      </div>
      {tab === "terminal" && <TerminalSessions />}
      {tab === "routines" && <RoutinesRunsTab />}
      {tab === "config" && <RoutinesConfigTab />}
    </div>
  );
}
