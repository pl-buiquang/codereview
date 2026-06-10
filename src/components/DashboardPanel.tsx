import { useUIStore, type HomeSection } from "../store";
import { InboxView } from "./InboxView";
import { ReviewsView } from "./ReviewsView";
import { ArchiveView } from "./ArchiveView";
import { RepositoriesView } from "./RepositoriesView";
import { Icon, type IconName } from "./icons";

const NAV: { key: HomeSection; label: string; icon: IconName }[] = [
  { key: "inbox", label: "Inbox", icon: "inbox" },
  { key: "reviews", label: "Reviews", icon: "review" },
  { key: "archive", label: "Archive", icon: "archive" },
  { key: "repositories", label: "Repositories", icon: "repo" },
];

export function DashboardPanel() {
  const section = useUIStore((s) => s.homeSection);
  const setSection = useUIStore((s) => s.setHomeSection);
  const openSettingsTab = useUIStore((s) => s.openSettingsTab);

  return (
    <div className="dashboard">
      <nav className="cr-side">
        <div className="cr-side-brand">
          <span className="cr-side-logo">cr</span>
          codereview
        </div>
        <div className="cr-nav">
          {NAV.map((n) => (
            <button
              key={n.key}
              className={`cr-nav-item${section === n.key ? " active" : ""}`}
              onClick={() => setSection(n.key)}
            >
              <Icon name={n.icon} size={15} />
              {n.label}
            </button>
          ))}
        </div>
        <div className="cr-side-foot cr-nav">
          <button className="cr-nav-item" onClick={openSettingsTab} title="Settings">
            <Icon name="gear" size={15} />
            Settings
          </button>
        </div>
      </nav>

      <div className="dashboard-main">
        {section === "inbox" && <InboxView />}
        {section === "reviews" && <ReviewsView />}
        {section === "archive" && <ArchiveView />}
        {section === "repositories" && <RepositoriesView />}
      </div>
    </div>
  );
}
