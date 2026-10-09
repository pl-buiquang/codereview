import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useUIStore, type Tab } from "../store";
import { toast } from "../lib/toast";
import { Icon, type IconName } from "./icons";
import { flyoutHover } from "./HomeFlyout";

function tabMagicLink(tab: Tab): string | null {
  if (tab.kind === "review" && tab.reviewId != null) {
    return `codereview://review/${tab.reviewId}`;
  }
  return null;
}

/** The type glyph shown on an inactive document tab (active tabs show the dot). */
function tabIcon(kind: Tab["kind"]): IconName {
  if (kind === "review") return "review";
  if (kind === "settings") return "gear";
  return "file";
}

/** The display label for a tab. Review titles come from the (cached) review
 *  query, so this is a hook shared by the tab strip and the overflow menu. */
function useTabLabel(tab: Tab): string {
  const reviewQuery = useQuery({
    queryKey: ["review", tab.reviewId],
    queryFn: () => api.getReview(tab.reviewId!),
    enabled: tab.kind === "review" && tab.reviewId != null,
  });

  if (tab.kind === "home") return "Home";
  if (tab.kind === "settings") return "Settings";
  return reviewQuery.data?.target.title ?? `Review #${tab.reviewId}`;
}

function TabContextMenu({
  tab,
  anchorRect,
  onClose,
}: {
  tab: Tab;
  anchorRect: DOMRect;
  onClose: () => void;
}) {
  const closeTab = useUIStore((s) => s.closeTab);
  const ref = useRef<HTMLDivElement>(null);
  const link = tabMagicLink(tab);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  return createPortal(
    <div
      className="tab-context-menu"
      ref={ref}
      style={{ position: "fixed", top: anchorRect.bottom + 2, left: anchorRect.left }}
      onClick={(e) => e.stopPropagation()}
    >
      {tab.kind === "review" && tab.reviewId != null && (
        <>
          <div className="tab-context-id">Review #{tab.reviewId}</div>
          <div className="tab-context-sep" />
        </>
      )}
      {link && (
        <button
          className="tab-context-item"
          onClick={() => {
            navigator.clipboard.writeText(link).then(() => {
              toast.success("Link copied");
            });
            onClose();
          }}
        >
          <Icon name="link" size={13} />
          Copy link
        </button>
      )}
      {tab.kind !== "home" && (
        <>
          {link && <div className="tab-context-sep" />}
          <button
            className="tab-context-item tab-context-item--danger"
            onClick={() => {
              closeTab(tab.id);
              onClose();
            }}
          >
            <Icon name="x" size={13} />
            Close tab
          </button>
        </>
      )}
    </div>,
    document.body,
  );
}

function TabItem({ tab }: { tab: Tab }) {
  const activeTabId = useUIStore((s) => s.activeTabId);
  const setActiveTab = useUIStore((s) => s.setActiveTab);
  const closeTab = useUIStore((s) => s.closeTab);
  const moveTab = useUIStore((s) => s.moveTab);
  const [dragOver, setDragOver] = useState(false);
  const [anchorRect, setAnchorRect] = useState<DOMRect | null>(null);
  const tabRef = useRef<HTMLDivElement>(null);

  // The home tab is pinned: it can't be dragged or accept a drop before it.
  const draggable = tab.kind !== "home";
  const isActive = tab.id === activeTabId;
  const label = useTabLabel(tab);

  // Only show a context menu if there's something to offer.
  const hasContextMenu = tabMagicLink(tab) !== null || tab.kind !== "home";

  return (
    <div
      ref={tabRef}
      className={`tab tab-${tab.kind} ${isActive ? "active" : ""} ${
        dragOver ? "drag-over" : ""
      }`}
      onClick={() => setActiveTab(tab.id)}
      onMouseEnter={tab.kind === "home" ? flyoutHover.enter : undefined}
      onMouseLeave={tab.kind === "home" ? flyoutHover.leave : undefined}
      onContextMenu={(e) => {
        if (!hasContextMenu) return;
        e.preventDefault();
        setAnchorRect(tabRef.current?.getBoundingClientRect() ?? null);
      }}
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
      title={tab.kind === "review" && tab.reviewId != null ? `${label} (#${tab.reviewId})` : label}
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
      {tab.kind === "home" ? (
        <Icon name="home" size={15} className="home-icon" />
      ) : (
        <>
          {isActive ? (
            <span className="tab-dot" />
          ) : (
            <Icon name={tabIcon(tab.kind)} size={12} className="tab-icon" />
          )}
          <span className="tab-label">{label}</span>
          <button
            className="tab-close"
            title="Close tab"
            onClick={(e) => {
              e.stopPropagation();
              closeTab(tab.id);
            }}
          >
            <Icon name="x" size={10} />
          </button>
        </>
      )}
      {anchorRect && (
        <TabContextMenu tab={tab} anchorRect={anchorRect} onClose={() => setAnchorRect(null)} />
      )}
    </div>
  );
}

function OverflowRow({
  tab,
  onPick,
}: {
  tab: Tab;
  onPick: () => void;
}) {
  const activeTabId = useUIStore((s) => s.activeTabId);
  const setActiveTab = useUIStore((s) => s.setActiveTab);
  const closeTab = useUIStore((s) => s.closeTab);
  const label = useTabLabel(tab);

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
          <Icon name="x" size={10} />
        </button>
      )}
    </div>
  );
}

function TabOverflowMenu({ tabs }: { tabs: Tab[] }) {
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
        <Icon name="chev" size={14} />
      </button>
      {open && (
        <div className="tab-overflow-menu">
          {tabs.map((tab) => (
            <OverflowRow key={tab.id} tab={tab} onPick={() => setOpen(false)} />
          ))}
        </div>
      )}
    </div>
  );
}

export function TabBar() {
  const tabs = useUIStore((s) => s.tabs);

  return (
    <nav className="tab-bar">
      <div className="tab-bar-tabs">
        {tabs.map((tab) => (
          <TabItem key={tab.id} tab={tab} />
        ))}
      </div>
      {tabs.length > 1 && <TabOverflowMenu tabs={tabs} />}
    </nav>
  );
}
