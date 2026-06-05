import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useUIStore, type Tab } from "../store";
import { repoLabel } from "./HomePanel";
import type { Repository } from "../lib/types";

function HomeIcon() {
  return (
    <svg
      className="home-icon"
      width="15"
      height="15"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-label="Home"
    >
      <path d="M3 10.5 12 3l9 7.5" />
      <path d="M5 9.5V21h14V9.5" />
    </svg>
  );
}

function TabItem({ tab, repos }: { tab: Tab; repos: Repository[] }) {
  const activeTabId = useUIStore((s) => s.activeTabId);
  const setActiveTab = useUIStore((s) => s.setActiveTab);
  const closeTab = useUIStore((s) => s.closeTab);

  const reviewQuery = useQuery({
    queryKey: ["review", tab.reviewId],
    queryFn: () => api.getReview(tab.reviewId!),
    enabled: tab.kind === "review" && tab.reviewId != null,
  });

  let label: string;
  if (tab.kind === "home") {
    label = "Home";
  } else if (tab.kind === "settings") {
    label = "⚙ Settings";
  } else if (tab.kind === "review") {
    label = reviewQuery.data?.target.title ?? `Review #${tab.reviewId}`;
  } else {
    const repo = repos.find((r) => r.id === tab.repoId);
    label = repo ? repoLabel(repo) : `repo #${tab.repoId}`;
  }

  return (
    <div
      className={`tab tab-${tab.kind} ${tab.id === activeTabId ? "active" : ""}`}
      onClick={() => setActiveTab(tab.id)}
      title={label}
    >
      {tab.kind === "home" ? <HomeIcon /> : <span className="tab-label">{label}</span>}
      {tab.kind !== "home" && (
        <button
          className="tab-close"
          title="Close tab"
          onClick={(e) => {
            e.stopPropagation();
            closeTab(tab.id);
          }}
        >
          ✕
        </button>
      )}
    </div>
  );
}

export function TabBar() {
  const tabs = useUIStore((s) => s.tabs);

  const reposQuery = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });
  const repos = reposQuery.data ?? [];

  return (
    <nav className="tab-bar">
      {tabs.map((tab) => (
        <TabItem key={tab.id} tab={tab} repos={repos} />
      ))}
    </nav>
  );
}
