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

  return (
    <div className="dashboard">
      <HomeNav collapsed={collapsed} />
      <div className="dashboard-main">
        {homeRepoId != null ? (
          <RepoPane key={homeRepoId} repoId={homeRepoId} />
        ) : (
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

function RepoPane({ repoId }: { repoId: number }) {
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
}
