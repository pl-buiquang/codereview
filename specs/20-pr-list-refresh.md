# Spec 20 — PR-list refresh + optional polling

Implements the remaining half of ROADMAP §3 "Auto-refresh & polling" (`ROADMAP.md:45-46`):
*"still want PR-list refresh and optional interval polling."* Frontend-only; no backend change.

## Problem

The GitHub-PRs tab of a repo (`PrList`, `src/components/RepoView.tsx:219-280`) fetches the open-PR
list exactly once per cache lifetime:

- The query (`["prs", repo.path]`, `RepoView.tsx:222-226`) has no `refetchInterval`, and the global
  `QueryClient` disables focus refetching (`src/main.tsx:8-10`,
  `refetchOnWindowFocus: false`), so a PR opened/closed/retitled on GitHub never shows up until the
  app restarts or the cache is otherwise invalidated.
- There is no Refresh affordance anywhere on the list, and no indication of how stale the data is.
  The Inbox already solved this UX (`src/components/InboxView.tsx:146-155`: "updated Xm ago" +
  `↻ Refresh` button with a `.spinner`); the PR list should mirror it.

Note: Spec 00's locked "manual button only, no interval polling" decision was scoped to **review
head-SHA freshness** (the per-review diff). This spec is the *repo PR list*, which the ROADMAP
explicitly calls out for optional polling. No conflict.

## Decisions (locked)

- **Frontend-only, XS.** Reuse `api.listPrs` (`src/lib/api.ts:64`) unchanged; everything is
  query-options + a small toolbar.
- **Refresh = `prsQuery.refetch()`**, spinner driven by `prsQuery.isFetching` — one indicator
  covers both manual refreshes and background polls.
- **Polling setting lives in `useSettingsStore`** (`src/lib/settings.ts`, persisted as
  `codereview-settings`) — *not* `src/store.ts`, which persists UI tabs only. Adding a key with a
  default needs **no persist version bump**: the `version >= 1` migrate branch returns the
  persisted object unchanged (`settings.ts:334-336`) and zustand's default shallow merge fills
  missing keys from the initializer.
- **Setting shape: `prListPollMs: number`, `0` = off (default).** Choices `0 / 30_000 / 60_000 /
  300_000` ("off / 30s / 60s / 5m"). A raw ms number feeds `refetchInterval` directly; no enum
  mapping layer.
- **Global setting, not per-repo** — one knob, simplest thing that satisfies the ROADMAP item.
- **Interval control stays inline in the PR-list toolbar.** A `SettingsView.tsx` does exist, but
  this is a list-context control (like the Virtual-PR tab's merge-base checkbox,
  `RepoView.tsx:176-179`); do not add a SettingsView section this round.
- **Staleness label** uses the query's `dataUpdatedAt` + the existing `timeAgo` helper. Widen
  `timeAgo` to accept `string | number` (`new Date()` handles both) rather than converting at the
  call site. A local 30s tick keeps the label honest while polling is off.
- **Polling pauses when the window is unfocused** — TanStack v5's `refetchIntervalInBackground`
  defaults to `false`; accept that (it is the desirable desktop behavior, no battery burn).

## Design

### 1. `src/lib/settings.ts` — persisted poll-interval setting

```ts
// Near the other exported presets (cf. MONO_FONT_PRESETS, settings.ts:101):
/** PR-list auto-refresh choices; value is the refetchInterval in ms, 0 = off. */
export const PR_LIST_POLL_OPTIONS: { label: string; value: number }[] = [
  { label: "off", value: 0 },
  { label: "30s", value: 30_000 },
  { label: "60s", value: 60_000 },
  { label: "5m", value: 300_000 },
];
```

- `SettingsState` (`settings.ts:243-265`): add
  `prListPollMs: number;` and `setPrListPollMs: (ms: number) => void;`.
- Initializer (`settings.ts:269-277`): `prListPollMs: 0,` and
  `setPrListPollMs: (prListPollMs) => set({ prListPollMs }),`.
- `PersistedSettings` pick-union (`settings.ts:231-241`): add `"prListPollMs"`.
- `partialize` (`settings.ts:324-333`): add `prListPollMs: s.prListPollMs,`.
- v0 migrate branch (`settings.ts:340-349`): no change needed (key absent → default via merge),
  but adding `prListPollMs: 0` to the constructed `next` object keeps `PersistedSettings`
  type-complete — do that.

