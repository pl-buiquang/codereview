import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSettingsStore } from "../lib/settings";
import { timeAgo } from "../lib/timeAgo";
import { useUIStore } from "../store";
import type { BoardCard, MonitorLogEntry } from "../lib/types";
import { Icon } from "./icons";

const LOG_LIMIT = 200;

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

  const openCard = (entry: MonitorLogEntry) => {
    if (visibleIds.has(entry.card_id)) showBoardCard(entry.card_id);
    else {
      const card = cards.find((c) => c.id === entry.card_id);
      if (card) api.openUrl(card.url);
    }
  };

  const entries = query.data ?? [];

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
        {query.isLoading ? (
          <span className="muted">Loading…</span>
        ) : entries.length === 0 ? (
          <span className="muted">
            Nothing done yet. Actions the monitor takes (branch updates, CI reruns, giving up) are
            logged here.
          </span>
        ) : (
          <ul className="board-digest-list">
            {entries.map((e) => (
              <li key={e.id} className="board-digest-row">
                <span className="cr-sub board-monitor-time" title={new Date(e.at).toLocaleString()}>
                  {timeAgo(e.at)}
                </span>
                <span className={KIND_CLASS[e.kind]}>{KIND_LABEL[e.kind]}</span>
                <button className="board-overview-ref" onClick={() => openCard(e)}>
                  {e.repo.split("/").pop()}#{e.number}
                </button>
                <span className="board-digest-title-text">{e.detail}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
