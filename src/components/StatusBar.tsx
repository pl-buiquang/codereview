import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { isWaiting, useLiveSessions } from "../lib/sessionCounts";
import { repoDisplayName } from "../lib/repoLabel";
import { useUIStore } from "../store";
import type { Target } from "../lib/types";

/** `#123` for a GitHub PR, `base...head` / `base..head` for a local virtual PR. */
export function targetLabel(t: Target): string {
  if (t.kind === "github_pr" && t.github_pr_number != null) return `#${t.github_pr_number}`;
  return `${t.base_ref}${t.three_dot ? "..." : ".."}${t.head_ref}`;
}

export function formatUsd(n: number): string {
  return `$${n.toFixed(2)}`;
}

/** Bottom strip: repo/target of the active tab, Claude sessions, today's AI spend, app version. A fixed
 *  `--statusbar-h` flex child of `.app-shell`, so everything above it shrinks instead of overflowing. */
export function StatusBar() {
  const tabs = useUIStore((s) => s.tabs);
  const activeTabId = useUIStore((s) => s.activeTabId);
  const homeRepoId = useUIStore((s) => s.homeRepoId);
  const setHomeSection = useUIStore((s) => s.setHomeSection);
  const active = tabs.find((t) => t.id === activeTabId) ?? tabs[0];
  const reviewId = active?.kind === "review" ? (active.reviewId ?? null) : null;

  const reposQuery = useQuery({ queryKey: ["repositories"], queryFn: api.listRepositories });
  const reviewQuery = useQuery({
    queryKey: ["review", reviewId],
    queryFn: () => api.getReview(reviewId as number),
    enabled: reviewId != null,
  });
  const usageQuery = useQuery({
    queryKey: ["ai-usage", "summary", "today"],
    queryFn: () => api.aiUsageSummary("today"),
  });
  const versionQuery = useQuery({ queryKey: ["appVersion"], queryFn: api.appVersion, staleTime: Infinity });
  const live = useLiveSessions();
  const waiting = live.filter(isWaiting).length;

  let location: { text: string; title: string } | null = null;
  if (reviewId != null && reviewQuery.data) {
    const d = reviewQuery.data;
    const repo = reposQuery.data?.find((r) => r.id === d.target.repo_id);
    const name = repo ? repoDisplayName(repo) : (d.friendly_name ?? "");
    location = {
      text: [name, targetLabel(d.target)].filter(Boolean).join(" "),
      title: `${d.target.title}\n${d.target.base_ref} ← ${d.target.head_ref}`,
    };
  } else if (reviewId == null && homeRepoId != null) {
    const repo = reposQuery.data?.find((r) => r.id === homeRepoId);
    if (repo) location = { text: repoDisplayName(repo), title: repo.local_path ?? "" };
  }

  const totals = usageQuery.data?.totals;
  const tokens = totals
    ? totals.input_tokens + totals.output_tokens + totals.cache_read_tokens + totals.cache_creation_tokens
    : 0;
  const runs = totals ? `${totals.runs} run${totals.runs === 1 ? "" : "s"}` : "";

  return (
    <footer className="status-bar" role="contentinfo">
      <div className="status-bar-left">
        {location && (
          <span className="status-bar-seg" title={location.title}>
            {location.text}
          </span>
        )}
        {live.length > 0 && (
          <button className="status-bar-seg" onClick={() => setHomeSection("sessions")} title="Open Claude sessions">
            {live.length} live
            {waiting > 0 && (
              <>
                {" · "}
                <span className="attention">{waiting} waiting</span>
              </>
            )}
          </button>
        )}
        {totals && (
          <span className="status-bar-seg" title={`AI usage today: ${runs}, ${tokens.toLocaleString()} tokens`}>
            {formatUsd(totals.cost_usd)} · {runs} today
          </span>
        )}
      </div>
      {versionQuery.data && (
        <span className="status-bar-version" title={`CodeReview ${versionQuery.data}`}>
          {versionQuery.data}
        </span>
      )}
    </footer>
  );
}
