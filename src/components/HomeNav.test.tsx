import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const listRepositories = vi.fn();
vi.mock("../lib/api", () => ({ api: { listRepositories: () => listRepositories() } }));

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

    await user.click(await screen.findByText("acme/two"));
    expect(screen.queryByText("acme/one")).not.toBeInTheDocument();
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

    expect(await screen.findByTitle("acme/two")).toBeInTheDocument();
    expect(screen.queryByText("acme/two")).not.toBeInTheDocument();
    expect(screen.queryByText("Inbox")).not.toBeInTheDocument();
    expect(screen.getByTitle("Inbox")).toBeInTheDocument();
  });

  it("toggles the collapsed flag", async () => {
    const user = userEvent.setup();
    renderNav(false);

    await user.click(screen.getByLabelText("Collapse sidebar"));
    expect(useUIStore.getState().sidebarCollapsed).toBe(true);
  });
});
