# Board: auto-merge monitor, batch PR actions, persisted filters

## Context
`~/scratch/claude-config/skills/approved-pr-monitor` is a `/loop` skill. On each pass, `monitor.sh` finds open PRs (your own across 9 repos, plus a watchlist file) and then:
- runs `gh pr update-branch` on approved PRs that are BEHIND;
- squash-merges approved, green, CLEAN PRs that have GitHub auto-merge on;
- hands failing CI to Claude, which reads the logs and reruns only failures it judges flaky.

It also writes a markdown dashboard to the notes.

The board already tracks the same PRs (authored ones plus tracked ones, which replaces the watchlist), and the mirror keeps their merge state and CI status fresh. GitHub's own auto-merge does the merge itself once the PR is green and up to date. So the monitor reduces to two jobs: **update the branch when BEHIND** and **rerun failed jobs on CI failure**, capped by GitHub's run attempt number.

Per ADR 0003, scheduling belongs to the Rust sync engine, not frontend timers. The monitor is therefore a sync-engine job, not a React hook.

Goals:
- Move the monitor into the sync engine, switched on by a board toggle.
- Add per-card and batch (filtered cards) actions: update branch, enable auto-merge, rerun CI (all jobs / failed jobs).
- Persist the board filters across relaunch and section/pane switches.
- Delete the skill.

## Decisions (confirmed with user)
- **Monitor scope:** open, non-draft, tracked cards with `auto_merge != null` **and** `review_decision === "APPROVED"`.
- **Rerun cap:** automatic reruns use `--failed` only, and only while the run's `attempt < 3`. This is stateless and survives relaunch.
- **Manual rerun:** works like GitHub's buttons, on each workflow run of the head commit that has a failing check. "All jobs" runs `gh run rerun <id>`; "Failed jobs" runs `gh run rerun <id> --failed`.
- **Interval:** 5 minutes by default, configurable in Settings.
- **Runs app-wide:** the monitor ticks from the scheduler thread even when the Board isn't mounted and the window is hidden. This differs from light syncs, which need an active scope and a visible window. It runs whatever the Auto Sync setting is, because enabling it is an explicit opt-in.

## Backend (Rust)

### `src-tauri/src/board_monitor.rs` (new)
Pure logic, kept free of Tauri so a `cr` one-shot can reuse it later.
- `candidates(conn) -> Vec<Candidate>`: a query over `CARD_VIEW` (`commands/board.rs`) for PR cards with `state = 'OPEN'`, not draft, not discarded or archived, `auto_merge IS NOT NULL` and `review_decision = 'APPROVED'`. Returns card id, repo, number, `merge_state_status`, `viewer_can_update_branch` and `ci_status`.
- `run(db) -> Vec<MonitorAction{ card_id, repo, number, kind: "updated"|"reran"|"gave_up"|"error", detail }>`. For each candidate:
  1. If `merge_state_status == "BEHIND"` and `viewer_can_update_branch`, run `gh pr update-branch` (merge, matching the skill).
  2. Otherwise, if `ci_status` is FAILURE or ERROR: for each failing run from `gh::failing_workflow_runs`, rerun it with `--failed` when `gh::run_attempt < 3`; otherwise record `gave_up`.
  3. Refetch each touched card with the shared `refetch_card` helper.

  `gh` calls run unlocked and the refetch takes the DB lock briefly. A per-PR error is recorded as an `error` action and never aborts the run.
- Unit tests for `candidates` filtering, in the `seed_card` fixture style of the `commands/board.rs` tests.

### `src-tauri/src/sync/scheduler.rs`
- `SyncConfig` gains `board_monitor: bool` (default `false`) and `board_monitor_interval_min: u32` (default `5`). `State` gains `last_monitor: Option<Instant>` and a `monitoring` in-flight flag.
- In `tick()`, before the visibility / Auto Sync early return, spawn a monitor round on its own thread when the monitor is enabled, due and not already running:
  1. Run `Scope::Board` / `Mode::Light` through `execute`. The probe already compares `merge_state_status` and `ci_state` (`pipeline::needs_refetch`), so the mirror is fresh. If an overlapping Board sync is running, skip this tick.
  2. Call `board_monitor::run`.
  3. Emit `mirror-updated` (scope Board, touched ids) and a new `board-monitor` event carrying the actions. A failure goes into `last_error` and the event.
- `configure()` sets `last_monitor = None` when the monitor flips on, so a round runs on the next tick.

### `src-tauri/src/gh.rs`
- Add `failing_workflow_runs(owner, name, number) -> AppResult<Vec<FailingRun{ run_id: i64, workflow: String }>>`.
  - It's a GraphQL query on `pullRequest.commits(last:1).commit.statusCheckRollup.contexts(first:100)`, selecting `... on CheckRun { conclusion status checkSuite { workflowRun { databaseId workflow { name } } } }`. Use the existing `graphql<T>` helper.
  - `ContextNode` (the check-run union node used by the PR meta query) already models these contexts; extend it or add a sibling type rather than duplicating the shape.
  - Collect the distinct `databaseId`s whose CheckRun conclusion is one of FAILURE / TIMED_OUT / CANCELLED / STARTUP_FAILURE. That's the same set `monitor.sh` uses in `failing_for`.
- Add `run_attempt(repo, run_id) -> AppResult<i64>`, which reads `gh run view <id> --repo <repo> --json attempt`.
- Add `rerun_run(repo, run_id, failed_only)`, which runs `gh run rerun <id> --repo <repo> [--failed]`.
- Add unit tests for parsing the failing-run list, following the existing `map_pr_meta` fixture test in `gh.rs`.