### 2. `src/lib/timeAgo.ts` — accept epoch ms

```ts
/** Compact "x ago" relative time, e.g. "3h ago", "2d ago". Accepts ISO string or epoch ms. */
export function timeAgo(when: string | number): string {
  const then = new Date(when).getTime();
  // ...rest unchanged
```

(`new Date(0)` is valid, but the caller hides the label while `dataUpdatedAt === 0`; see below.)

### 3. `src/components/RepoView.tsx` — `PrList` toolbar + polling

All changes are inside `PrList` (`RepoView.tsx:219-280`).

```
┌ tabs: [Virtual PR] [GitHub PRs*] ─────────────────────────────────────┐
│ ┌ .pr-list-toolbar ────────────────────────────────────────────────┐ │
│ │ Open pull requests     updated 2m ago   auto [off ▾]  [↻ Refresh]│ │
│ └──────────────────────────────────────────────────────────────────┘ │
│ ┌ .pr-list ────────────────────────────────────────────────────────┐ │
│ │ #42 Fix anchor drift    alice · main ← fix/drift        [Review] │ │
│ │ #40 …                                                            │ │
│ └──────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────┘
```

```tsx
import { PR_LIST_POLL_OPTIONS, useSettingsStore } from "../lib/settings"; // already imported in RepoView
import { timeAgo } from "../lib/timeAgo";

function PrList({ repo, onOpen }: { repo: Repository; onOpen: (id: number) => void }) {
  const prListPollMs = useSettingsStore((s) => s.prListPollMs);
  const setPrListPollMs = useSettingsStore((s) => s.setPrListPollMs);

  const authQuery = useQuery({ queryKey: ["gh-auth"], queryFn: api.ghAuthStatus });
  const prsQuery = useQuery({
    queryKey: ["prs", repo.path],
    queryFn: () => api.listPrs(repo.path),
    enabled: authQuery.data === true,
    refetchInterval: prListPollMs > 0 ? prListPollMs : false,
  });

  // Re-render every 30s so the "updated Xm ago" label stays honest with polling off.
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);
  // ... startPrReview mutation unchanged (RepoView.tsx:228-240)
```

Rendering: keep the two **auth** early-returns (`RepoView.tsx:242-248`) as-is, then render the
toolbar **unconditionally** (so Refresh is available even on error/empty), followed by the body
(loading / error / empty / list — the current returns at `RepoView.tsx:249-254` become branches
under the toolbar):

```tsx
return (
  <>
    <div className="pr-list-toolbar">
      <span className="muted">Open pull requests</span>
      <span className="pr-list-toolbar-right">
        {prsQuery.dataUpdatedAt > 0 && (
          <span className="muted small">updated {timeAgo(prsQuery.dataUpdatedAt)}</span>
        )}
        <label className="muted small">
          auto
          <select
            value={prListPollMs}
            onChange={(e) => setPrListPollMs(Number(e.target.value))}
          >
            {PR_LIST_POLL_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </label>
        <button
          className="btn-primary"
          disabled={prsQuery.isFetching}
          onClick={() => prsQuery.refetch()}
        >
          {prsQuery.isFetching ? <><span className="spinner" /> Refreshing…</> : "↻ Refresh"}
        </button>
      </span>
    </div>
    {body /* isLoading → "Loading open PRs…"; isError → error <p>; [] → "No open pull requests."; else .pr-list rows (all existing markup, RepoView.tsx:249-279, unchanged) */}
  </>
);
```

Reused styles: `.btn-primary` (+ its `:disabled`, `styles.css:81-89`), `.spinner`
(`styles.css:2092-2101`), `.muted`/`.small` — exactly the Inbox refresh pattern
(`InboxView.tsx:146-155`).

### 4. `src/styles.css` — one new toolbar rule

Next to `.pr-list` (`styles.css:961`):

```css
.pr-list-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 4px 12px;
}
.pr-list-toolbar-right {
  display: flex;
  align-items: center;
  gap: 10px;
}
```

(Match the existing select/label look used by `.compare-bar` — no new select styling.)

## Tasks

