# Spec 02 — Manual refresh + head-moved badge

Implements ROADMAP §3 (auto-refresh, manual-only) and §2's "review-level badge + refresh-diff
action". No interval polling, no settings change. See Spec 00 for shared contract.

## 1. Refresh helper + command — `src-tauri/src/commands/review.rs`

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreshnessResult {
    pub head_moved: bool,
    pub previous_head_sha: Option<String>,
    pub current_head_sha: Option<String>,
}

#[tauri::command]
pub fn refresh_review(review_id: i64, db: State<Db>) -> AppResult<FreshnessResult>;
```

Register `commands::review::refresh_review` in `lib.rs`.

### Behaviour

Re-resolve the target's SHAs and persist them, mirroring how each kind already resolves:

- **local** (`target.kind == "local"`): `git::rev_parse(repo_path, base_ref)` and
  `rev_parse(repo_path, head_ref)`; `UPDATE target SET base_sha=?, head_sha=? WHERE id=?`
  (same columns as `get_or_create_local_target`, `review.rs:116`).
- **github_pr**: fetch `gh::pr_view(&ctx, number)` → `info.head_sha`; `UPDATE target SET title=?,
  base_ref=?, head_ref=?, head_sha=? WHERE id=?` (same as `get_or_create_pr_target`, `review.rs:155`).

Follow `create_review_for_pr`'s **split-lock** pattern (`review.rs:225`): resolve repo + `gh_ctx_for_repo`
under the lock, **drop the lock** for the `gh pr view` / `git rev-parse` subprocess, re-lock to UPDATE.

`previous_head_sha` = the value read before the update; `current_head_sha` = the freshly resolved
value; `head_moved = previous != current` (and current is `Some`).

Do **not** re-anchor here — refresh just updates SHAs and surfaces the badge. (Re-anchoring is the
explicit user action in the UI; publish does it automatically — Spec 03.)

Factor the SHA-resolution+persist into a private helper so `publish_review` can reuse the
"fetch fresh head + detect move" half (Spec 03). A reasonable shape:
`fn refresh_target_shas(db, &Target) -> AppResult<FreshnessResult>` using the split-lock internally,
or split into `resolve_fresh_head(ctx/repo, &Target) -> AppResult<PrInfo-ish>` + a persist step.

### Tests (`review.rs`, real git via the Spec 01 fixture)

- Local target: after the branch advances, `refresh_review` updates stored `head_sha`/`base_sha`
  and returns `head_moved == true` with correct `previous`/`current`.
- No-op when nothing moved → `head_moved == false`.

## 2. Frontend — `src/lib/api.ts` + `src/lib/types.ts`

`types.ts` (camelCase, matching the serde rename):

```ts
export interface FreshnessResult { headMoved: boolean; previousHeadSha: string | null; currentHeadSha: string | null }
export interface ReanchorResult { reanchored: number; lost: number; skippedNoChange: number }
```

`api.ts` (add to the `api` object; `publishReview` stays as-is):

```ts
refreshReview: (reviewId: number) => invoke<FreshnessResult>("refresh_review", { reviewId }),
reanchorComments: (reviewId: number) => invoke<ReanchorResult>("reanchor_comments", { reviewId }),
```

## 3. Frontend — `src/components/ReviewView.tsx`

Reuse existing patterns in this file (React Query `useMutation`, `queryClient.invalidateQueries`,
the toast/`onSaving`/`onSaved` plumbing, and the header button row near Export/Publish ~`:252`).

- **Diff query key:** add the head SHA so a head change auto-refetches the diff. Change
  `["review-diff", reviewId, detailQuery.data?.target.id]` →
  `["review-diff", reviewId, detailQuery.data?.target.id, detailQuery.data?.target.head_sha]`.

- **Refresh action** (header button): `useMutation(() => api.refreshReview(reviewId))`.
  `onSuccess`: `invalidateQueries(["review", reviewId])` and `invalidateQueries(["pr-threads", owner, name, prNumber])`
  (the diff refetches automatically via the new key once `target.head_sha` changes). Optionally toast
  "Head moved — re-anchor to update comments" when `data.headMoved`.

- **Review-level "head moved" badge:** compute `headMoved = detail.comments.some(c =>
  c.anchored_head_sha && target.head_sha && c.anchored_head_sha !== target.head_sha)`. When true,
  render a header badge (e.g. "⚠ head moved · N comments may be outdated") with a **"Re-anchor
  comments"** button. The existing per-comment "outdated" badge (`:986`) stays as the granular signal.

- **Re-anchor action:** `useMutation(() => api.reanchorComments(reviewId))`. `onSuccess`: toast
  ``Re-anchored ${r.reanchored}, ${r.lost} could not be moved`` and `invalidateQueries(["review", reviewId])`
  (which cascades a diff refetch). Disable both actions while `readOnly`/published.

No changes to `settings.ts`, `SettingsView.tsx`, `main.tsx`, or the inbox.

## 4. Verify (manual, `pnpm tauri dev`)

Open a local virtual-PR review, add a RIGHT comment, advance the head branch by inserting a line
above the commented line, click **Refresh** → the diff updates and the head-moved badge appears;
click **Re-anchor comments** → the comment lands on the shifted line and its "outdated" badge clears.
