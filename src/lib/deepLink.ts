import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useUIStore, type HomeSection } from "../store";
import { api } from "./api";
import { queryClient } from "./queryClient";
import { toast } from "./toast";
import { parseRepoBasePaths, useSettingsStore } from "./settings";
import { findBoardCard } from "./commandPaletteSearch";

type DeepLinkAction =
  | { kind: "openReview"; review_id: number }
  | { kind: "openPr"; owner: string; name: string; number: number }
  | { kind: "openDiff"; repo_id: number; base_ref: string; head_ref: string; three_dot: boolean }
  | { kind: "openRepo"; repo_id: number; ref_name: string | null; worktree: string | null }
  | { kind: "navigateHome"; section: string | null }
  | { kind: "showBoardCard"; owner: string; name: string; number: number };

const VALID_SECTIONS = new Set<HomeSection>(["sessions", "board", "inbox", "reviews", "archive", "worktrees", "repositories"]);

export async function handleDeepLinkAction(action: DeepLinkAction): Promise<void> {
  const { openReview, setActiveTab, setHomeSection, showBoardCard } = useUIStore.getState();

  switch (action.kind) {
    case "openReview":
      // The review may already be open with stale data (e.g. `cr focus set` ran).
      queryClient.invalidateQueries({ queryKey: ["review", action.review_id] });
      queryClient.invalidateQueries({ queryKey: ["focus", action.review_id] });
      openReview(action.review_id);
      break;

    case "openPr": {
      const basePaths = parseRepoBasePaths(useSettingsStore.getState().repoBasePaths);
      const review = await api.createReviewForPr(action.owner, action.name, action.number, basePaths);
      openReview(review.id);
      break;
    }

    case "openDiff": {
      const repos = await api.listRepositories();
      const repo = repos.find((r) => r.id === action.repo_id);
      if (!repo) {
        toast.error(`Repository #${action.repo_id} not found`);
        return;
      }
      if (!repo.local_path) {
        toast.error(`Repository #${action.repo_id} has no local clone; cannot create a virtual PR diff`);
        return;
      }
      const review = await api.createReview({
        repoId: action.repo_id,
        baseRef: action.base_ref,
        headRef: action.head_ref,
        threeDot: action.three_dot,
      });
      openReview(review.id, action.repo_id);
      break;
    }

    case "openRepo": {
      const repos = await api.listRepositories();
      const repo = repos.find((r) => r.id === action.repo_id);
      if (!repo) {
        toast.error(`Repository #${action.repo_id} not found`);
        return;
      }
      if (!repo.local_path) {
        toast.error(`Repository #${action.repo_id} has no local clone; cannot show its graph`);
        return;
      }
      useUIStore.getState().focusGraph({
        repoId: repo.id,
        ref: action.ref_name,
        worktree: action.worktree,
      });
      break;
    }

    case "showBoardCard": {
      const cards = await queryClient.fetchQuery({ queryKey: ["board"], queryFn: api.boardList });
      const card = findBoardCard(cards, `${action.owner}/${action.name}`, action.number);
      if (!card) {
        toast.error(`${action.owner}/${action.name}#${action.number} is not on the board`);
        return;
      }
      showBoardCard(card.id);
      break;
    }

    case "navigateHome":
      if (action.section && VALID_SECTIONS.has(action.section as HomeSection)) {
        setHomeSection(action.section as HomeSection);
      } else {
        setActiveTab("home");
      }
      break;
  }
}

export function useDeepLinkListener(): void {
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;

    listen<DeepLinkAction>("deep-link-action", (event) => {
      handleDeepLinkAction(event.payload).catch((e: unknown) => {
        toast.error(`Deep link failed: ${String(e)}`);
      });
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch((e) => {
        console.warn("Could not register deep-link listener", e);
      });

    listen<string>("deep-link-error", (event) => {
      toast.error(`Invalid deep link: ${event.payload}`);
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlistenError = dispose;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
      unlistenError?.();
    };
  }, []);
}
