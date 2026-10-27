import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useUIStore } from "../store";
import type { LogLevel } from "../lib/log";
import {
  AUTO_FETCH_OPTIONS,
  AUTO_SYNC_OPTIONS,
  BOARD_BADGE_KIND_OPTIONS,
  DEFAULT_BOARD_BADGE_KINDS,
  LOG_LEVEL_OPTIONS,
  BOARD_MONITOR_OPTIONS,
  DISCOVERY_INTERVAL_OPTIONS,
  exportSettings,
  importSettings,
  parsePromptOverrides,
  parseRepoBasePaths,
  TERMINAL_HOST_OPTIONS,
  TMUX_TARGET_OPTIONS,
  useSettingsStore,
  type BoardBorderSide,
  type TerminalHost,
  type TmuxTarget,
} from "../lib/settings";
import { DEFAULT_CONFIG_PATTERNS, DEFAULT_TEST_PATTERNS } from "../lib/fileCategory";
import { DEFAULT_SNOOZE_OPTIONS, isValidSnoozeOptions } from "../lib/snooze";
import { api, pickJsonFile, pickSavePath } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { invalidateMirrorQueries } from "../lib/sync";
import type { AutoSync, ToolEnv } from "../lib/types";
import { Icon, type IconName } from "./icons";
import { AiSection } from "./SettingsAiSection";
import { ThemeSection } from "./SettingsThemeSection";
import { ConversationRulesEditor } from "./SettingsConversationRules";
import { UsageSection } from "./SettingsUsageSection";

type SettingsSection = "general" | "theme" | "ai" | "usage" | "logs";

const SECTIONS: { key: SettingsSection; label: string; icon: IconName }[] = [
  { key: "general", label: "General", icon: "gear" },
  { key: "theme", label: "Theme", icon: "palette" },
  { key: "ai", label: "AI", icon: "bot" },
  { key: "usage", label: "Usage", icon: "flame" },
  { key: "logs", label: "Logs", icon: "file" },
];

function EnvironmentPanel() {
  const envQuery = useQuery({ queryKey: ["environment"], queryFn: api.checkEnvironment });

  if (envQuery.isLoading) return <p className="muted">Checking tools…</p>;
  if (envQuery.isError || !envQuery.data)
    return <p className="error">Could not check environment: {String(envQuery.error)}</p>;

  const env: ToolEnv = envQuery.data;
  return (
    <div className="env-rows">
      <ToolRow
        label="git"
        value={env.git}
        missingHint="Not found on PATH — install git and reopen the app."
      />
      <ToolRow
        label="gh"
        value={env.gh}
        missingHint="Not found on PATH — install the GitHub CLI to list/publish PRs."
      />
      <div className="settings-row">
        <span>GitHub auth</span>
        {env.gh == null ? (
          <span className="muted">—</span>
        ) : env.gh_authed ? (
          <span className="env-ok">Authenticated</span>
        ) : (
          <span className="error">
            Not authenticated — run <code>gh auth login</code>.
          </span>
        )}
      </div>
    </div>
  );
}

function ToolRow({
  label,
  value,
  missingHint,
}: {
  label: string;
  value: string | null;
  missingHint: string;
}) {
  return (
    <div className="settings-row">
      <span>{label}</span>
      {value ? (
        <code className="env-path" title={value}>
          {value}
        </code>
      ) : (
        <span className="error">{missingHint}</span>
      )}
    </div>
  );
}

const LOG_LINES = 400;

