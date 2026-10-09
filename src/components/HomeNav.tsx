import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { identicon } from "../lib/avatar";
import { repoLabel } from "../lib/repoLabel";
import { useUIStore, type HomeSection } from "../store";
import { Icon, type IconName } from "./icons";

const NAV: { key: HomeSection; label: string; icon: IconName }[] = [
  { key: "inbox", label: "Inbox", icon: "inbox" },
  { key: "reviews", label: "Reviews", icon: "review" },
  { key: "archive", label: "Archive", icon: "archive" },
  { key: "repositories", label: "Repositories", icon: "repo" },
];

export function HomeNav({
  collapsed,
  showToggle = true,
  onNavigate,
  className = "",
}: {
  collapsed: boolean;
  showToggle?: boolean;
  onNavigate?: () => void;
  className?: string;
}) {
  const section = useUIStore((s) => s.homeSection);
  const homeRepoId = useUIStore((s) => s.homeRepoId);
  const pinnedRepoIds = useUIStore((s) => s.pinnedRepoIds);
  const setSection = useUIStore((s) => s.setHomeSection);
  const openRepo = useUIStore((s) => s.openRepo);
  const togglePinRepo = useUIStore((s) => s.togglePinRepo);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const openSettingsTab = useUIStore((s) => s.openSettingsTab);

  const { data: repos } = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });
  const pinned = pinnedRepoIds.flatMap((id) => repos?.find((r) => r.id === id) ?? []);

  const go = (fn: () => void) => () => {
    fn();
    onNavigate?.();
  };

  return (
    <nav className={`cr-side${collapsed ? " collapsed" : ""} ${className}`}>
      <div className="cr-side-brand">
        <span className="cr-side-logo">cr</span>
        {!collapsed && <span className="cr-side-brand-name">codereview</span>}
        {showToggle && (
          <button
            className="btn-icon cr-side-toggle"
            onClick={toggleSidebar}
            title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          >
            <Icon name="sidebar" size={14} />
          </button>
        )}
      </div>
      <div className="cr-nav">
        {NAV.map((n) => (
          <button
            key={n.key}
            className={`cr-nav-item${section === n.key && homeRepoId == null ? " active" : ""}`}
            onClick={go(() => setSection(n.key))}
            title={collapsed ? n.label : undefined}
          >
            <Icon name={n.icon} size={15} />
            {!collapsed && n.label}
          </button>
        ))}
      </div>
      {pinned.length > 0 && (
        <div className="cr-nav cr-pinned">
          {collapsed ? (
            <div className="cr-pinned-sep" />
          ) : (
            <div className="cr-pinned-head">Pinned</div>
          )}
          {pinned.map((repo) => {
            const label = repoLabel(repo);
            return (
              <div
                key={repo.id}
                role="button"
                tabIndex={0}
                className={`cr-nav-item cr-pinned-item${homeRepoId === repo.id ? " active" : ""}`}
                onClick={go(() => openRepo(repo.id))}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    openRepo(repo.id);
                    onNavigate?.();
                  }
                }}
                title={label}
              >
                <img className="cr-pinned-avatar" src={identicon(label)} alt="" />
                {!collapsed && (
                  <>
                    <span className="cr-pinned-label">{label}</span>
                    <button
                      className="cr-pinned-unpin"
                      title="Unpin"
                      aria-label={`Unpin ${label}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        togglePinRepo(repo.id);
                      }}
                    >
                      <Icon name="x" size={10} />
                    </button>
                  </>
                )}
              </div>
            );
          })}
        </div>
      )}
      <div className="cr-side-foot cr-nav">
        <button className="cr-nav-item" onClick={go(openSettingsTab)} title="Settings">
          <Icon name="gear" size={15} />
          {!collapsed && "Settings"}
        </button>
      </div>
    </nav>
  );
}
