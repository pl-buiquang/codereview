import { memo } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useUIStore } from "../store";
import { InboxView } from "./InboxView";
import { ReviewsView } from "./ReviewsView";
import { ArchiveView } from "./ArchiveView";
import { RepositoriesView } from "./RepositoriesView";
import { RepoView } from "./RepoView";
import { HomeNav } from "./HomeNav";

export function DashboardPanel() {
  const section = useUIStore((s) => s.homeSection);
  const homeRepoId = useUIStore((s) => s.homeRepoId);
  const collapsed = useUIStore((s) => s.sidebarCollapsed);
  const openRepoIds = useUIStore((s) => s.openRepoIds);
  // Stable order so closing/opening repos never moves the mounted subtrees.
  const panes = [...openRepoIds].sort((a, b) => a - b);

  return (
    <div className="dashboard">
      <HomeNav collapsed={collapsed} />
      <div className="dashboard-main">
        {panes.map((id) => (
          <RepoSlot key={id} repoId={id} active={id === homeRepoId} />
        ))}
        {homeRepoId == null && (
          <>
            {section === "inbox" && <InboxView />}
            {section === "reviews" && <ReviewsView />}
            {section === "archive" && <ArchiveView />}
            {section === "repositories" && <RepositoriesView />}
          </>
        )}
      </div>
    </div>
  );
}

// Open repos stay mounted (hidden when not shown) so switching keeps their
// state and is instant, as when they were separate tabs.
const RepoSlot = memo(function RepoSlot({ repoId, active }: { repoId: number; active: boolean }) {
  return (
    <div className="tab-pane" style={active ? undefined : { display: "none" }}>
      <RepoPane repoId={repoId} />
    </div>
  );
});

const RepoPane = memo(function RepoPane({ repoId }: { repoId: number }) {
  const { data: repos, isLoading } = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });
  const repo = repos?.find((r) => r.id === repoId);
  if (!repo) {
    return (
      <section className="main-panel empty">
        <p className="muted">{isLoading ? "Loading…" : "This repository is no longer available."}</p>
      </section>
    );
  }
  return <RepoView repo={repo} />;
});