function LogsSection() {
  const logLevel = useSettingsStore((s) => s.logLevel);
  const setLogLevel = useSettingsStore((s) => s.setLogLevel);
  const pathQuery = useQuery({ queryKey: ["log-path"], queryFn: api.logPath });
  const logQuery = useQuery({
    queryKey: ["app-log"],
    queryFn: () => api.readAppLog(LOG_LINES),
    refetchInterval: 3000,
  });

  const copyPath = () => {
    if (!pathQuery.data) return;
    navigator.clipboard.writeText(pathQuery.data).then(
      () => toast.success("Copied log path"),
      (e) => toast.error(String(e)),
    );
  };

  return (
    <section className="settings-group">
      <h3>Logs</h3>
      <label
        className="settings-row"
        title="How much the app writes to the log file. Warning is the default; pick Debug to trace every backend call, sync, git/gh command and query refetch while diagnosing a problem."
      >
        <span>Log level</span>
        <select className="select" value={logLevel} onChange={(e) => setLogLevel(e.target.value as LogLevel)}>
          {LOG_LEVEL_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
      <div className="settings-row">
        <span>Log file</span>
        <span className="settings-control">
          <code className="mono settings-log-path" title={pathQuery.data ?? undefined}>
            {pathQuery.data ?? "…"}
          </code>
          <button className="btn btn-sm" disabled={!pathQuery.data} onClick={copyPath}>
            Copy path
          </button>
          <button
            className="btn btn-sm"
            disabled={!pathQuery.data}
            onClick={() => pathQuery.data && api.openInDefaultApp(pathQuery.data).catch((e) => toast.error(String(e)))}
          >
            Open
          </button>
          <button className="btn btn-sm" onClick={() => logQuery.refetch()} disabled={logQuery.isFetching}>
            Refresh
          </button>
        </span>
      </div>
      <p className="muted settings-log-hint">
        Last {LOG_LINES} lines. Rotates to <code className="mono">.log.old</code> past 5&nbsp;MB. Auto-refreshes
        every few seconds while this page is open.
      </p>
      <pre className="settings-log-view">
        {logQuery.isLoading
          ? "Loading…"
          : logQuery.data && logQuery.data.length > 0
            ? logQuery.data
            : "Nothing logged yet."}
      </pre>
    </section>
  );
}

function AppVersionRow() {
  const versionQuery = useQuery({ queryKey: ["appVersion"], queryFn: api.appVersion });
  return (
    <p className="settings-version muted">
      CodeReview {versionQuery.data ?? "…"}
    </p>
  );
}

function GeneralSection() {
  const defaultViewType = useSettingsStore((s) => s.defaultViewType);
  const defaultThreeDot = useSettingsStore((s) => s.defaultThreeDot);
  const botLogins = useSettingsStore((s) => s.botLogins);
  const repoStripPrefixes = useSettingsStore((s) => s.repoStripPrefixes);
  const repoBasePaths = useSettingsStore((s) => s.repoBasePaths);
  const setDefaultViewType = useSettingsStore((s) => s.setDefaultViewType);
  const setDefaultThreeDot = useSettingsStore((s) => s.setDefaultThreeDot);
  const markEdits = useSettingsStore((s) => s.markEdits);
  const boardStatusBorderSide = useSettingsStore((s) => s.boardStatusBorderSide);
  const setBotLogins = useSettingsStore((s) => s.setBotLogins);
  const setRepoStripPrefixes = useSettingsStore((s) => s.setRepoStripPrefixes);
  const setRepoBasePaths = useSettingsStore((s) => s.setRepoBasePaths);
  const setMarkEdits = useSettingsStore((s) => s.setMarkEdits);
  const largeDiffMaxLines = useSettingsStore((s) => s.largeDiffMaxLines);
  const setLargeDiffMaxLines = useSettingsStore((s) => s.setLargeDiffMaxLines);
  const setBoardStatusBorderSide = useSettingsStore((s) => s.setBoardStatusBorderSide);
  const autoSync = useSettingsStore((s) => s.autoSync);
  const setAutoSync = useSettingsStore((s) => s.setAutoSync);
  const discoveryIntervalMin = useSettingsStore((s) => s.discoveryIntervalMin);
  const setDiscoveryIntervalMin = useSettingsStore((s) => s.setDiscoveryIntervalMin);
  const boardArchiveDays = useSettingsStore((s) => s.boardArchiveDays);
  const setBoardArchiveDays = useSettingsStore((s) => s.setBoardArchiveDays);
  const boardCountDone = useSettingsStore((s) => s.boardCountDone);
  const boardNeedsActionSnoozed = useSettingsStore((s) => s.boardNeedsActionSnoozed);
  const setBoardNeedsActionSnoozed = useSettingsStore((s) => s.setBoardNeedsActionSnoozed);
  const boardBadgeKinds = useSettingsStore((s) => s.boardBadgeKinds);
  const setBoardBadgeKind = useSettingsStore((s) => s.setBoardBadgeKind);
  const boardSnoozeOptions = useSettingsStore((s) => s.boardSnoozeOptions);
  const setBoardSnoozeOptions = useSettingsStore((s) => s.setBoardSnoozeOptions);
  const [snoozeDraft, setSnoozeDraft] = useState(boardSnoozeOptions);
  const setBoardCountDone = useSettingsStore((s) => s.setBoardCountDone);
  const boardMonitorIntervalMin = useSettingsStore((s) => s.boardMonitorIntervalMin);
  const setBoardMonitorIntervalMin = useSettingsStore((s) => s.setBoardMonitorIntervalMin);
  const autoFetchMinutes = useSettingsStore((s) => s.autoFetchMinutes);
  const setAutoFetchMinutes = useSettingsStore((s) => s.setAutoFetchMinutes);
  const fileTestPatterns = useSettingsStore((s) => s.fileTestPatterns);
  const fileConfigPatterns = useSettingsStore((s) => s.fileConfigPatterns);
  const defaultTerminal = useSettingsStore((s) => s.defaultTerminal);
  const setDefaultTerminal = useSettingsStore((s) => s.setDefaultTerminal);
  const tmuxTarget = useSettingsStore((s) => s.tmuxTarget);
  const setTmuxTarget = useSettingsStore((s) => s.setTmuxTarget);
  const tmuxSessionName = useSettingsStore((s) => s.tmuxSessionName);
  const setTmuxSessionName = useSettingsStore((s) => s.setTmuxSessionName);
  const sessionHistoryDays = useSettingsStore((s) => s.sessionHistoryDays);
  const setSessionHistoryDays = useSettingsStore((s) => s.setSessionHistoryDays);
  const sessionSummaryDir = useSettingsStore((s) => s.sessionSummaryDir);
  const setSessionSummaryDir = useSettingsStore((s) => s.setSessionSummaryDir);
  const sessionSummaryFork = useSettingsStore((s) => s.sessionSummaryFork);
  const setSessionSummaryFork = useSettingsStore((s) => s.setSessionSummaryFork);
  const setFileTestPatterns = useSettingsStore((s) => s.setFileTestPatterns);
  const setFileConfigPatterns = useSettingsStore((s) => s.setFileConfigPatterns);

  const onExportSettings = async () => {
    try {
      const dest = await pickSavePath("codereview-settings.json", "json");
      if (!dest) return;
      const prompts = Object.fromEntries(
        (await api.promptsList()).flatMap((p) => (p.override != null ? [[p.key, p.override]] : [])),
      );
      await api.writeSettingsFile(dest, exportSettings(prompts));
      toast.success("Settings exported");
    } catch (err) {
      toast.error(`Export failed:\n${String(err)}`);
    }
  };

  const onImportSettings = async () => {
    try {
      const src = await pickJsonFile();
      if (!src) return;
      const json = await api.readSettingsFile(src);
      const count = importSettings(json);
      const overrides = parsePromptOverrides(json);
      if (overrides) {
        const known = await api.promptsList();
        await Promise.all(
          known.map((p) =>
            p.key in overrides ? api.promptSet(p.key, overrides[p.key]) : api.promptReset(p.key),
          ),
        );
        queryClient.invalidateQueries({ queryKey: ["prompts"] });
      }
      toast.success(`Imported ${count} setting${count === 1 ? "" : "s"}`);
    } catch (err) {
      toast.error(`Import failed:\n${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const queryClient = useQueryClient();
  const resetMirror = useMutation({
    mutationFn: api.resetMirror,
    onSuccess: () => {
      invalidateMirrorQueries();
      toast.success("Mirror reset — rebuilding from GitHub");
    },
    onError: (err) => toast.error(`Reset failed:\n${String(err)}`),
  });
  const onResetMirror = async () => {
    const ok = await confirmDialog({
      title: "Reset mirror",
      message:
        "Delete the local copy of GitHub PRs, issues and users, then rebuild it from GitHub? Inbox triage, board tracking and reviews are kept; they show placeholders until the rebuild lands.",
      confirmLabel: "Reset",
    });
    if (ok) resetMirror.mutate();
  };
  const autoLink = useMutation({
    mutationFn: () => api.autoLinkRepos(parseRepoBasePaths(repoBasePaths)),
    onSuccess: (count) => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      toast.success(count > 0 ? `Linked ${count} repo${count === 1 ? "" : "s"}` : "No repos to auto-link");
    },
    onError: (err) => toast.error(`Auto-link failed:\n${String(err)}`),
  });

  return (
    <div className="settings-section-narrow">
      <section className="settings-group">
        <h3>Review defaults</h3>
        <div className="settings-row">
          <span>Default diff view</span>
          <span className="view-toggle">
            <button
              className={defaultViewType === "split" ? "active" : ""}
              onClick={() => setDefaultViewType("split")}
            >
              Split
            </button>
            <button
              className={defaultViewType === "unified" ? "active" : ""}
              onClick={() => setDefaultViewType("unified")}
            >
              Unified
            </button>
          </span>
        </div>
        <label className="settings-row">
          <span>Default to merge-base (three-dot) diff</span>
          <input
            type="checkbox"
            checked={defaultThreeDot}
            onChange={(e) => setDefaultThreeDot(e.target.checked)}
          />
        </label>
        <label className="settings-row">
          <span>Highlight intra-line changes</span>
          <input
            type="checkbox"
            checked={markEdits}
            onChange={(e) => setMarkEdits(e.target.checked)}
          />
        </label>
        <label
          className="settings-row"
          title="Files with more changed lines are not rendered (to keep the UI responsive). Open them in VSCode instead."
        >
          <span>Max diff lines per file</span>
          <input
            type="number"
            className="input"
            min={100}
            step={100}
            value={largeDiffMaxLines}
            onChange={(e) => {
              const n = Number(e.target.value);
              if (Number.isFinite(n)) setLargeDiffMaxLines(Math.max(100, Math.round(n)));
            }}
          />
        </label>
      </section>

      <section className="settings-group">
        <h3>Board</h3>
        <label className="settings-row">
          <span>PR status border side</span>
          <select
            className="select"
            value={boardStatusBorderSide}
            onChange={(e) => setBoardStatusBorderSide(e.target.value as BoardBorderSide)}
          >
            <option value="top">Top</option>
            <option value="right">Right</option>
            <option value="bottom">Bottom</option>
            <option value="left">Left</option>
          </select>
        </label>
        <label
          className="settings-row"
          title="The Done column's archive button permanently archives cards whose PR was closed or merged more than this many days ago."
        >
          <span>Archive done cards after (days)</span>
          <input
            type="number"
            className="input"
            min={1}
            max={365}
            value={boardArchiveDays}
            onChange={(e) => {
              const n = Number(e.target.value);
              if (Number.isFinite(n)) setBoardArchiveDays(Math.max(1, Math.min(365, Math.round(n))));
            }}
          />
        </label>
        <label
          className="settings-row"
          title="Whether cards in the Done lane are included in the numbers next to the Board's filter options. The Done lane itself is unchanged."
        >
          <span>Count Done cards in filter counts</span>
          <input
            type="checkbox"
            checked={boardCountDone}
            onChange={(e) => setBoardCountDone(e.target.checked)}
          />
        </label>
        {BOARD_BADGE_KIND_OPTIONS.map((o) => (
          <label key={o.kind} className="settings-row" title={`${o.hint}. Counted in the sidebar Board badge when on.`}>
            <span>Badge: {o.label}</span>
            <input
              type="checkbox"
              checked={boardBadgeKinds[o.kind] ?? DEFAULT_BOARD_BADGE_KINDS[o.kind]}
              onChange={(e) => setBoardBadgeKind(o.kind, e.target.checked)}
            />
          </label>
        ))}
        <label
          className="settings-row"
          title="Whether the Board's Needs action filter also shows cards you snoozed. The sidebar badge still leaves them out."
        >
          <span>Needs action filter: include snoozed cards</span>
          <input
            type="checkbox"
            checked={boardNeedsActionSnoozed}
            onChange={(e) => setBoardNeedsActionSnoozed(e.target.checked)}
          />
        </label>
        <label
          className="settings-row"
          title="Durations offered by a card's Snooze button, comma-separated: a number then h (hours), d (days) or w (weeks). A snoozed card is left out of the sidebar Board badge until it lapses."
        >
          <span>Snooze durations</span>
          <input
            className="input"
            value={snoozeDraft}
            aria-invalid={!isValidSnoozeOptions(snoozeDraft)}
            placeholder={DEFAULT_SNOOZE_OPTIONS}
            style={isValidSnoozeOptions(snoozeDraft) ? undefined : { borderColor: "var(--danger)" }}
            onChange={(e) => {
              setSnoozeDraft(e.target.value);
              if (isValidSnoozeOptions(e.target.value)) setBoardSnoozeOptions(e.target.value.trim());
            }}
          />
        </label>
      </section>

      <section className="settings-group">
        <h3>Sync</h3>
        <label
          className="settings-row"
          title="When Inbox, Board and Review sync from GitHub on their own. On focus: when a view opens or the window regains focus. Periodic: also every few minutes while the app is visible (slower when nothing changes)."
        >
          <span>Auto Sync</span>
          <select className="select" value={autoSync} onChange={(e) => setAutoSync(e.target.value as AutoSync)}>
            {AUTO_SYNC_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label
          className="settings-row"
          title="Search GitHub for new review requests, mentions and authored PRs this often while the app is visible. GitHub notifications trigger a search as soon as something changes, so this is a safety net (at least every 30 min) unless notifications are unavailable. Needs Auto Sync on; a Sync click always looks."
        >
          <span>Look for new PRs/issues every</span>
          <select
            className="select"
            value={discoveryIntervalMin}
            onChange={(e) => setDiscoveryIntervalMin(Number(e.target.value))}
          >
            {DISCOVERY_INTERVAL_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label
          className="settings-row"
          title="How often the Board's Auto-merge monitor runs. It updates BEHIND branches and reruns failed CI (up to 3 attempts) on approved PRs with auto-merge on. It runs while the app is open, even in the background, once switched on from the Board."
        >
          <span>Auto-merge monitor every</span>
          <select
            className="select"
            value={boardMonitorIntervalMin}
            onChange={(e) => setBoardMonitorIntervalMin(Number(e.target.value))}
          >
            {BOARD_MONITOR_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <div className="settings-row" title="Wipe the local copy of GitHub data and rebuild it. Your own state is kept.">
          <span>GitHub mirror</span>
          <span className="settings-control">
            <button className="btn btn-sm" onClick={onResetMirror} disabled={resetMirror.isPending}>
              Reset mirror…
            </button>
          </span>
        </div>
      </section>

      <section className="settings-group">
        <h3>Repositories</h3>
        <label
          className="settings-row"
          title="git fetch a repository when its Graph opens or the window regains focus, if the last fetch is older than this"
        >
          <span>Background fetch</span>
          <select
            className="select"
            value={autoFetchMinutes}
            onChange={(e) => setAutoFetchMinutes(Number(e.target.value))}
          >
            {AUTO_FETCH_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.value === 0 ? o.label : `when older than ${o.label}`}
              </option>
            ))}
          </select>
        </label>
      </section>

      <section className="settings-group">
        <h3>Sessions</h3>
        <label className="settings-row">
          <span>Default terminal</span>
          <select
            className="select"
            value={defaultTerminal}
            onChange={(e) => setDefaultTerminal(e.target.value as TerminalHost)}
          >
            {TERMINAL_HOST_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label className="settings-row" title="New terminals open as a tmux window; the terminal app only attaches tmux">
          <span>Open new terminals in</span>
          <select
            className="select"
            value={tmuxTarget}
            onChange={(e) => setTmuxTarget(e.target.value as TmuxTarget)}
          >
            {TMUX_TARGET_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label className="settings-row" title="Used by 'Shared session', and by 'Current' when no tmux client is attached">
          <span>Shared tmux session</span>
          <input
            className="input"
            value={tmuxSessionName}
            placeholder="codereview"
            onChange={(e) => setTmuxSessionName(e.target.value)}
          />
        </label>
        <label className="settings-row">
          <span>History (days)</span>
          <input
            type="number"
            className="input"
            min={0}
            max={30}
            value={sessionHistoryDays}
            onChange={(e) => {
              const n = Number(e.target.value);
              if (Number.isFinite(n)) setSessionHistoryDays(Math.max(0, Math.min(30, Math.round(n))));
            }}
          />
        </label>
        <label
          className="settings-row"
          title="Markdown session summaries (frontmatter with session_id, optional title) shown on history cards. Empty = off."
        >
          <span>History summary directory</span>
          <input
            className="input"
            value={sessionSummaryDir}
            placeholder="~/Nextcloud/Notes/_work/sessions"
            onChange={(e) => setSessionSummaryDir(e.target.value)}
          />
        </label>
        <label
          className="settings-row"
          title="Generate live-session summaries by forking the session when it was active in the last 5 minutes, reusing its prompt cache. Cache hits vary: a miss is billed at the session's own model price. Off = a cheap haiku run over a transcript extract."
        >
          <span>Summarize live sessions by forking</span>
          <input
            type="checkbox"
            checked={sessionSummaryFork}
            onChange={(e) => setSessionSummaryFork(e.target.checked)}
          />
        </label>
      </section>

      <section className="settings-group">
        <h3>File types</h3>
        <label className="settings-row settings-row-stack">
          <span>Test patterns</span>
          <textarea
            className="textarea settings-patterns"
            rows={6}
            spellCheck={false}
            value={fileTestPatterns}
            onChange={(e) => setFileTestPatterns(e.target.value)}
          />
        </label>
        <label className="settings-row settings-row-stack">
          <span>Config &amp; docs patterns</span>
          <textarea
            className="textarea settings-patterns"
            rows={6}
            spellCheck={false}
            value={fileConfigPatterns}
            onChange={(e) => setFileConfigPatterns(e.target.value)}
          />
          <span className="settings-hint muted">
            One glob per line (<code>#</code> for comments). Patterns without a <code>/</code>{" "}
            match the file name (<code>*_spec.rb</code>); others match the full path (
            <code>**/tests/**</code>). Tests are checked first, then config &amp; docs; anything
            else counts as code.
          </span>
        </label>
        <div className="settings-row">
          <span />
          <button
            className="btn btn-sm"
            onClick={() => {
              setFileTestPatterns(DEFAULT_TEST_PATTERNS);
              setFileConfigPatterns(DEFAULT_CONFIG_PATTERNS);
            }}
          >
            Reset to defaults
          </button>
        </div>
      </section>

      <section className="settings-group">
        <h3>Inbox</h3>
        <label className="settings-row settings-row-stack">
          <span>Bot logins</span>
          <input
            type="text"
            className="input"
            placeholder="dependabot, renovate"
            value={botLogins}
            onChange={(e) => setBotLogins(e.target.value)}
          />
          <span className="settings-hint muted">
            Comma-separated GitHub logins routed to the Bots tab (case-insensitive; GitHub
            strips the <code>[bot]</code> suffix).
          </span>
        </label>
        <label className="settings-row settings-row-stack">
          <span>Strip repo prefixes</span>
          <input
            type="text"
            className="input"
            placeholder="philips-internal/cardiologs-"
            value={repoStripPrefixes}
            onChange={(e) => setRepoStripPrefixes(e.target.value)}
          />
          <span className="settings-hint muted">
            Comma-separated prefixes removed from repo names in the inbox. Hovering a stripped
            name shows the full <code>owner/name</code>.
          </span>
        </label>
        <ConversationRulesEditor />
      </section>

      <section className="settings-group">
        <h3>Repositories</h3>
        <label className="settings-row settings-row-stack">
          <span>Base paths for auto-linking</span>
          <input
            type="text"
            className="input"
            placeholder="~/projects, ~/work"
            value={repoBasePaths}
            onChange={(e) => setRepoBasePaths(e.target.value)}
          />
          <span className="settings-hint muted">
            Comma-separated directories where your git clones live. When a GitHub PR arrives,
            the app searches here (recursively, 2 levels) to auto-link a local clone.
          </span>
        </label>
        <div className="settings-row">
          <span />
          <button
            className="btn btn-sm"
            onClick={() => autoLink.mutate()}
            disabled={autoLink.isPending}
          >
            {autoLink.isPending ? "Scanning…" : "Scan now"}
          </button>
        </div>
      </section>

      <section className="settings-group">
        <h3>Backup</h3>
        <div className="settings-row">
          <span>Settings file (JSON)</span>
          <span className="settings-control">
            <button className="btn btn-sm" onClick={onExportSettings}>
              Export…
            </button>
            <button className="btn btn-sm" onClick={onImportSettings}>
              Import…
            </button>
          </span>
        </div>
      </section>

      <section className="settings-group">
        <h3>Environment</h3>
        <EnvironmentPanel />
      </section>

      <AppVersionRow />
    </div>
  );
}

export function SettingsView() {
  const closeSettings = useUIStore((s) => s.closeSettings);
  const [section, setSection] = useState<SettingsSection>("general");

  return (
    <section className="main-panel settings-panel">
      <header className="main-header settings-header">
        <button onClick={closeSettings}>← Back</button>
        <h2>Settings</h2>
      </header>

      <div className="settings-layout">
        <nav className="settings-sidebar cr-nav">
          {SECTIONS.map((s) => (
            <button
              key={s.key}
              className={`cr-nav-item${section === s.key ? " active" : ""}`}
              onClick={() => setSection(s.key)}
            >
              <Icon name={s.icon} size={15} />
              <span>{s.label}</span>
            </button>
          ))}
        </nav>

        <div className="settings-content">
          {section === "general" && <GeneralSection />}
          {section === "theme" && <ThemeSection />}
          {section === "ai" && <AiSection />}
          {section === "usage" && <UsageSection />}
          {section === "logs" && <LogsSection />}
        </div>
      </div>
    </section>
  );
}
