import { memo, useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { TabBar } from "./components/TabBar";
import { DashboardPanel } from "./components/DashboardPanel";
import { HomeFlyout } from "./components/HomeFlyout";
import { ReviewView } from "./components/ReviewView";
import { Toaster } from "./components/Toaster";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { CommandPalette } from "./components/CommandPalette";
import { UpdateBanner } from "./components/UpdateBanner";
import { SettingsView } from "./components/SettingsView";
import { api } from "./lib/api";
import { toast } from "./lib/toast";
import { isCloseTabShortcut, isCommandPaletteShortcut } from "./lib/keyboard";
import { toggleCommandPalette } from "./lib/commandPalette";
import { useApplySettings } from "./lib/useApplySettings";
import { useDeepLinkListener } from "./lib/deepLink";
import { useUIStore, type Tab } from "./store";

const CLOSE_ACTIVE_TAB_EVENT = "close-active-tab";

function closeActiveTab() {
  const { activeTabId, closeTab } = useUIStore.getState();
  closeTab(activeTabId);
}

function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

function registerCloseTabKeydown() {
  const onKey = (e: KeyboardEvent) => {
    if (!isCloseTabShortcut(e)) return;
    e.preventDefault();
    closeActiveTab();
  };
  window.addEventListener("keydown", onKey);
  return () => window.removeEventListener("keydown", onKey);
}

function App() {
  useApplySettings();
  useDeepLinkListener();
  const tabs = useUIStore((s) => s.tabs);
  const closeTab = useUIStore((s) => s.closeTab);
  const openRepoIds = useUIStore((s) => s.openRepoIds);
  const homeRepoId = useUIStore((s) => s.homeRepoId);
  const forgetRepo = useUIStore((s) => s.forgetRepo);

  useEffect(() => {
    if (isTauriRuntime()) {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      let unlistenFallback: (() => void) | undefined;
      listen(CLOSE_ACTIVE_TAB_EVENT, closeActiveTab)
        .then((dispose) => {
          if (disposed) dispose();
          else unlisten = dispose;
        })
        .catch((e) => {
          console.warn("Could not register close-tab menu shortcut", e);
          if (!disposed) unlistenFallback = registerCloseTabKeydown();
        });
      return () => {
        disposed = true;
        unlisten?.();
        unlistenFallback?.();
      };
    }

    return registerCloseTabKeydown();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isCommandPaletteShortcut(e)) return;
      e.preventDefault();
      toggleCommandPalette();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const reposQuery = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });
  const repos = reposQuery.data;
  const reposFetching = reposQuery.isFetching;

  // Show a one-time toast when the app installs/updates the bundled cr CLI.
  useEffect(() => {
    if (!isTauriRuntime()) return;
    api.crInstallNote().then((note) => {
      if (note) toast.success(note);
    });
  }, []);

  // Drop review tabs and open repos whose repository was removed in a previous session.
  // Only act on a settled list — acting mid-fetch would race a just-added repo
  // (whose tab is opened optimistically before the refetch lands).
  useEffect(() => {
    if (!repos || reposFetching) return;
    const ids = new Set(repos.map((r) => r.id));
    for (const tab of tabs) {
      if (tab.kind === "review" && tab.repoId != null && !ids.has(tab.repoId)) {
        closeTab(tab.id);
      }
    }
    for (const id of [...openRepoIds, homeRepoId]) {
      if (id != null && !ids.has(id)) forgetRepo(id);
    }
  }, [repos, reposFetching, tabs, closeTab, openRepoIds, homeRepoId, forgetRepo]);

  return (
    <div className="app-shell">
      <UpdateBanner />
      <TabBar />
      <TabPanes />
      <HomeFlyout />
      <Toaster />
      <ConfirmDialog />
      <CommandPalette />
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
  if (tab.kind === "settings") return <SettingsView />;
  if (tab.kind === "review" && tab.reviewId != null) {
    return <ReviewView key={tab.reviewId} reviewId={tab.reviewId} />;
  }
  return <DashboardPanel />;
});

export default App;
