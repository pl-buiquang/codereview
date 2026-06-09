# Spec 10 — `base_sha` for GitHub-PR targets (LEFT-side context expansion)

Implements ROADMAP §1 "Diff context expansion on GitHub-PR targets".

## Problem

Expanding collapsed diff context needs the **base** (LEFT/old-side) file, because
`react-diff-view`'s expansion math works in old-side line numbers (`expandFromRawCode` over the
raw base source — see `ensureSource`, `src/components/ReviewView.tsx:524-537`, which always fetches
`side = "LEFT"`). Today this only works for local targets:

- `get_or_create_pr_target` inserts `base_sha = NULL` for every `github_pr` target
  (`src-tauri/src/commands/review.rs:182`) and its refresh UPDATE never touches `base_sha`
  (`review.rs:173`).
- `refresh_target_shas`'s `github_pr` arm (`review.rs:217-233`) likewise updates only
  `title/base_ref/head_ref/head_sha`.
- So `file_source` (`review.rs:495-538`) resolves `"LEFT" => detail.target.base_sha` to `None`
  (`review.rs:504`) and fails with *"this side has no source (file added or deleted)"*
  (`review.rs:509`) — even though the clone-less fallback `gh::file_at_ref`
  (`src-tauri/src/gh.rs:145-155`) would happily serve the blob if we had a SHA.
- The frontend therefore gates expansion to local targets: `canExpand` requires
  `detail.target.kind === "local"` (`src/components/ReviewView.tsx:518-522`), with a comment
  saying PR targets are "rejected server-side (v1)".

### Why merge-base, not `baseRefOid`

PR diffs are **three-dot** (`base...head`): the old side of every hunk is the file at the
**merge-base** of base and head, not at the base-branch tip. ROADMAP §1 calls this out
explicitly: *"Mind the three-dot caveat: the diff's old side is the merge-base, not the
base-branch tip."* If base advanced after the head branched off, `file@baseRefOid` has different
line numbers than the diff's old side, and expanded context would be offset/garbled. So
`target.base_sha` for `github_pr` targets must store the **merge-base SHA**. (Conveniently,
`gh::pr_view` doesn't even fetch `baseRefOid` today — `gh.rs:283-303` — so there is no temptation
to store the wrong thing.)

The REST compare API already used by `gh::compare` (`gh.rs:244-254`) returns the merge-base in
the same response (`merge_base_commit.sha`); we just don't capture it.

ROADMAP §1 also requires a **backfill**: reopening a saved review doesn't re-run
`get_or_create_pr_target`, so pre-existing target rows keep `NULL` forever unless something heals
them.

## Decisions (locked)

- **`base_sha` for `kind='github_pr'` = the merge-base SHA of `base_ref` and the PR head**, never
  `baseRefOid`. Document this on the column's writers. Local targets are unchanged
  (`git rev-parse <base_ref>`, `review.rs:126`).
- **New minimal `gh` call, `compare()` untouched.** Add `gh::merge_base_sha(owner, name, base,
  head)` hitting `repos/{o}/{n}/compare/{base}...{head}?per_page=1` and reading only
  `merge_base_commit.sha`. Rationale: `compare()`'s `Vec<ComparedFile>` return type stays stable
  for the re-anchor caller (`review.rs:341`); `?per_page=1` caps the commits payload (the `files`
  array still arrives on page 1 but is ignored — one HTTP call, no extra parsing logic). Capture
  the field by extending the existing private `CompareRaw` (`gh.rs:236-239`).
- **Compare head argument = the pinned head SHA when known** (`info.head_sha` from `pr_view` at
  create/refresh time; stored `target.head_sha`, falling back to `head_ref`, in the lazy
  backfill). Rationale: the merge-base must match the head the stored diff/comments are pinned to.
- **Failure degrades gracefully, never propagates.** If the compare call fails (network, deleted
  base branch, merged PR), keep the existing `base_sha` value (possibly `NULL`) and let the parent
  operation succeed. Persist via `base_sha = COALESCE(?new, base_sha)` so a transient failure
  never clobbers a previously-resolved value. A `NULL` that survives the lazy backfill surfaces as
  a clear `file_source` error which the frontend already renders (`setExpandError`,
  `ReviewView.tsx:532`).
