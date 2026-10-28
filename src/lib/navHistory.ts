import { useUIStore, type HomeSection } from "../store";

/** A position inside a review tab: how far its diff is scrolled and which file pane is open. */
export interface ViewPos {
  scrollTop: number;
  filePane: { path: string; line?: number; scrollTop?: number } | null;
}

/** What a mounted review view lends to the history so jumps inside it can be undone. */
export interface ReviewNav {
  capture(): ViewPos;
  restore(pos: ViewPos): void;
}

/** Where the user is: enough to put them back via the UI store. `repoId` is the
 *  parent repo of a review tab, so a closed review can be reopened in place. `pos` is only set
 *  for jumps inside a review (see [`recordJump`]) and for the entry Back/Forward leave behind. */
interface Location {
  activeTabId: string;
  homeSection: HomeSection;
  homeRepoId: number | null;
  repoId?: number;
  pos?: ViewPos;
}

const MAX_ENTRIES = 100;

let back: Location[] = [];
let forward: Location[] = [];
let applying = false;

type NavState = ReturnType<typeof useUIStore.getState>;

const REVIEW_PREFIX = "review-";
const views = new Map<number, ReviewNav>();
/** Positions to apply once a review that Back reopened has mounted its view. */
const pendingPos = new Map<number, ViewPos>();

const reviewIdOf = (tabId: string) =>
  tabId.startsWith(REVIEW_PREFIX) ? Number(tabId.slice(REVIEW_PREFIX.length)) : null;

/** A mounted review view registers here; returns the unregister function. */
export function registerReviewNav(reviewId: number, nav: ReviewNav): () => void {
  views.set(reviewId, nav);
  const pending = pendingPos.get(reviewId);
  if (pending) {
    pendingPos.delete(reviewId);
    nav.restore(pending);
  }
  return () => {
    if (views.get(reviewId) === nav) views.delete(reviewId);
  };
}

/** `withPos` adds the live view position; only reliable for the visible tab. */
function locationOf(s: NavState, withPos = false): Location {
  const tab = s.tabs.find((t) => t.id === s.activeTabId);
  const reviewId = reviewIdOf(s.activeTabId);
  const view = withPos && reviewId != null ? views.get(reviewId) : undefined;
  return {
    activeTabId: s.activeTabId,
    homeSection: s.homeSection,
    homeRepoId: s.homeRepoId,
    repoId: tab?.repoId,
    pos: view?.capture(),
  };
}

/**
 * Remember where the user is before a jump that stays in the same tab (go-to-definition), so
 * Back returns to this scroll position and Forward to where the jump landed. Call it right before
 * moving.
 */
export function recordJump() {
  back.push(locationOf(useUIStore.getState(), true));
  if (back.length > MAX_ENTRIES) back.shift();
  forward = [];
}

const sameLocation = (a: Location, b: Location) =>
  a.activeTabId === b.activeTabId && a.homeSection === b.homeSection && a.homeRepoId === b.homeRepoId;

/** Record every location change made by the store actions; back/forward replays are ignored. */
export function startNavHistory(): () => void {
  return useUIStore.subscribe((state, prev) => {
    if (applying) return;
    const before = locationOf(prev);
    if (sameLocation(before, locationOf(state))) return;
    back.push(before);
    if (back.length > MAX_ENTRIES) back.shift();
    forward = [];
  });
}

function restore(loc: Location) {
  applying = true;
  try {
    const store = useUIStore.getState();
    if (loc.activeTabId === "settings") {
      store.openSettingsTab();
    } else if (loc.activeTabId.startsWith(REVIEW_PREFIX)) {
      const reviewId = Number(loc.activeTabId.slice(REVIEW_PREFIX.length));
      if (store.tabs.some((t) => t.id === loc.activeTabId)) store.setActiveTab(loc.activeTabId);
      else store.openReview(reviewId, loc.repoId);
      if (loc.pos) {
        const view = views.get(reviewId);
        if (view) view.restore(loc.pos);
        else pendingPos.set(reviewId, loc.pos);
      }
    } else {
      useUIStore.setState((s) => ({
        activeTabId: "home",
        homeSection: loc.homeSection,
        homeRepoId: loc.homeRepoId,
        openRepoIds:
          loc.homeRepoId == null || s.openRepoIds.includes(loc.homeRepoId)
            ? s.openRepoIds
            : [...s.openRepoIds, loc.homeRepoId],
      }));
    }
  } finally {
    applying = false;
  }
}

function step(from: Location[], to: Location[]) {
  const target = from.pop();
  if (!target) return;
  to.push(locationOf(useUIStore.getState(), true));
  restore(target);
}

export const goBack = () => step(back, forward);
export const goForward = () => step(forward, back);

export function __resetNavHistory() {
  back = [];
  forward = [];
  applying = false;
  views.clear();
  pendingPos.clear();
}
