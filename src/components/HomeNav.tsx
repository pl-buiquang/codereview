import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { identicon } from "../lib/avatar";
import { useBoardActionCount } from "../lib/boardCounts";
import { useInboxCounts } from "../lib/inboxCounts";
import { useWaitingSessions } from "../lib/sessionCounts";
import { repoLabel, repoShortName } from "../lib/repoLabel";
import { useUIStore, type HomeSection } from "../store";
import { Icon, type IconName } from "./icons";
import { Tooltip } from "./Tooltip";
import { SyncIndicator } from "./SyncIndicator";

const REPO_DRAG_MIME = "application/x-cr-repo";

const NAV: { key: HomeSection; label: string; icon: IconName }[] = [
  { key: "sessions", label: "Sessions", icon: "terminal" },
  { key: "board", label: "Board", icon: "board" },
  { key: "inbox", label: "Inbox", icon: "inbox" },
  { key: "reviews", label: "Reviews", icon: "review" },
  { key: "worktrees", label: "Worktrees", icon: "branch" },
  { key: "repositories", label: "Repositories", icon: "repo" },
];

export function HomeNav({
  collapsed,
  onNavigate,
  className = "",
}: {
  collapsed: boolean;
  onNavigate?: () => void;
  className?: string;
}) {
  const activeTabId = useUIStore((s) => s.activeTabId);
  const section = useUIStore((s) => s.homeSection);
  const homeRepoId = useUIStore((s) => s.homeRepoId);
  const openRepoIds = useUIStore((s) => s.openRepoIds);
  const setSection = useUIStore((s) => s.setHomeSection);
  const openRepo = useUIStore((s) => s.openRepo);
  const closeRepo = useUIStore((s) => s.closeRepo);
  const moveOpenRepo = useUIStore((s) => s.moveOpenRepo);
  const [dragOverId, setDragOverId] = useState<number | null>(null);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const openSettingsTab = useUIStore((s) => s.openSettingsTab);
  const onHome = activeTabId === "home";
  const inboxCounts = useInboxCounts();
  const waitingSessions = useWaitingSessions();
  const boardActions = useBoardActionCount();
  const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? "" : "s"}`;
  const badges: Partial<Record<HomeSection, { count: number; title: string; attention?: boolean }>> = {
    inbox: {
      count: inboxCounts.total,
      title: `${plural(inboxCounts.newItems, "new item")}, ${plural(inboxCounts.unreadConversations, "unread conversation")}`,
    },
    sessions: {
      count: waitingSessions.length,
      title: `Waiting for input:\n${waitingSessions
        .map((s) => `${s.name ?? s.session_id.slice(0, 8)} (${s.waiting_for ?? "input needed"})`)
        .join("\n")}`,
      attention: true,
    },
    board: { count: boardActions, title: `${plural(boardActions, "card")} ${boardActions === 1 ? "needs" : "need"} your action` },
  };
  const versionQuery = useQuery({ queryKey: ["appVersion"], queryFn: api.appVersion, staleTime: Infinity });

  const { data: repos } = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });
  const openRepos = openRepoIds.flatMap((id) => repos?.find((r) => r.id === id) ?? []);

  const go = (fn: () => void) => () => {
    fn();
    onNavigate?.();
  };

  return (
    <nav className={`cr-side${collapsed ? " collapsed" : ""} ${className}`}>
      <div className="cr-side-brand">
        <span className="cr-side-logo">cr</span>
        {!collapsed && <span className="cr-side-brand-name">codereview</span>}
        <button
          className="btn-icon cr-side-toggle"
          onClick={toggleSidebar}
          title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          <Icon name="sidebar" size={14} />
        </button>
      </div>
      <div className="cr-nav">
        {NAV.map((n) => {
          const badge = badges[n.key];
          return (
          <button
            key={n.key}
            className={`cr-nav-item${onHome && homeRepoId == null && section === n.key ? " active" : ""}`}
            onClick={go(() => setSection(n.key))}
            title={collapsed ? n.label : undefined}
          >
            <Icon name={n.icon} size={15} />
            {!collapsed && n.label}
            {badge && badge.count > 0 && (
              <span className={`cr-nav-badge${badge.attention ? " attention" : ""}`} title={badge.title}>
                {badge.count > 99 ? "99+" : badge.count}
              </span>
            )}
          </button>
          );
        })}
      </div>
      {openRepos.length > 0 && (
        <div className="cr-nav cr-open-repos">
          {collapsed ? (
            <div className="cr-open-repos-sep" />
          ) : (
            <div className="cr-open-repos-head">Open</div>
          )}
          {openRepos.map((repo) => {
            const label = repoLabel(repo);
            return (
              <Tooltip key={repo.id} content={label}>
              <div
                role="button"
                tabIndex={0}
                className={`cr-nav-item cr-open-repo${onHome && homeRepoId === repo.id ? " active" : ""}${
                  dragOverId === repo.id ? " drag-over" : ""
                }`}
                draggable
                onDragStart={(e) => {
                  e.dataTransfer.setData(REPO_DRAG_MIME, String(repo.id));
                  e.dataTransfer.effectAllowed = "move";
                }}
                onDragOver={(e) => {
                  if (!e.dataTransfer.types.includes(REPO_DRAG_MIME)) return;
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "move";
                  setDragOverId(repo.id);
                }}
                onDragLeave={() => setDragOverId((id) => (id === repo.id ? null : id))}
                onDragEnd={() => setDragOverId(null)}
                onDrop={(e) => {
                  e.preventDefault();
                  setDragOverId(null);
                  const fromId = Number(e.dataTransfer.getData(REPO_DRAG_MIME));
                  if (fromId) moveOpenRepo(fromId, repo.id);
                }}
                onClick={go(() => openRepo(repo.id))}
                onMouseDown={(e) => {
                  if (e.button === 1) e.preventDefault();
                }}
                onAuxClick={(e) => {
                  if (e.button === 1) closeRepo(repo.id);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    openRepo(repo.id);
                    onNavigate?.();
                  }
                }}
                aria-label={label}
              >
                <img className="cr-open-repo-avatar" src={identicon(label)} alt="" />
                {!collapsed && (
                  <>
                    <span className="cr-open-repo-label">{repo.friendly_name?.trim() || repoShortName(repo)}</span>
                    <button
                      className="cr-open-repo-close"
                      title="Close repository"
                      aria-label={`Close ${label}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        closeRepo(repo.id);
                      }}
                    >
                      <Icon name="x" size={10} />
                    </button>
                  </>
                )}
              </div>
              </Tooltip>
            );
          })}
        </div>
      )}
      <div className="cr-side-foot cr-nav">
        <SyncIndicator collapsed={collapsed} />
        <button
          className={`cr-nav-item${activeTabId === "settings" ? " active" : ""}`}
          onClick={go(openSettingsTab)}
          title="Settings"
        >
          <Icon name="gear" size={15} />
          {!collapsed && "Settings"}
        </button>
        {!collapsed && versionQuery.data && (
          <div className="cr-side-version" title={`CodeReview ${versionQuery.data}`}>
            {versionQuery.data}
          </div>
        )}
      </div>
    </nav>
  );
}