- **Lazy backfill lives in `file_source`**, under the lock the function already holds — precedent:
  `file_source` already shells out to `gh::file_at_ref` under that same lock (`review.rs:533`).
  No new command, no manual migration step; old rows heal on first LEFT request.
- **No DB migration.** `target.base_sha` exists since `0001_init.sql`. Migrations 0007/0008 stay
  reserved for specs 12/16.
- **Frontend change is a minimal predicate edit** in `ReviewView.tsx` (`canExpand`) — the file is
  contended by other specs; do not restructure anything else. Leading/trailing expanders
  (`ReviewView.tsx:917-918` "v1.1") stay out of scope.
- **Split-lock discipline** for the new subprocess calls in `create_review_for_pr` and
  `refresh_target_shas`: resolve owner/name under the lock, run `gh` with the lock dropped,
  re-lock to UPDATE — same pattern `refresh_target_shas` already documents (`review.rs:211-213`).

## Design

### 1. `src-tauri/src/gh.rs` — capture `merge_base_commit`

```rust
/// The merge-base commit reference embedded in a REST compare response.
#[derive(Debug, Deserialize)]
struct CommitRef {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct CompareRaw {
    #[serde(default)]
    merge_base_commit: Option<CommitRef>, // NEW (absent in old fixtures → None)
    files: Vec<ComparedFile>,
}

/// Merge-base of `base` and `head` via the REST compare API (clone-less).
/// `?per_page=1` caps the commits payload; only `merge_base_commit.sha` is read.
/// This is the LEFT side of GitHub's three-dot PR diff — NOT the base-branch tip.
pub fn merge_base_sha(owner: &str, name: &str, base: &str, head: &str) -> AppResult<String> {
    let endpoint = format!("repos/{owner}/{name}/compare/{base}...{head}?per_page=1");
    let ctx = GhRepo::Remote { owner: owner.to_string(), name: name.to_string() };
    let out = run_gh(&ctx, &["api", &endpoint])?;
    let raw: CompareRaw = serde_json::from_str(&out)
        .map_err(|e| AppError::Gh(format!("failed to parse compare: {e}")))?;
    raw.merge_base_commit
        .map(|c| c.sha)
        .ok_or_else(|| AppError::Gh("compare response missing merge_base_commit".into()))
}
```

`compare()` (`gh.rs:244-254`) is structurally unchanged — only its `CompareRaw` parse now
tolerates/ignores the extra optional field. But `files` must remain a required field there
(compare responses always carry it), so deserialization behavior for `compare()` is identical.

### 2. `src-tauri/src/commands/review.rs` — write `base_sha`

**`get_or_create_pr_target`** (`review.rs:157-194`) gains a parameter. It runs under the caller's
lock, so the subprocess call happens in the caller, not here:

```rust
/// Reuse one `target` per GitHub PR number, refreshing title/refs/head sha.
/// `merge_base_sha` is the resolved merge-base of base_ref...head (the LEFT side
/// of the three-dot PR diff); None means resolution failed/was skipped and the
/// stored value is preserved (COALESCE).
pub(crate) fn get_or_create_pr_target(
    conn: &Connection,
    repo_id: i64,
    pr_number: i64,
    info: &gh::PrInfo,
    merge_base_sha: Option<&str>,
) -> AppResult<Target>
```

- Update branch (`review.rs:172-175`) becomes:

```sql
UPDATE target SET title = ?1, base_ref = ?2, head_ref = ?3, head_sha = ?4,
                  base_sha = COALESCE(?5, base_sha)
WHERE id = ?6
```

- Insert branch (`review.rs:179-192`): replace the literal `NULL` with the `merge_base_sha` param
  (`params![..., merge_base_sha, ...]` — a `None` still inserts NULL).

**`create_review_for_pr`** (`review.rs:446-464`) — the only production caller. After
`gh::pr_view` (lock already dropped, `review.rs:460`), resolve the merge-base with the `owner`/
`name` it already has as arguments, then pass it through:

