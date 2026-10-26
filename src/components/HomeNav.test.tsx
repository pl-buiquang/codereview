import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const listRepositories = vi.fn();
const listClaudeSessions = vi.fn();
const boardList = vi.fn();
const listInbox = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    listRepositories: () => listRepositories(),
    listClaudeSessions: () => listClaudeSessions(),
    boardList: () => boardList(),
    listInbox: () => listInbox(),
    listConversations: () => Promise.resolve({ items: [], total: 0, facets: { unread: 0 } }),
  },
}));

import { HomeNav } from "./HomeNav";
import { useUIStore } from "../store";
import { useSettingsStore } from "../lib/settings";

function renderNav(collapsed: boolean, onNavigate?: () => void) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<HomeNav collapsed={collapsed} onNavigate={onNavigate} />, { wrapper });
}

beforeEach(() => {
  vi.clearAllMocks();
  useUIStore.setState({
    tabs: [{ id: "home", kind: "home" }],
    activeTabId: "home",
    homeSection: "inbox",
    homeRepoId: null,
    openRepoIds: [2],
    sidebarCollapsed: false,
  });
  listRepositories.mockResolvedValue([
    { id: 1, local_path: "/p/one", remote_owner: "acme", remote_name: "one" },
    { id: 2, local_path: "/p/two", remote_owner: "acme", remote_name: "two" },
  ]);
  listClaudeSessions.mockResolvedValue({ sessions: [], card_sessions: [] });
  boardList.mockResolvedValue([]);
  listInbox.mockResolvedValue([]);
});

describe("HomeNav", () => {
  it("lists only open repos and shows one on click", async () => {
    const onNavigate = vi.fn();
    const user = userEvent.setup();
    renderNav(false, onNavigate);

    await user.click(await screen.findByText("two"));
    expect(screen.queryByText("one")).not.toBeInTheDocument();
    expect(useUIStore.getState().homeRepoId).toBe(2);
    expect(onNavigate).toHaveBeenCalled();
  });

  it("closes an open repo", async () => {
    const user = userEvent.setup();
    useUIStore.setState({ homeRepoId: 2 });
    renderNav(false);

    await user.click(await screen.findByLabelText("Close acme/two"));
    expect(useUIStore.getState().openRepoIds).toEqual([]);
    expect(useUIStore.getState().homeRepoId).toBeNull();
  });

  it("hides labels when collapsed but keeps tooltips", async () => {
    renderNav(true);

    expect(await screen.findByLabelText("acme/two")).toBeInTheDocument();
    expect(screen.queryByText("two")).not.toBeInTheDocument();
    expect(screen.queryByText("Inbox")).not.toBeInTheDocument();
    expect(screen.getByTitle("Inbox")).toBeInTheDocument();
  });

  it("shows the full repo name in a hover tooltip", async () => {
    const user = userEvent.setup();
    renderNav(false);

    await user.hover(await screen.findByText("two"));
    expect(await screen.findByRole("tooltip")).toHaveTextContent("acme/two");
    await user.unhover(screen.getByText("two"));
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("toggles the collapsed flag", async () => {
    const user = userEvent.setup();
    renderNav(false);

    await user.click(screen.getByLabelText("Collapse sidebar"));
    expect(useUIStore.getState().sidebarCollapsed).toBe(true);
  });

  it("badges Sessions only for live sessions waiting for input", async () => {
    const base = { live: true, status: "busy", waiting_for: null, session_id: "abcdef123456", name: "fixer" };
    listClaudeSessions.mockResolvedValue({
      sessions: [
        base,
        { ...base, session_id: "s2", waiting_for: "input needed" },
        { ...base, session_id: "s3", live: false, waiting_for: "input needed" },
      ],
      card_sessions: [],
    });
    renderNav(false);

    expect(await screen.findByTitle(/Waiting for input/)).toHaveTextContent("1");
  });

  it("badges Inbox with the New items not viewed yet", async () => {
    listInbox.mockResolvedValue([
      { id: "viewed", viewed_at: "2026-01-01T00:00:00Z", reasons: [] },
      { id: "unviewed", viewed_at: null, reasons: [] },
    ]);
    renderNav(false);

    expect(await screen.findByTitle("1 new item, 0 unread conversations")).toHaveTextContent("1");
  });

  it("badges Board with the cards that need my action", async () => {
    boardList.mockResolvedValue([
      {
        id: "c1", type: "pr", repo: "acme/back", number: 1, title: "t", state: "open", is_draft: false,
        review_decision: "APPROVED", is_mine: true, updated_at: new Date().toISOString(), lane: "submitted",
        meta: [], linked_issue_ids: ["i"], linked_pr_ids: [], archived_at: null, discarded_at: null,
      },
    ]);
    renderNav(false);

    expect(await screen.findByTitle("1 card needs your action")).toHaveTextContent("1");
  });

  it("leaves snoozed cards and disabled kinds out of the Board badge", async () => {
    const base = {
      type: "pr", repo: "acme/back", title: "t", state: "open", is_draft: false, is_mine: true,
      updated_at: new Date().toISOString(), lane: "submitted", meta: [], linked_issue_ids: ["i"],
      linked_pr_ids: [], archived_at: null, discarded_at: null,
    };
    boardList.mockResolvedValue([
      { ...base, id: "snoozed", number: 1, review_decision: "APPROVED" },
      { ...base, id: "ci", number: 2, review_decision: null, ci_status: "FAILURE" },
      { ...base, id: "ready", number: 3, review_decision: "APPROVED" },
    ]);
    useSettingsStore.getState().snoozeCard("snoozed", Date.now() + 3_600_000);
    renderNav(false);

    // "ci" is off by default and "snoozed" is snoozed: only "ready" counts.
    expect(await screen.findByTitle("1 card needs your action")).toHaveTextContent("1");
  });
});
