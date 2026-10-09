import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { create } from "zustand";
import { useUIStore } from "../store";
import { HomeNav } from "./HomeNav";

const OPEN_DELAY_MS = 120;
const CLOSE_DELAY_MS = 250;

let openTimer: ReturnType<typeof setTimeout> | undefined;
let closeTimer: ReturnType<typeof setTimeout> | undefined;

export const useFlyoutStore = create<{ open: boolean }>(() => ({ open: false }));

function clearTimers() {
  clearTimeout(openTimer);
  clearTimeout(closeTimer);
}

/** Hover-intent handlers for anything that should reveal the home menu flyout. */
export const flyoutHover = {
  enter() {
    clearTimers();
    if (useFlyoutStore.getState().open) return;
    openTimer = setTimeout(() => useFlyoutStore.setState({ open: true }), OPEN_DELAY_MS);
  },
  leave() {
    clearTimers();
    closeTimer = setTimeout(() => useFlyoutStore.setState({ open: false }), CLOSE_DELAY_MS);
  },
  close() {
    clearTimers();
    useFlyoutStore.setState({ open: false });
  },
};

export function HomeFlyout() {
  const open = useFlyoutStore((s) => s.open);
  // The docked sidebar already shows everything when home is active and expanded.
  const redundant = useUIStore((s) => s.activeTabId === "home" && !s.sidebarCollapsed);
  const ref = useRef<HTMLDivElement>(null);
  const visible = open && !redundant;
  const top = document.querySelector(".tab-bar")?.getBoundingClientRect().bottom ?? 0;

  useEffect(() => {
    if (!visible) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) flyoutHover.close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") flyoutHover.close();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [visible]);

  return createPortal(
    <>
      {!redundant && (
        <div
          className="home-edge-trigger"
          style={{ top }}
          onMouseEnter={flyoutHover.enter}
          onMouseLeave={flyoutHover.leave}
        />
      )}
      {visible && (
        <div
          ref={ref}
          className="home-flyout"
          style={{ top }}
          onMouseEnter={flyoutHover.enter}
          onMouseLeave={flyoutHover.leave}
        >
          <HomeNav collapsed={false} showToggle={false} onNavigate={flyoutHover.close} />
        </div>
      )}
    </>,
    document.body,
  );
}