1. `settings.ts`: add `PR_LIST_POLL_OPTIONS`, `prListPollMs` (+ setter) to state, defaults,
   `PersistedSettings`, `partialize`, v0-migrate object. Update the `beforeEach` state reset in
   `src/lib/settings.test.ts:14-25` to include `prListPollMs: 0`.
2. `timeAgo.ts`: widen the parameter to `string | number`.
3. `RepoView.tsx`: restructure `PrList` per Design §3 (toolbar + `refetchInterval` + 30s tick),
   leaving the row markup and `startPrReview` untouched.
4. `styles.css`: add `.pr-list-toolbar` / `.pr-list-toolbar-right`.
5. Tests (below), then gates.

## Test matrix (vitest)

| Test (file) | Asserts |
|---|---|
| `settings.test.ts` — `defaults prListPollMs to 0 (off)` | `useSettingsStore.getState().prListPollMs === 0` |
| `settings.test.ts` — `setPrListPollMs updates and is persisted` | after `setPrListPollMs(30000)`, state is `30000` and the `partialize`d snapshot (`useSettingsStore.persist.getOptions().partialize!(state)`) contains `prListPollMs: 30000` |
| NEW `src/lib/timeAgo.test.ts` — `accepts epoch ms` | `timeAgo(Date.now() - 5 * 60_000)` → `"5m ago"` (use `vi.setSystemTime` for determinism) |
| `timeAgo.test.ts` — `still accepts ISO strings / garbage` | ISO string 2h back → `"2h ago"`; `timeAgo("nope")` → `""` |
| NEW `src/components/RepoView.test.tsx` — `manual refresh refetches the PR list` | mock `../lib/api` (`ghAuthStatus` → `true`, `listPrs` → one `PrSummary`) following the `RepositoriesView.test.tsx:1-24` pattern; render `<RepoView repo={…}/>`, click the "GitHub PRs" tab, `waitFor` the PR row, click `↻ Refresh`, `waitFor(() => expect(listPrs).toHaveBeenCalledTimes(2))` |
| `RepoView.test.tsx` — `staleness label appears after first load` | after the row renders, `screen.getByText(/updated just now/)` exists |
| `RepoView.test.tsx` — `interval select writes the setting` | `userEvent.selectOptions` to `"30s"` → `useSettingsStore.getState().prListPollMs === 30000`; reset the store in `beforeEach` |
| `RepoView.test.tsx` — `toolbar still renders when the list is empty` | `listPrs` → `[]`: "No open pull requests." **and** the Refresh button are both present |

Do **not** fake-timer-test the `refetchInterval` itself (TanStack-internal, flaky); the wiring is a
one-liner covered by the select→store test.

## Gates

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test`
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

(The two cargo gates must stay green but should be no-ops — this spec touches no Rust.)

## Manual verify

1. `pnpm tauri dev`; open a repo tab for a repo with a GitHub remote (`gh auth login` done);
   switch to the **GitHub PRs** tab.
2. Toolbar shows "updated just now" after load. Click **↻ Refresh** — button disables and shows the
   spinner, then the timestamp resets.
3. In a terminal, open or close a PR (`gh pr create`/`gh pr close -R <owner>/<repo> <n>`), click
   Refresh — the list reflects it.
4. Set **auto** to `30s`; repeat step 3 without clicking — within ~30s the list updates by itself
   and the "updated" label resets. Leave the app idle >1m with auto **off** — the label ticks to
   "1m ago" on its own.
5. Set auto to `5m`, quit, relaunch (`pnpm tauri dev` again): the select still reads `5m`
   (persisted in `localStorage` key `codereview-settings`).
6. Sanity: error path — temporarily break the remote (e.g. rename it), Refresh shows the error
   `<p>` but the toolbar/Refresh button remain usable.

## Out of scope

- Polling anything else (PR threads, review diff, inbox) — review freshness is specs 00-03 and is
  manual-only by locked decision; the inbox has its own refresh.
- Per-repo intervals, `refetchIntervalInBackground: true`, or pausing on metered connections.
- A SettingsView entry for the interval (inline control only this round).
- PR-list search/filter/sort (separate ROADMAP "Future ideas" item, `ROADMAP.md:121`).
- Backend caching or a `list_prs` delta API — `gh pr list` is cheap at this scale.
