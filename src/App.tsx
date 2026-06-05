import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { TabBar } from "./components/TabBar";
import { HomePanel } from "./components/HomePanel";
import { RepoView } from "./components/RepoView";
import { ReviewView } from "./components/ReviewView";
import { Toaster } from "./components/Toaster";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { SettingsView } from "./components/SettingsView";
import { api } from "./lib/api";
import { useApplySettings } from "./lib/useApplySettings";
import { useUIStore, type Tab } from "./store";
import type { Repository } from "./lib/types";

function App() {
  useApplySettings();
  const tabs = useUIStore((s) => s.tabs);
  const activeTabId = useUIStore((s) => s.activeTabId);
  const closeTab = useUIStore((s) => s.closeTab);

  const reposQuery = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });
  const repos = reposQuery.data;

  // Drop repo/review tabs whose repository was removed in a previous session.
  useEffect(() => {
    if (!repos) return;
    const ids = new Set(repos.map((r) => r.id));
    for (const tab of tabs) {
      if (
        (tab.kind === "repo" || tab.kind === "review") &&
        tab.repoId != null &&
        !ids.has(tab.repoId)
      ) {
        closeTab(tab.id);
      }
    }
  }, [repos, tabs, closeTab]);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? tabs[0];

  return (
    <div className="app-shell">
      <TabBar />
      <div className="tab-content">{renderTab(activeTab, repos)}</div>
      <Toaster />
      <ConfirmDialog />
    </div>
  );
}

function renderTab(tab: Tab, repos: Repository[] | undefined) {
  if (tab.kind === "home") return <HomePanel />;
  if (tab.kind === "settings") return <SettingsView />;
  if (tab.kind === "review" && tab.reviewId != null) {
    return <ReviewView key={tab.reviewId} reviewId={tab.reviewId} />;
  }

  const repo = repos?.find((r) => r.id === tab.repoId);
  if (!repo) {
    return (
      <section className="main-panel empty">
        <p className="muted">This repository is no longer available.</p>
      </section>
    );
  }
  return <RepoView repo={repo} />;
}

export default App;
