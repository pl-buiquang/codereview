import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useUIStore, type HomeSection } from "../store";
import { api } from "./api";
import { toast } from "./toast";

type DeepLinkAction =
  | { kind: "openReview"; review_id: number }
  | { kind: "openPr"; owner: string; name: string; number: number }
  | { kind: "openDiff"; repo_id: number; base_ref: string; head_ref: string; three_dot: boolean }
  | { kind: "navigateHome"; section: string | null };

const VALID_SECTIONS = new Set<HomeSection>(["inbox", "reviews", "archive", "repositories"]);

export async function handleDeepLinkAction(action: DeepLinkAction): Promise<void> {
  const { openReview, setActiveTab, setHomeSection } = useUIStore.getState();

  switch (action.kind) {
    case "openReview":
      openReview(action.review_id);
      break;

    case "openPr": {
      const review = await api.createReviewForPr(action.owner, action.name, action.number);
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
      const review = await api.createReview({
        repoId: action.repo_id,
        repoPath: repo.path,
        baseRef: action.base_ref,
        headRef: action.head_ref,
        threeDot: action.three_dot,
      });
      openReview(review.id, action.repo_id);
      break;
    }

    case "navigateHome":
      setActiveTab("home");
      if (action.section && VALID_SECTIONS.has(action.section as HomeSection)) {
        setHomeSection(action.section as HomeSection);
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
