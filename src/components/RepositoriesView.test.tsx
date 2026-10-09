import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Mock the API/dialog layer used by the home panel.
const listRepositories = vi.fn();
const addRepository = vi.fn();
const removeRepository = vi.fn();
const linkLocalPath = vi.fn();
const autoLinkRepos = vi.fn();
const pickFolder = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    listRepositories: () => listRepositories(),
    addRepository: (p: string) => addRepository(p),
    removeRepository: (id: number) => removeRepository(id),
    linkLocalPath: (id: number, p: string) => linkLocalPath(id, p),
    autoLinkRepos: (paths: string[]) => autoLinkRepos(paths),
  },
  pickFolder: () => pickFolder(),
}));

// The remove flow now uses the in-app confirm dialog instead of window.confirm.
const confirmDialog = vi.fn();
vi.mock("../lib/confirm", () => ({ confirmDialog: (...a: unknown[]) => confirmDialog(...a) }));

import { RepositoriesView } from "./RepositoriesView";
import { useUIStore } from "../store";
import type { Repository } from "../lib/types";

const repo = (over: Partial<Repository> = {}): Repository => ({
  id: 1,
  local_path: "/home/me/projects/widget",
  remote_owner: null,
  remote_name: null,
  default_branch: "main",
  added_at: "2026-01-01",
  ...over,
});

function renderHome() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<RepositoriesView />, { wrapper });
}

beforeEach(() => {
  vi.clearAllMocks();
  useUIStore.setState({
    tabs: [{ id: "home", kind: "home" }],
    activeTabId: "home",
    homeRepoId: null,
    pinnedRepoIds: [],
  });
  listRepositories.mockResolvedValue([]);
});

describe("RepositoriesView", () => {
  it("shows the empty state when there are no repos", async () => {
    renderHome();
    expect(await screen.findByText(/No repositories yet/i)).toBeInTheDocument();
  });

  it("renders owner/name when a remote is known, else the folder name", async () => {
    listRepositories.mockResolvedValue([
      repo({ id: 1, remote_owner: "acme", remote_name: "widget" }),
      repo({ id: 2, local_path: "/home/me/localonly", remote_owner: null, remote_name: null }),
    ]);
    renderHome();

    expect(await screen.findByText("acme/widget")).toBeInTheDocument();
    expect(screen.getByText("localonly")).toBeInTheDocument();
  });

  it("opens the repo inside the home tab on click", async () => {
    const user = userEvent.setup();
    listRepositories.mockResolvedValue([repo({ id: 7, remote_owner: "a", remote_name: "b" })]);
    renderHome();

    await user.click(await screen.findByText("a/b"));
    const s = useUIStore.getState();
    expect(s.activeTabId).toBe("home");
    expect(s.homeRepoId).toBe(7);
  });

  it("pins and unpins a repo without opening it", async () => {
    const user = userEvent.setup();
    listRepositories.mockResolvedValue([repo({ id: 7, remote_owner: "a", remote_name: "b" })]);
    renderHome();

    await screen.findByText("a/b");
    await user.click(screen.getByTitle("Pin to sidebar"));
    expect(useUIStore.getState().pinnedRepoIds).toEqual([7]);
    expect(useUIStore.getState().homeRepoId).toBeNull();
    await user.click(screen.getByTitle("Unpin from sidebar"));
    expect(useUIStore.getState().pinnedRepoIds).toEqual([]);
  });

  it("adds a repo via the folder picker and opens it", async () => {
    const user = userEvent.setup();
    pickFolder.mockResolvedValue("/new/repo/path");
    addRepository.mockResolvedValue(repo({ id: 99 }));
    renderHome();

    await user.click(await screen.findByRole("button", { name: /Add repo/i }));

    await waitFor(() => expect(addRepository).toHaveBeenCalledWith("/new/repo/path"));
    await waitFor(() => expect(useUIStore.getState().homeRepoId).toBe(99));
  });

  it("does not call addRepository when the folder picker is cancelled", async () => {
    const user = userEvent.setup();
    pickFolder.mockResolvedValue(null);
    renderHome();

    await user.click(await screen.findByRole("button", { name: /Add repo/i }));
    await waitFor(() => expect(pickFolder).toHaveBeenCalled());
    expect(addRepository).not.toHaveBeenCalled();
  });

  it("removes a repo after the user confirms", async () => {
    const user = userEvent.setup();
    confirmDialog.mockResolvedValue(true);
    removeRepository.mockResolvedValue(undefined);
    listRepositories.mockResolvedValue([repo({ id: 5, remote_owner: "a", remote_name: "b" })]);
    renderHome();

    await screen.findByText("a/b");
    await user.click(screen.getByTitle("Remove repository"));

    await waitFor(() => expect(removeRepository).toHaveBeenCalledWith(5));
  });

  it("does not remove when the user cancels the confirm dialog", async () => {
    const user = userEvent.setup();
    confirmDialog.mockResolvedValue(false);
    listRepositories.mockResolvedValue([repo({ id: 5, remote_owner: "a", remote_name: "b" })]);
    renderHome();

    await screen.findByText("a/b");
    await user.click(screen.getByTitle("Remove repository"));
    await waitFor(() => expect(confirmDialog).toHaveBeenCalled());
    expect(removeRepository).not.toHaveBeenCalled();
  });
});
