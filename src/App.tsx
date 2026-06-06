import { memo, useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { TabBar } from "./components/TabBar";
import { DashboardPanel } from "./components/DashboardPanel";
import { RepoView } from "./components/RepoView";
import { ReviewView } from "./components/ReviewView";
import { Toaster } from "./components/Toaster";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { SettingsView } from "./components/SettingsView";
import { api } from "./lib/api";
import { useApplySettings } from "./lib/useApplySettings";
import { useUIStore, type Tab } from "./store";

function App() {
  useApplySettings();
  const tabs = useUIStore((s) => s.tabs);
  const closeTab = useUIStore((s) => s.closeTab);

  const reposQuery = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });
  const repos = reposQuery.data;
  const reposFetching = reposQuery.isFetching;

  // Drop repo/review tabs whose repository was removed in a previous session.
  // Only act on a settled list — acting mid-fetch would race a just-added repo
  // (whose tab is opened optimistically before the refetch lands).
  useEffect(() => {
    if (!repos || reposFetching) return;
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
  }, [repos, reposFetching, tabs, closeTab]);

  return (
    <div className="app-shell">
      <TabBar />
      <TabPanes />
      <Toaster />
      <ConfirmDialog />
    </div>
  );
}

// Keep every tab mounted (hidden when inactive) so switching preserves each
// tab's scroll position and component state. Subscribing to `tabs` (not the
// active id) keeps this list from re-rendering on a switch; only the two panes
// whose `active` prop flips re-render, and their heavy content is memoized.
function TabPanes() {
  const tabs = useUIStore((s) => s.tabs);
  const activeTabId = useUIStore((s) => s.activeTabId);
  const activeId = tabs.some((t) => t.id === activeTabId) ? activeTabId : tabs[0]?.id;
  // Render panes in a stable id-sorted order, decoupled from the tab-bar order.
  // Only `display` decides which pane shows, so the order here is invisible — but
  // keeping it stable means reordering tabs never moves these heavy mounted diff
  // subtrees in the DOM, which is what made a drag-drop feel laggy.
  const panes = [...tabs].sort((a, b) => a.id.localeCompare(b.id));
  return (
    <div className="tab-content">
      {panes.map((tab) => (
        <TabPane key={tab.id} tab={tab} active={tab.id === activeId} />
      ))}
    </div>
  );
}

// Visibility (the `active` flag) is split from content so a tab switch only
// toggles this wrapper's `display` — TabContent, memoized on the stable `tab`
// object, never re-renders, so the diff subtree is left untouched.
const TabPane = memo(function TabPane({ tab, active }: { tab: Tab; active: boolean }) {
  return (
    <div className="tab-pane" style={active ? undefined : { display: "none" }}>
      <TabContent tab={tab} />
    </div>
  );
});

const TabContent = memo(function TabContent({ tab }: { tab: Tab }) {
  if (tab.kind === "home") return <DashboardPanel />;
  if (tab.kind === "settings") return <SettingsView />;
  if (tab.kind === "review" && tab.reviewId != null) {
    return <ReviewView key={tab.reviewId} reviewId={tab.reviewId} />;
  }
  return <RepoPane repoId={tab.repoId} />;
});

// Looks up its own repo rather than taking it from App, so the pane doesn't
// depend on App's repos query (whose identity changes on every refetch would
// otherwise defeat TabContent's memo).
function RepoPane({ repoId }: { repoId?: number }) {
  const { data: repos } = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });
  const repo = repos?.find((r) => r.id === repoId);
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