```rust
let info = gh::pr_view(&ctx, pr_number)?;
let merge_base = gh::merge_base_sha(&owner, &name, &info.base_ref, &info.head_sha).ok();
let conn = db.0.lock().unwrap();
let target = get_or_create_pr_target(&conn, repo_id, pr_number, &info, merge_base.as_deref())?;
```

**New helper** next to `gh_ctx_for_repo` (`review.rs:66-86`), mirroring its owner/name
resolution (stored columns first, `github:owner/name` sentinel fallback) but returning an option
instead of erroring, because a purely-local repo legitimately has no GitHub identity:

```rust
/// The GitHub owner/name of a repository, if it has one: the stored remote
/// columns, else parsed from the clone-less `github:owner/name` path sentinel.
/// None for purely local repos (callers skip GitHub-API work gracefully).
fn repo_owner_name(conn: &Connection, repo_id: i64) -> AppResult<Option<(String, String)>>
```

**`refresh_target_shas`** `github_pr` arm (`review.rs:217-233`): under the first lock (where
`gh_ctx_for_repo` runs, `review.rs:222-225`), also call `repo_owner_name`. After `gh::pr_view`
succeeds (lock dropped), resolve the merge-base; re-lock and persist:

```rust
let info = gh::pr_view(&ctx, number)?;
let merge_base = owner_name
    .and_then(|(o, n)| gh::merge_base_sha(&o, &n, &info.base_ref, &info.head_sha).ok());
let conn = db.0.lock().unwrap();
conn.execute(
    "UPDATE target SET title = ?1, base_ref = ?2, head_ref = ?3, head_sha = ?4,
                       base_sha = COALESCE(?5, base_sha)
     WHERE id = ?6",
    params![info.title, info.base_ref, info.head_ref, info.head_sha, merge_base, target.id],
)?;
```

Side effect worth a doc-comment line: `publish_review` calls `refresh_target_shas`
(`review.rs:867`), and the Refresh button calls it via `refresh_review` (`review.rs:275-278`), so
both now heal `base_sha` too.

**`file_source` lazy backfill** (`review.rs:495-538`): restructure the SHA resolution at
`review.rs:503-509` so a NULL LEFT on a PR target self-heals before erroring:

```rust
let sha: Option<String> = match side.as_str() {
    "LEFT" => detail.target.base_sha.clone(),
    "RIGHT" => detail.target.head_sha.clone(),
    other => return Err(AppError::Other(format!("invalid side: {other}"))),
};

// Lazy backfill: github_pr targets created before base_sha was populated (or
// whose resolution failed) store NULL. Resolve the merge-base now and persist
// it so the row heals without a manual refresh. file_source already runs gh
// under this lock (file_at_ref below), so this follows the same precedent.
let sha = match (sha, side.as_str(), detail.target.kind.as_str()) {
    (None, "LEFT", "github_pr") => {
        let resolved = repo_owner_name(&conn, detail.target.repo_id)?.and_then(|(o, n)| {
            let head = detail.target.head_sha.as_deref().unwrap_or(&detail.target.head_ref);
            gh::merge_base_sha(&o, &n, &detail.target.base_ref, head).ok()
        });
        if let Some(mb) = &resolved {
            conn.execute(
                "UPDATE target SET base_sha = ?1 WHERE id = ?2",
                params![mb, detail.target.id],
            )?;
        }
        resolved.ok_or_else(|| {
            AppError::Other("could not resolve the PR merge-base for the base side".into())
        })?
    }
    (sha, _, _) => sha.ok_or_else(|| {
        AppError::Other("this side has no source (file added or deleted)".into())
    })?,
};
```

Everything downstream (`git::show_file` attempt, `gh::file_at_ref` fallback,
`review.rs:511-537`) is unchanged and already handles a merge-base SHA fine: for a PR with a
local clone whose merge-base commit isn't checked out, the `git show` failure falls through to
the contents API (`review.rs:517-524`).

