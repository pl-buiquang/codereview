import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { useUIStore } from "../store";
import {
  __resetNavHistory,
  goBack,
  goForward,
  recordJump,
  registerReviewNav,
  startNavHistory,
  type ViewPos,
} from "./navHistory";

const reset = () =>
  useUIStore.setState({
    tabs: [{ id: "home", kind: "home" }],
    activeTabId: "home",
    homeSection: "board",
    homeRepoId: null,
    openRepoIds: [],
  });

describe("navHistory", () => {
  let stop: () => void;
  beforeEach(() => {
    reset();
    __resetNavHistory();
    stop = startNavHistory();
  });
  afterEach(() => stop());

  const s = () => useUIStore.getState();

  it("steps back and forward through home sections", () => {
    s().setHomeSection("inbox");
    s().setHomeSection("reviews");
    goBack();
    expect(s().homeSection).toBe("inbox");
    goBack();
    expect(s().homeSection).toBe("board");
    goForward();
    expect(s().homeSection).toBe("inbox");
  });

  it("does nothing when there is no history", () => {
    goBack();
    goForward();
    expect(s().homeSection).toBe("board");
  });

  it("a new navigation after Back clears Forward", () => {
    s().setHomeSection("inbox");
    goBack();
    s().setHomeSection("reviews");
    goForward();
    expect(s().homeSection).toBe("reviews");
  });

  it("Back reopens a review tab that was closed, under its parent repo", () => {
    s().openRepo(7);
    s().openReview(42);
    expect(s().activeTabId).toBe("review-42");
    s().closeReview();
    expect(s().tabs.some((t) => t.id === "review-42")).toBe(false);
    goBack();
    expect(s().activeTabId).toBe("review-42");
    expect(s().tabs.find((t) => t.id === "review-42")?.repoId).toBe(7);
  });

  it("Back to a repo that was closed re-adds it to the open repos", () => {
    s().openRepo(3);
    s().setHomeSection("inbox");
    s().closeRepo(3);
    goBack();
    expect(s().homeRepoId).toBe(3);
    expect(s().openRepoIds).toContain(3);
  });

  describe("jumps inside a review (go-to-definition)", () => {
    // A stand-in for a mounted review view: a scroll offset and a file pane.
    const mount = (reviewId: number, initial: ViewPos) => {
      const view = { pos: initial };
      const unregister = registerReviewNav(reviewId, {
        capture: () => ({ ...view.pos }),
        restore: (p) => {
          view.pos = { ...p };
        },
      });
      return { view, unregister };
    };

    it("Back returns to where the jump started, Forward to where it landed", () => {
      s().openReview(5);
      const { view } = mount(5, { scrollTop: 100, filePane: null });
      recordJump();
      view.pos = { scrollTop: 900, filePane: null }; // the jump
      goBack();
      expect(view.pos.scrollTop).toBe(100);
      goForward();
      expect(view.pos.scrollTop).toBe(900);
      expect(s().activeTabId).toBe("review-5");
    });

    it("restores the file pane that was (not) open", () => {
      s().openReview(5);
      const { view } = mount(5, { scrollTop: 10, filePane: null });
      recordJump();
      view.pos = { scrollTop: 10, filePane: { path: "a.rb", line: 12 } }; // "Open file"
      goBack();
      expect(view.pos.filePane).toBeNull();
      goForward();
      expect(view.pos.filePane).toEqual({ path: "a.rb", line: 12 });
    });

    it("a jump is one Back step, ahead of earlier tab changes", () => {
      s().setHomeSection("inbox");
      s().openReview(5);
      const { view } = mount(5, { scrollTop: 100, filePane: null });
      recordJump();
      view.pos = { scrollTop: 500, filePane: null };
      goBack();
      expect(view.pos.scrollTop).toBe(100);
      expect(s().activeTabId).toBe("review-5");
      goBack();
      expect(s().activeTabId).toBe("home");
      expect(s().homeSection).toBe("inbox");
    });

    it("a jump after Back clears Forward", () => {
      s().openReview(5);
      const { view } = mount(5, { scrollTop: 100, filePane: null });
      recordJump();
      view.pos = { scrollTop: 500, filePane: null };
      goBack();
      recordJump();
      view.pos = { scrollTop: 700, filePane: null };
      goForward(); // nothing ahead
      expect(view.pos.scrollTop).toBe(700);
    });

    it("applies the position once a reopened review mounts", () => {
      s().openReview(5);
      const first = mount(5, { scrollTop: 100, filePane: null });
      recordJump();
      first.view.pos = { scrollTop: 500, filePane: null };
      first.unregister();
      s().closeReview(); // tab closed and view unmounted
      goBack(); // reopens the tab (the close itself is a tab-level step); no view yet
      expect(s().activeTabId).toBe("review-5");
      goBack(); // the jump entry: its position waits for the view
      const second = mount(5, { scrollTop: 0, filePane: null });
      expect(second.view.pos.scrollTop).toBe(100);
    });
  });

  it("Back returns to settings", () => {
    s().openSettingsTab();
    s().setHomeSection("inbox");
    goBack();
    expect(s().activeTabId).toBe("settings");
  });
});