### `src-tauri/src/commands/board.rs`
- Add `board_pr_rerun_ci(card_id, failed_only) -> AppResult<u32>`.
  - Use `pr_card_coords` to get the coordinates, list the failing runs, rerun each one and return the count.
  - If there are no failing runs, return `AppError` ("no failed workflow runs").
  - Afterwards, refetch the card. Pull the tail of `run_pr_action` (`resolve_pr` + `upsert_node`) out into a small `refetch_card` helper so this command, `run_pr_action` and `board_monitor` share it.
- Register `board_pr_rerun_ci` in `lib.rs` `invoke_handler`, next to the other `board_pr_*` commands. There is no tick command: the scheduler drives the monitor.

No DB migration is needed.

## Frontend

### `src/lib/api.ts` / `src/lib/types.ts`
- Add `boardPrRerunCi(cardId, failedOnly): Promise<number>`.
- `syncConfigure` sends the two new monitor fields in `config`.
- Add the `MonitorAction` type.

### `src/lib/settings.ts`
- Add `boardMonitor: boolean` (default `false`) and `boardMonitorIntervalMin: number` (default `5`), each with a setter.
- Follow the `boardOverviewOpen` recipe: `SettingsState`, `PersistedSettings` Pick, `DEFAULTS`, setters. No version bump is needed, since the default merge fills new keys.
- Add a `BOARD_MONITOR_OPTIONS` list (2 / 5 / 10 / 15 / 30 min).

### `src/lib/sync.ts` (`useSyncBridge`)
- Pass the two settings into the existing `syncConfigure` effect.
- Add a `useTauriListener("board-monitor", ...)` that shows one toast summarizing the non-empty actions, e.g. "Monitor: updated 2 branches, reran 1 CI · 1 gave up (repo#123)", and uses `toast.error` on failure. `mirror-updated` already invalidates `["board"]`, so no manual invalidation is needed.

### `src/components/SettingsView.tsx`
- Add an "Auto-merge monitor interval" select next to the existing Sync settings.

### `src/store.ts` (filter persistence)
- Move the board filters into the persisted `useUIStore`: `boardFilters: { repo, status, ci, merge, tags, excludeTags }` plus a `setBoardFilters(patch)` setter, added to `partialize`.
- Bump the version from 3 to 4, extending `migratePersisted` to default the field.
- In `BoardView.tsx`, replace the six filter `useState` calls (`repoFilter`, `statusFilter`, `ciFilter`, `mergeFilter`, `tagFilter`, `tagExclude`) with selectors and the setter. The "effective filter" logic already drops stale values, so nothing else changes.

### `src/components/BoardView.tsx` header
- **Monitor toggle:** add an "Auto-merge monitor" toggle button right after "AI Overview", using the same `btn` + `active` pattern, flipping `settings.boardMonitor`. Its title shows the interval and a short explanation.
- **Batch menu:** add a "Batch (N)" `KebabMenu`, where N is the number of filtered open PR cards (`filtered`). Each item:
  - lists the eligible cards it will act on, client-side;
  - asks for confirmation with `confirmDialog` ("Update branch on 4 of 9 filtered PRs?");
  - runs the existing per-card api calls sequentially;
  - ends with one aggregate toast ("Updated 3/4 · 1 failed: repo#12 …") and one `invalidate()`.

  Items and the cards each one acts on:

  | Item | Eligible cards |
  |---|---|
  | Update branch | `merge_state_status === "BEHIND"` and `viewer_can_update_branch`; merge, not rebase |
  | Enable auto-merge | open, not draft, `auto_merge == null` |
  | Re-run failed jobs | `ci_status` FAILURE/ERROR → `boardPrRerunCi(id, true)` |
  | Re-run all jobs | the same cards → `boardPrRerunCi(id, false)` |

- Add a small `runBatch(cards, fn, label)` helper inside `BoardView` for the loop, aggregation and toast.
- Track a `batchPending` flag so the menu stays disabled while a batch runs.

### `src/components/BoardCard.tsx` `PrActionsMenu`
- Add "Re-run failed jobs" and "Re-run all jobs" items next to "Update branch".
- Disable them with a reason when CI isn't failing.
- Wire them to a new `prRerunCi` mutation in `BoardView` (next to `prUpdateBranch`), and include it in `prActionCardId` so the busy state covers it. The update-branch and auto-merge per-card actions already exist.

## Skill removal
- In the `claude-config` repo (`~/scratch/claude-config`), `git rm -r skills/approved-pr-monitor` and commit (`chore(skills): remove approved-pr-monitor`).
- Leave `~/Nextcloud/Notes/_work/approved-pr-watchlist.txt`, `approved-pr-monitor.md` and `.approved-pr-monitor.json` in place, and tell the user about them. Any watchlist PRs should be tracked on the board instead (its "Track" dialog).

## Verification
- `cargo clippy` and `cargo test --manifest-path src-tauri/Cargo.toml` (covers the failing-run parser and `board_monitor::candidates`).
- `pnpm exec tsc --noEmit`, and `pnpm vitest run` (`gitChanged.test.ts` exists).
- Run `pnpm tauri dev` and check these in the app:
  - Set filters, switch to Inbox and back, open a repo and back, then relaunch: the filters are still there.
  - Batch "Update branch" on a filtered set with a BEHIND PR: the confirm shows the right count, the branch updates and the toast reports it.
  - Card "Re-run failed jobs" on a PR with red CI: the run restarts on GitHub (check with `gh run list`).
  - Toggle the monitor on: it ticks within about 20 s (the scheduler tick) and the toast or debug log (`[sync]` / `board-monitor`) shows actions. It keeps ticking after you leave the board section and while the window is hidden. Changing the interval in Settings takes effect.