Update the stale doc comment on `file_source` (`review.rs:490-493`) — it no longer needs the
"LEFT→base" caveat changed, but drop/adjust any wording implying LEFT is local-only.

### 3. `src/components/ReviewView.tsx` — lift the local-only gate

Minimal predicate change at `ReviewView.tsx:515-522` (delete the kind check, fix the comment):

```ts
// react-diff-view's expansion math works in OLD/LEFT line numbers, so the raw
// source must be the base file. Added/deleted/binary files have no usable base
// side. For GitHub-PR targets LEFT is the merge-base blob, served via the
// backend's base_sha (lazily backfilled) + contents-API fallback.
const canExpand =
  file.type !== "add" && file.type !== "delete" && !file.isBinary;
```

No other frontend change: `ensureSource` already calls
`api.fileSource(detail.review.id, file.oldPath, "LEFT")` (`ReviewView.tsx:528`,
`src/lib/api.ts:47-48`) and already surfaces failures via `expandError`.

Widget placement (unchanged — the existing between-hunk expander simply starts appearing on PR
targets too):

```
┌─ src/lib/foo.ts ──────────────────────── [Viewed] ─┐
│ @@ -10,6 +10,8 @@                                  │
│   10  10   context …                               │
│ ── ↕ Expand 37 lines  | Expand all ──  ← now also  │
│ @@ -48,5 +50,5 @@        rendered for github_pr    │
│   48  50   context …                               │
└────────────────────────────────────────────────────┘
```

### Data flow summary

```
create_review_for_pr ── gh::pr_view ── gh::merge_base_sha(owner, name, base_ref, head_sha) ──┐
refresh_target_shas ──── gh::pr_view ── gh::merge_base_sha(…) ───────────────────────────────┤
                                                                                             ▼
                                                  target.base_sha = COALESCE(merge_base, base_sha)
                                                                                             │
ReviewView canExpand → ensureSource → api.fileSource(LEFT) → file_source                     │
        └─ if base_sha NULL && github_pr: merge_base_sha → persist → serve  ◄────────────────┘
        └─ git show <merge_base>:<path>  ‖ fallback gh::file_at_ref(owner, name, path, merge_base)
```

## Tasks

1. **gh.rs:** add `CommitRef`, extend `CompareRaw` with optional `merge_base_commit`, add
   `merge_base_sha()`. Extend `COMPARE_FIXTURE` (`gh.rs:1035-1052`) with a `merge_base_commit`
   object; add the fixture tests below. Buildable alone.
2. **review.rs:** add `repo_owner_name` helper; add the `merge_base_sha: Option<&str>` parameter
   to `get_or_create_pr_target` with the COALESCE update + insert change; update
   `create_review_for_pr` to resolve and pass it; update the two existing test call sites
   (`review.rs:1106`, `review.rs:1114`) to pass `None`; add the new target tests below.
3. **review.rs:** write `base_sha` in `refresh_target_shas`'s `github_pr` arm (COALESCE form),
   using `repo_owner_name` resolved under the first lock.
4. **review.rs:** lazy backfill + clearer LEFT error in `file_source`; refresh its doc comment.
5. **ReviewView.tsx:** lift the `kind === "local"` gate in `canExpand`; update the two comments
   (`ReviewView.tsx:515-517`).
6. **ROADMAP.md:** drop the §1 "Diff context expansion on GitHub-PR targets" bullet (repo
   convention: `docs(roadmap): drop items shipped …`).

## Test matrix

### Rust — `gh.rs` (fixture parse, no network; mirrors `compare_parses_fixture`, `gh.rs:1054-1069`)

| Test | Asserts |
|---|---|
| `compare_fixture_parses_merge_base` | extended `COMPARE_FIXTURE` (with `"merge_base_commit": {"sha": "abc123"}`) parses; `merge_base_commit.unwrap().sha == "abc123"`; `files` still parse as before |
| `compare_fixture_without_merge_base_is_none` | a fixture lacking `merge_base_commit` parses with `merge_base_commit == None` (the `#[serde(default)]` contract `compare()` relies on) |

### Rust — `commands/review.rs` (in-memory DB via `open_memory()`, no network)

