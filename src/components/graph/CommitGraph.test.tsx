import { createRef } from "react";
import { describe, it, expect, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import {
  CommitGraph,
  ROW_HEIGHT,
  WIP_SELECTION,
  type CommitGraphHandle,
  type CommitGraphProps,
} from "./CommitGraph";
import type { GraphRow as FixtureRow } from "../../lib/graphLayout";
import type { GraphCommit, RefInfo, WorktreeInfo } from "../../lib/types";

function linearFixture(n: number): { commits: GraphCommit[]; rows: FixtureRow[] } {
  const sha = (i: number) => `c${String(i).padStart(39, "0")}`;
  const commits: GraphCommit[] = Array.from({ length: n }, (_, i) => ({
    sha: sha(i),
    parents: i + 1 < n ? [sha(i + 1)] : [],
    subject: `subject ${i}`,
    body_preview: i % 2 === 0 ? `body ${i}` : "",
    author_name: "dev",
    author_email: "dev@example.com",
    author_time: 1_700_000_000 - i,
    committer_time: 1_700_000_000 - i,
  }));
  const rows: FixtureRow[] = commits.map((c, i) => ({
    sha: c.sha,
    lane: 0,
    color: 0,
    edgesUp: i === 0 ? [] : [{ fromLane: 0, toLane: 0, color: 0 }],
    edgesDown: i + 1 < n ? [{ fromLane: 0, toLane: 0, color: 0 }] : [],
  }));
  return { commits, rows };
}

function setup(overrides: Partial<CommitGraphProps> = {}, n = 5) {
  const { commits, rows } = linearFixture(n);
  const props: CommitGraphProps = {
    commits,
    rows,
    maxLanes: 1,
    refs: [],
    worktrees: [],
    headSha: commits[0]?.sha ?? null,
    wip: null,
    selectedSha: null,
    onSelect: vi.fn(),
    onEndReached: vi.fn(),
    onRefContextMenu: vi.fn(),
    ...overrides,
  };
  const handle = createRef<CommitGraphHandle>();
  const utils = render(<CommitGraph ref={handle} {...props} />);
  return { ...utils, props, handle, commits };
}

const commitRows = (container: HTMLElement) => container.querySelectorAll(".cg-row[data-sha]");

describe("CommitGraph", () => {
  it("uses the author's avatar when known and an identicon otherwise", () => {
    const { container, rerender, props, handle } = setup();
    const href = () => container.querySelector(".cg-row[data-sha] .cg-avatar")?.getAttribute("href") ?? "";
    expect(href()).toMatch(/^data:image\/svg\+xml/);
    rerender(
      <CommitGraph ref={handle} {...props} avatars={new Map([["dev@example.com", "data:image/png;base64,AAAA"]])} />,
    );
    expect(href()).toBe("data:image/png;base64,AAAA");
  });

  it("renders only a window of rows for a long list", () => {
    const { container } = setup({}, 2000);
    const rendered = commitRows(container).length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(100);
    expect(screen.getByTestId("commit-graph")).toHaveAttribute("aria-rowcount", "2000");
    expect(container.querySelector(".cg-canvas")).toHaveStyle({ height: `${2000 * ROW_HEIGHT}px` });
  });

  it("shifts the window when scrolled", () => {
    const { container, commits } = setup({}, 2000);
    const scroller = screen.getByTestId("commit-graph");
    act(() => {
      scroller.scrollTop = 1000 * ROW_HEIGHT;
      fireEvent.scroll(scroller);
    });
    expect(container.querySelector(`[data-sha="${commits[0].sha}"]`)).toBeNull();
    expect(container.querySelector(`[data-sha="${commits[1000].sha}"]`)).not.toBeNull();
  });

  it("renders subject and body preview without author/sha columns", () => {
    setup();
    expect(screen.getByText("subject 0")).toBeInTheDocument();
    expect(screen.getByText("body 0")).toBeInTheDocument();
    expect(screen.queryByText("dev")).toBeNull();
  });

  it("shows the WIP row above HEAD when wip is present", () => {
    const { container, commits } = setup({ headSha: null, wip: { count: 3 } });
    expect(screen.getByText("// WIP +3")).toBeInTheDocument();
    const all = Array.from(container.querySelectorAll(".cg-row"));
    expect(all[0]).toHaveAttribute("data-testid", "cg-wip-row");
    expect(all[1]).toHaveAttribute("data-sha", commits[0].sha);
  });

  it("places the WIP row directly above a non-first HEAD", () => {
    const { commits } = linearFixture(5);
    const { container } = setup({ headSha: commits[2].sha, wip: { count: 1 } });
    const all = Array.from(container.querySelectorAll(".cg-row"));
    const wipIdx = all.findIndex((el) => el.getAttribute("data-testid") === "cg-wip-row");
    expect(all[wipIdx + 1]).toHaveAttribute("data-sha", commits[2].sha);
  });

  it("hides the WIP row when the tree is clean", () => {
    setup({ wip: null });
    expect(screen.queryByTestId("cg-wip-row")).toBeNull();
    setup({ wip: { count: 0 } });
    expect(screen.queryByTestId("cg-wip-row")).toBeNull();
  });

  it("selects a commit on click and marks the selected row", () => {
    const { props, commits, rerender, handle } = setup();
    fireEvent.click(screen.getByText("subject 1"));
    expect(props.onSelect).toHaveBeenCalledWith(commits[1].sha);
    rerender(<CommitGraph ref={handle} {...props} selectedSha={commits[1].sha} />);
    expect(screen.getByText("subject 1").closest(".cg-row")).toHaveAttribute("aria-selected", "true");
  });

  it("selects the WIP sentinel when the WIP row is clicked", () => {
    const { props } = setup({ wip: { count: 2 } });
    fireEvent.click(screen.getByTestId("cg-wip-row"));
    expect(props.onSelect).toHaveBeenCalledWith(WIP_SELECTION);
  });

  it("renders ref, HEAD and worktree pills and forwards right-click on ref pills", () => {
    const { commits } = linearFixture(5);
    const main: RefInfo = { name: "main", kind: "local", sha: commits[0].sha, is_head: true, updated_at: 0 };
    const tag: RefInfo = { name: "v1.0.0", kind: "tag", sha: commits[3].sha, is_head: false, updated_at: 0 };
    const wt: WorktreeInfo = {
      path: "/tmp/repo-feature",
      display_path: "~/repo-feature",
      head_sha: commits[2].sha,
      branch: "feature",
      is_main: false,
      is_prunable: false,
      prunable_reason: null,
      source: "manual",
    };
    const { props } = setup({ refs: [main, tag], worktrees: [wt] });

    expect(screen.getByText("HEAD")).toHaveClass("cg-pill-head");
    expect(screen.getByText("main")).toHaveClass("cg-pill-local");
    expect(screen.getByText("v1.0.0")).toHaveClass("cg-pill-tag");
    expect(screen.getByText("repo-feature")).toHaveClass("cg-pill-worktree");

    fireEvent.contextMenu(screen.getByText("v1.0.0"));
    expect(props.onRefContextMenu).toHaveBeenCalledTimes(1);
    expect(vi.mocked(props.onRefContextMenu!).mock.calls[0][0]).toEqual(tag);
    expect(props.onSelect).not.toHaveBeenCalled();
  });

  it("scrollToSha returns true for loaded commits and false otherwise", () => {
    const { handle, commits, container } = setup({}, 2000);
    let ok = false;
    act(() => {
      ok = handle.current!.scrollToSha(commits[1500].sha);
    });
    expect(ok).toBe(true);
    expect(container.querySelector(`[data-sha="${commits[1500].sha}"]`)).not.toBeNull();
    expect(handle.current!.scrollToSha("deadbeef")).toBe(false);
  });

  it("fires onEndReached once when the list end is within the window", () => {
    const { props, rerender, handle } = setup({}, 10);
    expect(props.onEndReached).toHaveBeenCalledTimes(1);
    rerender(<CommitGraph ref={handle} {...props} selectedSha={props.commits[0].sha} />);
    expect(props.onEndReached).toHaveBeenCalledTimes(1);
  });

  it("fires onEndReached only after scrolling near the bottom of a long list", () => {
    const { props } = setup({}, 2000);
    expect(props.onEndReached).not.toHaveBeenCalled();
    const scroller = screen.getByTestId("commit-graph");
    act(() => {
      scroller.scrollTop = 1990 * ROW_HEIGHT;
      fireEvent.scroll(scroller);
    });
    expect(props.onEndReached).toHaveBeenCalledTimes(1);
  });
});
