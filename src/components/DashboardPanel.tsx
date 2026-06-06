import { useUIStore, type HomeSection } from "../store";
import { InboxView } from "./InboxView";
import { ArchiveView } from "./ArchiveView";
import { RepositoriesView } from "./RepositoriesView";

const NAV: { key: HomeSection; label: string; emoji: string }[] = [
  { key: "inbox", label: "Inbox", emoji: "📨" },
  { key: "archive", label: "Archive", emoji: "🗄" },
  { key: "repositories", label: "Repositories", emoji: "📁" },
];

export function DashboardPanel() {
  const section = useUIStore((s) => s.homeSection);
  const setSection = useUIStore((s) => s.setHomeSection);
  const openSettingsTab = useUIStore((s) => s.openSettingsTab);

  return (
    <div className="dashboard">
      <nav className="dashboard-sidebar">
        <span className="app-title">codereview</span>
        <ul className="nav-list">
          {NAV.map((n) => (
            <li key={n.key}>
              <button
                className={`nav-item${section === n.key ? " active" : ""}`}
                onClick={() => setSection(n.key)}
              >
                <span className="nav-emoji">{n.emoji}</span>
                <span>{n.label}</span>
              </button>
            </li>
          ))}
        </ul>
        <button className="nav-item nav-settings" onClick={openSettingsTab} title="Settings">
          <span className="nav-emoji">⚙</span>
          <span>Settings</span>
        </button>
      </nav>

      <div className="dashboard-main">
        {section === "inbox" && <InboxView />}
        {section === "archive" && <ArchiveView />}
        {section === "repositories" && <RepositoriesView />}
      </div>
    </div>
  );
}
