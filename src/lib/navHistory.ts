import { useUIStore, type HomeSection } from "../store";

/** Where the user is: enough to put them back via the UI store. `repoId` is the
 *  parent repo of a review tab, so a closed review can be reopened in place. */
interface Location {
  activeTabId: string;
  homeSection: HomeSection;
  homeRepoId: number | null;
  repoId?: number;
}

const MAX_ENTRIES = 100;

let back: Location[] = [];
let forward: Location[] = [];
let applying = false;

type NavState = ReturnType<typeof useUIStore.getState>;

function locationOf(s: NavState): Location {
  const tab = s.tabs.find((t) => t.id === s.activeTabId);
  return {
    activeTabId: s.activeTabId,
    homeSection: s.homeSection,
    homeRepoId: s.homeRepoId,
    repoId: tab?.repoId,
  };
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
    } else if (loc.activeTabId.startsWith("review-")) {
      const reviewId = Number(loc.activeTabId.slice("review-".length));
      if (store.tabs.some((t) => t.id === loc.activeTabId)) store.setActiveTab(loc.activeTabId);
      else store.openReview(reviewId, loc.repoId);
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
  to.push(locationOf(useUIStore.getState()));
  restore(target);
}

export const goBack = () => step(back, forward);
export const goForward = () => step(forward, back);

export function __resetNavHistory() {
  back = [];
  forward = [];
  applying = false;
}
