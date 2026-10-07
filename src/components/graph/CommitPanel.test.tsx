import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const success = vi.fn();
const error = vi.fn();
vi.mock("../../lib/toast", () => ({
  toast: { success: (m: string) => success(m), error: (m: string) => error(m) },
}));

import { CommitPanel } from "./CommitPanel";
import type { CommitDetail, WipStatus } from "../../lib/types";

const SHA = "abcdef1234567890abcdef1234567890abcdef12";
const P1 = "1111111aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const P2 = "2222222bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

const detail = (over: Partial<CommitDetail> = {}): CommitDetail => ({
  sha: SHA,
  parents: [P1, P2],
  subject: "Merge feature into main",
  body: "Longer explanation\n\nsecond paragraph",
  author_name: "Ada Lovelace",
  author_email: "ada@example.com",
  author_time: 1_700_000_000,
  committer_name: "Grace Hopper",
  committer_email: "grace@example.com",
  committer_time: 1_700_000_100,
  files: [
    { path: "src/a.ts", old_path: null, status: "A", additions: 10, deletions: 0 },
    { path: "src/b.ts", old_path: null, status: "M", additions: 3, deletions: 2 },
    { path: "src/c.ts", old_path: null, status: "M", additions: null, deletions: null },
    { path: "src/new.ts", old_path: "src/old.ts", status: "R", additions: 1, deletions: 1 },
    { path: "gone.txt", old_path: null, status: "D", additions: 0, deletions: 5 },
  ],
  ...over,
});

describe("CommitPanel (commit mode)", () => {
  beforeEach(() => {
    success.mockReset();
    error.mockReset();
  });

  it("renders the header fields", () => {
    render(<CommitPanel detail={detail()} onOpenFile={vi.fn()} onJumpToSha={vi.fn()} />);
    expect(screen.getByText("Merge feature into main")).toBeInTheDocument();
    expect(screen.getByText(/second paragraph/)).toBeInTheDocument();
    expect(screen.getByText("Ada Lovelace")).toBeInTheDocument();
    expect(screen.getByText("<ada@example.com>")).toBeInTheDocument();
    expect(screen.getByText("Grace Hopper")).toBeInTheDocument();
    expect(screen.getByText("<grace@example.com>")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: `Copy sha ${SHA}` })).toHaveTextContent("abcdef1");
    expect(screen.getByText("1111111")).toBeInTheDocument();
    expect(screen.getByText("2222222")).toBeInTheDocument();
    const counts = screen.getByTestId("cp-counts");
    expect(counts).toHaveTextContent("A 1");
    expect(counts).toHaveTextContent("M 2");
    expect(counts).toHaveTextContent("D 1");
    expect(counts).toHaveTextContent("R 1");
    expect(screen.getByText("src/old.ts → src/new.ts")).toBeInTheDocument();
    expect(screen.getByText("+10")).toBeInTheDocument();
  });

  it("calls onOpenFile with the clicked file", async () => {
    const onOpenFile = vi.fn();
    const d = detail();
    render(<CommitPanel detail={d} onOpenFile={onOpenFile} onJumpToSha={vi.fn()} />);
    await userEvent.click(screen.getByText("src/old.ts → src/new.ts"));
    expect(onOpenFile).toHaveBeenCalledWith(d.files[3]);
  });

  it("calls onJumpToSha when a parent is clicked", async () => {
    const onJumpToSha = vi.fn();
    render(<CommitPanel detail={detail()} onOpenFile={vi.fn()} onJumpToSha={onJumpToSha} />);
    await userEvent.click(screen.getByText("2222222"));
    expect(onJumpToSha).toHaveBeenCalledWith(P2);
  });

  it("copies the full sha and toasts", async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    render(<CommitPanel detail={detail()} onOpenFile={vi.fn()} onJumpToSha={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: `Copy sha ${SHA}` }));
    expect(writeText).toHaveBeenCalledWith(SHA);
    expect(success).toHaveBeenCalled();
  });

  it("labels a root commit", () => {
    render(
      <CommitPanel detail={detail({ parents: [] })} onOpenFile={vi.fn()} onJumpToSha={vi.fn()} />,
    );
    expect(screen.getByText(/root commit/)).toBeInTheDocument();
  });
});

describe("CommitPanel (WIP mode)", () => {
  const wip: WipStatus = {
    head_sha: SHA,
    staged: [{ path: "s1.ts", old_path: null, status: "M", additions: 1, deletions: 0 }],
    unstaged: [
      { path: "u1.ts", old_path: null, status: "M", additions: 2, deletions: 2 },
      { path: "u2.ts", old_path: null, status: "D", additions: 0, deletions: 4 },
    ],
    untracked: ["new1.txt", "new2.txt", "new3.txt"],
  };

  it("renders the three groups with counts", () => {
    render(<CommitPanel wip={wip} onOpenWipFile={vi.fn()} />);
    const staged = screen.getByRole("region", { name: "Staged" });
    const unstaged = screen.getByRole("region", { name: "Unstaged" });
    const untracked = screen.getByRole("region", { name: "Untracked" });
    expect(within(staged).getByText("1")).toBeInTheDocument();
    expect(within(staged).getByText("s1.ts")).toBeInTheDocument();
    expect(within(unstaged).getByText("2")).toBeInTheDocument();
    expect(within(unstaged).getByText("u2.ts")).toBeInTheDocument();
    expect(within(untracked).getByText("3")).toBeInTheDocument();
    expect(within(untracked).getByText("new3.txt")).toBeInTheDocument();
  });

  it("calls onOpenWipFile with path and kind", async () => {
    const onOpenWipFile = vi.fn();
    render(<CommitPanel wip={wip} onOpenWipFile={onOpenWipFile} />);
    await userEvent.click(screen.getByText("s1.ts"));
    await userEvent.click(screen.getByText("u1.ts"));
    await userEvent.click(screen.getByText("new2.txt"));
    expect(onOpenWipFile.mock.calls).toEqual([
      ["s1.ts", "staged"],
      ["u1.ts", "unstaged"],
      ["new2.txt", "untracked"],
    ]);
  });
});
