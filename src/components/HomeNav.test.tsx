import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const listRepositories = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    listRepositories: () => listRepositories(),
    listInbox: () => Promise.resolve([]),
    inboxMeta: () => Promise.resolve({ newSeenAt: "2000-01-01T00:00:00Z" }),
    listConversations: () => Promise.resolve({ items: [], total: 0, facets: { unread: 0 } }),
  },
}));

import { HomeNav } from "./HomeNav";
import { useUIStore } from "../store";

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
});
