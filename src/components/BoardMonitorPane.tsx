import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSettingsStore } from "../lib/settings";
import { timeAgo } from "../lib/timeAgo";
import { useUIStore } from "../store";
import type { BoardCard, MonitorLogEntry } from "../lib/types";
import { CiBadge, MergeStateBadge, MergeableBadge } from "./InboxBadges";
import { Icon } from "./icons";

// Only the newest action per card is shown; the full history lives in the app log.
const LOG_LIMIT = 500;

const KIND_LABEL: Record<MonitorLogEntry["kind"], string> = {
  updated: "Updated branch",
  reran: "Reran CI",
  gave_up: "Gave up",
  error: "Error",
};

const KIND_CLASS: Record<MonitorLogEntry["kind"], string> = {
  updated: "board-monitor-kind ok",
  reran: "board-monitor-kind ok",
  gave_up: "board-monitor-kind warn",
  error: "board-monitor-kind bad",
};

/** Mirrors `board_monitor::candidates`: the PRs the monitor acts on. */
function isMonitored(c: BoardCard): boolean {
  return (
    c.type === "pr" &&
    c.state === "open" &&
    !c.is_draft &&
    !c.discarded_at &&
    !c.archived_at &&
    c.auto_merge != null &&
    c.review_decision === "APPROVED"
  );
}

export function BoardMonitorPane({
  cards,
  visibleIds,
}: {
  cards: BoardCard[];
  visibleIds: Set<string>;
}) {
  const monitorOn = useSettingsStore((s) => s.boardMonitor);
  const setMonitorOn = useSettingsStore((s) => s.setBoardMonitor);
  const intervalMin = useSettingsStore((s) => s.boardMonitorIntervalMin);
  const showBoardCard = useUIStore((s) => s.showBoardCard);
  const query = useQuery({
    queryKey: ["board-monitor-log"],
    queryFn: () => api.boardMonitorLog(LOG_LIMIT),
  });

  const openCard = (card: BoardCard) => {
    if (visibleIds.has(card.id)) showBoardCard(card.id);
    else api.openUrl(card.url);
  };

  // The log is newest first, so the first entry seen per card is its last action.
  const lastByCard = new Map<string, MonitorLogEntry>();
  for (const e of query.data ?? []) if (!lastByCard.has(e.card_id)) lastByCard.set(e.card_id, e);
  const watched = cards.filter(isMonitored);

  return (
    <section className="board-overview">
      <div className="board-overview-head">
        <Icon name="refresh" size={14} />
        <strong>Auto-merge monitor</strong>
        <span className="cr-sub">
          {monitorOn
            ? `on · every ${intervalMin} min · updates BEHIND branches, reruns failed CI (max 3 attempts) on approved PRs with auto-merge`
            : "off"}
        </span>
        <span className="cr-spacer" />
        <button className="btn btn-sm" onClick={() => setMonitorOn(!monitorOn)}>
          {monitorOn ? "Turn off" : "Turn on"}
        </button>
      </div>
      <div className="board-overview-body">
        {watched.length === 0 ? (
          <span className="muted">No approved PRs with auto-merge on.</span>
        ) : (
          <ul className="board-digest-list">
            {watched.map((c) => {
              const last = lastByCard.get(c.id);
              return (
                <li key={c.id} className="board-digest-row">
                  <button className="board-overview-ref" onClick={() => openCard(c)}>
                    {c.repo.split("/").pop()}#{c.number}
                  </button>
                  <CiBadge state={c.ci_status ? c.ci_status.toLowerCase() : null} />
                  <MergeableBadge mergeable={c.mergeable} />
                  <MergeStateBadge state={c.merge_state_status} stacked={c.stack_number != null} />
                  <span className="board-digest-title-text">{c.title}</span>
                  {last ? (
                    <>
                      <span className={KIND_CLASS[last.kind]}>{KIND_LABEL[last.kind]}</span>
                      <span className="cr-sub">{last.detail}</span>
                      <span className="cr-sub board-monitor-time" title={new Date(last.at).toLocaleString()}>
                        {timeAgo(last.at)}
                      </span>
                    </>
                  ) : (
                    <span className="cr-sub">no action yet</span>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </section>
  );
}
