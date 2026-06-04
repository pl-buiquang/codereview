import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Mock the API/dialog layer used by the sidebar.
const listRepositories = vi.fn();
const addRepository = vi.fn();
const removeRepository = vi.fn();
const pickFolder = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    listRepositories: () => listRepositories(),
    addRepository: (p: string) => addRepository(p),
    removeRepository: (id: number) => removeRepository(id),
  },
  pickFolder: () => pickFolder(),
}));

import { RepoSidebar } from "./RepoSidebar";
import { useUIStore } from "../store";
import type { Repository } from "../lib/types";

const repo = (over: Partial<Repository> = {}): Repository => ({
  id: 1,
  path: "/home/me/projects/widget",
  remote_owner: null,
  remote_name: null,
  default_branch: "main",
  added_at: "2026-01-01",
  ...over,
});

function renderSidebar() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<RepoSidebar />, { wrapper });
}

beforeEach(() => {
  vi.clearAllMocks();
  useUIStore.setState({ activeRepoId: null, activeReviewId: null });
  listRepositories.mockResolvedValue([]);
});

describe("RepoSidebar", () => {
  it("shows the empty state when there are no repos", async () => {
    renderSidebar();
    expect(await screen.findByText(/No repositories yet/i)).toBeInTheDocument();
  });

  it("renders owner/name when a remote is known, else the folder name", async () => {
    listRepositories.mockResolvedValue([
      repo({ id: 1, remote_owner: "acme", remote_name: "widget" }),
      repo({ id: 2, path: "/home/me/localonly", remote_owner: null, remote_name: null }),
    ]);
    renderSidebar();

    expect(await screen.findByText("acme/widget")).toBeInTheDocument();
    expect(screen.getByText("localonly")).toBeInTheDocument();
  });

  it("selects a repo on click", async () => {
    const user = userEvent.setup();
    listRepositories.mockResolvedValue([repo({ id: 7, remote_owner: "a", remote_name: "b" })]);
    renderSidebar();

    await user.click(await screen.findByText("a/b"));
    expect(useUIStore.getState().activeRepoId).toBe(7);
  });

  it("adds a repo via the folder picker and selects it", async () => {
    const user = userEvent.setup();
    pickFolder.mockResolvedValue("/new/repo/path");
    addRepository.mockResolvedValue(repo({ id: 99 }));
    renderSidebar();

    await user.click(await screen.findByRole("button", { name: /Add repo/i }));

    await waitFor(() => expect(addRepository).toHaveBeenCalledWith("/new/repo/path"));
    await waitFor(() => expect(useUIStore.getState().activeRepoId).toBe(99));
  });

  it("does not call addRepository when the folder picker is cancelled", async () => {
    const user = userEvent.setup();
    pickFolder.mockResolvedValue(null);
    renderSidebar();

    await user.click(await screen.findByRole("button", { name: /Add repo/i }));
    await waitFor(() => expect(pickFolder).toHaveBeenCalled());
    expect(addRepository).not.toHaveBeenCalled();
  });

  it("removes a repo after the user confirms", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    removeRepository.mockResolvedValue(undefined);
    listRepositories.mockResolvedValue([repo({ id: 5, remote_owner: "a", remote_name: "b" })]);
    renderSidebar();

    await screen.findByText("a/b");
    await user.click(screen.getByTitle("Remove repository"));

    await waitFor(() => expect(removeRepository).toHaveBeenCalledWith(5));
  });

  it("does not remove when the user cancels the confirm dialog", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "confirm").mockReturnValue(false);
    listRepositories.mockResolvedValue([repo({ id: 5, remote_owner: "a", remote_name: "b" })]);
    renderSidebar();

    await screen.findByText("a/b");
    await user.click(screen.getByTitle("Remove repository"));
    expect(removeRepository).not.toHaveBeenCalled();
  });
});
