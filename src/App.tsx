import { useQuery } from "@tanstack/react-query";
import { RepoSidebar } from "./components/RepoSidebar";
import { RepoView } from "./components/RepoView";
import { ReviewView } from "./components/ReviewView";
import { Toaster } from "./components/Toaster";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { SettingsView } from "./components/SettingsView";
import { api } from "./lib/api";
import { useApplySettings } from "./lib/useApplySettings";
import { useUIStore } from "./store";

function App() {
  useApplySettings();
  const activeRepoId = useUIStore((s) => s.activeRepoId);
  const activeReviewId = useUIStore((s) => s.activeReviewId);
  const settingsOpen = useUIStore((s) => s.settingsOpen);
  const reposQuery = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });
  const activeRepo = reposQuery.data?.find((r) => r.id === activeRepoId) ?? null;

  let main;
  if (settingsOpen) {
    main = <SettingsView />;
  } else if (activeReviewId != null) {
    main = <ReviewView reviewId={activeReviewId} />;
  } else if (activeRepo) {
    main = <RepoView repo={activeRepo} />;
  } else {
    main = (
      <section className="main-panel empty">
        <p className="muted">Select or add a repository to begin.</p>
      </section>
    );
  }

  return (
    <div className="layout">
      <RepoSidebar />
      {main}
      <Toaster />
      <ConfirmDialog />
    </div>
  );
}

export default App;