| Test | Asserts |
|---|---|
| `pr_target_stores_merge_base_sha` | `get_or_create_pr_target(…, Some("mb1"))` → `base_sha == Some("mb1")` on insert |
| `pr_target_refresh_preserves_base_sha_on_none` | after the above, a second call with `None` keeps `base_sha == Some("mb1")` (COALESCE); a third call with `Some("mb2")` overwrites to `"mb2"` |
| `pr_target_is_created_then_refreshed` (existing, `review.rs:1102-1118`) | updated to pass `None`; additionally assert `base_sha.is_none()` survives create+refresh |
| `repo_owner_name_resolution` | repo seeded with `remote_owner/remote_name` → `Some(("owner","repo"))`; clone-less `github:acme/widget` path with NULL columns → `Some(("acme","widget"))`; plain local repo → `None` |

The `gh`-subprocess paths (`create_review_for_pr` resolution, `refresh_target_shas` arm,
`file_source` backfill) are covered by manual verify — consistent with how `pr_view`/`file_at_ref`
are (not) tested today.

### vitest

None new. The frontend change is the deletion of one predicate clause inside `FileReview`
(no component test exists for `ReviewView`); behavior is exercised in manual verify.

## Gates

1. `pnpm exec tsc --noEmit`
2. `pnpm build`
3. `pnpm test` (vitest run)
4. `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
5. `cargo test --manifest-path src-tauri/Cargo.toml`

Spec-specific: `cargo test --manifest-path src-tauri/Cargo.toml pr_target` and `… compare_fixture`
pass; no new migration files exist in the diff.

## Manual verify

Requires `gh auth login` and a real GitHub PR (any open PR on a repo you can read works).

1. `pnpm tauri dev`, add the GitHub repo (or use the clone-less inbox), open a PR review.
2. On a **modified** file, hover a gap between two hunks → the "Expand N lines" control now
   renders (previously absent on PR targets). Click it → real context lines appear with correct
   line numbers (compare against the file on github.com at the PR's "Files changed" view).
3. Verify persistence:
   `sqlite3 ~/.local/share/com.codereview.app/codereview.db "SELECT kind, base_sha, head_sha FROM target;"`
   → the `github_pr` row has a non-NULL `base_sha`. Cross-check it is the **merge-base**:
   `gh api repos/<o>/<n>/compare/<base_ref>...<head_sha> --jq .merge_base_commit.sha`.
4. Backfill path: `sqlite3 … "UPDATE target SET base_sha = NULL WHERE kind = 'github_pr';"`,
   restart the app (or just reopen the review), expand a gap **without** pressing Refresh →
   expansion works and `base_sha` is repopulated (re-run the SELECT).
5. Merge-base-vs-tip correctness (the three-dot caveat): pick a PR whose base branch advanced
   after branch-off (or push a commit to base). Refresh, expand context around a region the base
   moved past → lines align with the diff's old side, not the base tip.
6. Failure path: temporarily break the network (or use a PR whose base branch was deleted),
   set `base_sha` NULL as in step 4, click an expander → a readable error appears under the file
   ("could not resolve the PR merge-base…"), the app does not crash, and the review still loads.
7. Sanity on local targets: open a virtual PR, expansion still works exactly as before.

## Out of scope

- **Leading/trailing expanders** (above first hunk / below last) — separate ROADMAP §1 bullet,
  still "v1.1" (`ReviewView.tsx:917-918`).
- **`anchored_base_sha` / LEFT-side re-anchoring** (ROADMAP §2, spec 16, reserved migration 0008).
  This spec stores the target-level merge-base only; it does not track per-comment base pins.
- **Re-resolving `base_sha` when the base branch moves between refreshes** beyond what
  `refresh_target_shas` already does — no polling, no staleness detection for the base side.
- **`pr_view` changes** — do not add `baseRefOid` to the `--json` field list; it is the wrong
  value to store and is not needed.
- **Changing `compare()`'s signature or pagination behavior**, RIGHT-side expansion plumbing
  (already works via `head_sha`), and any wider `ReviewView.tsx` refactor.
