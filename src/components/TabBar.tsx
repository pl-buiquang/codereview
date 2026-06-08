import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useUIStore, type Tab } from "../store";
import { repoLabel } from "../lib/repoLabel";
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

/** The display label for a tab. Review titles come from the (cached) review
 *  query, so this is a hook shared by the tab strip and the overflow menu. */
function useTabLabel(tab: Tab, repos: Repository[]): string {
  const reviewQuery = useQuery({
    queryKey: ["review", tab.reviewId],
    queryFn: () => api.getReview(tab.reviewId!),
    enabled: tab.kind === "review" && tab.reviewId != null,
  });

  if (tab.kind === "home") return "Home";
  if (tab.kind === "settings") return "⚙ Settings";
  if (tab.kind === "review") return reviewQuery.data?.target.title ?? `Review #${tab.reviewId}`;
  const repo = repos.find((r) => r.id === tab.repoId);
  return repo ? repoLabel(repo) : `repo #${tab.repoId}`;
}

function TabItem({ tab, repos }: { tab: Tab; repos: Repository[] }) {
  const activeTabId = useUIStore((s) => s.activeTabId);
  const setActiveTab = useUIStore((s) => s.setActiveTab);
  const closeTab = useUIStore((s) => s.closeTab);
  const moveTab = useUIStore((s) => s.moveTab);
  const [dragOver, setDragOver] = useState(false);

  // The home tab is pinned: it can't be dragged or accept a drop before it.
  const draggable = tab.kind !== "home";
  const label = useTabLabel(tab, repos);

  return (
    <div
      className={`tab tab-${tab.kind} ${tab.id === activeTabId ? "active" : ""} ${
        dragOver ? "drag-over" : ""
      }`}
      onClick={() => setActiveTab(tab.id)}
      onAuxClick={(e) => {
        if (e.button === 1 && tab.kind !== "home") {
          e.preventDefault();
          closeTab(tab.id);
        }
      }}
      onMouseDown={(e) => {
        // Suppress the middle-click autoscroll cursor.
        if (e.button === 1) e.preventDefault();
      }}
      title={label}
      draggable={draggable}
      onDragStart={(e) => {
        e.dataTransfer.setData("text/plain", tab.id);
        e.dataTransfer.effectAllowed = "move";
      }}
      onDragOver={(e) => {
        if (tab.kind === "home") return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragOver(false);
        const fromId = e.dataTransfer.getData("text/plain");
        if (fromId) moveTab(fromId, tab.id);
      }}
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

function OverflowRow({
  tab,
  repos,
  onPick,
}: {
  tab: Tab;
  repos: Repository[];
  onPick: () => void;
}) {
  const activeTabId = useUIStore((s) => s.activeTabId);
  const setActiveTab = useUIStore((s) => s.setActiveTab);
  const closeTab = useUIStore((s) => s.closeTab);
  const label = useTabLabel(tab, repos);

  return (
    <div
      className={`tab-overflow-row ${tab.id === activeTabId ? "active" : ""}`}
      title={label}
      onClick={() => {
        setActiveTab(tab.id);
        onPick();
      }}
    >
      <span className="tab-overflow-label">{label}</span>
      {tab.kind !== "home" && (
        <button
          className="tab-overflow-close"
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

function TabOverflowMenu({ tabs, repos }: { tabs: Tab[]; repos: Repository[] }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  return (
    <div className="tab-overflow" ref={ref}>
      <button
        className="tab-overflow-btn"
        title="All tabs"
        onClick={() => setOpen((o) => !o)}
      >
        ⋯
      </button>
      {open && (
        <div className="tab-overflow-menu">
          {tabs.map((tab) => (
            <OverflowRow key={tab.id} tab={tab} repos={repos} onPick={() => setOpen(false)} />
          ))}
        </div>
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
      <div className="tab-bar-tabs">
        {tabs.map((tab) => (
          <TabItem key={tab.id} tab={tab} repos={repos} />
        ))}
      </div>
      {tabs.length > 1 && <TabOverflowMenu tabs={tabs} repos={repos} />}
    </nav>
  );
}
