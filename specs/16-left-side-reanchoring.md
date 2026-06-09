# Spec 16 — Re-anchor LEFT/base-side comments

Implements ROADMAP §2 "Re-anchor LEFT/base-side comments" (`ROADMAP.md:35-38`) — the only
remaining item in that section. **Wave 3: implement after specs 10, 11 and 12 are merged**; line
anchors below cite today's `main` plus the shapes those specs introduce (called out inline where
they differ).

## Problem

RIGHT-side comments re-anchor when the head moves (`anchor.rs` + `reanchor_review_comments`,
`src-tauri/src/commands/review.rs:292-403`). LEFT/base-side comments have no equivalent:

- There is no per-comment base pin. The `comment` table has `anchored_head_sha` but no
  `anchored_base_sha` (`src-tauri/src/db/migrations/0001_init.sql:43-58`); migration **0008 is
  reserved for exactly this column** (spec 10 "Out of scope", spec 12 "Decisions").
- The re-anchor helper filters `c.side == "RIGHT"` (`review.rs:306`) and walks only
  `anchored_head_sha` (`review.rs:314`), so LEFT comments are never moved when the base /
  merge-base advances.
- The publish gate `is_anchored_to` (`review.rs:702-707`) compares **every** comment — LEFT
  included — against `target.head_sha`. Two bugs in one: a stale-LEFT comment whose base moved
  still posts inline if its `anchored_head_sha` happens to match the head (422 / mislanded line),
  and a perfectly valid LEFT comment gets folded into the body just because the **head** moved
  (LEFT line numbers live in base coordinates; the head is irrelevant to them).
- The frontend "outdated" badge (`src/components/ReviewView.tsx:1052-1055`) and the header
  "head moved" banner predicate (`ReviewView.tsx:244-249`) likewise compare
  `anchored_head_sha !== head_sha` regardless of side.
- Coordinate-system gap for local three-dot targets: the displayed LEFT side of a three-dot diff
  is the file at the **merge-base** (`git.rs::diff`, `git.rs:126-133`), but
  `get_or_create_local_target` stores `base_sha = rev_parse(base_ref)` — the base **tip**
  (`review.rs:126`) — as does `refresh_target_shas`' local arm (`review.rs:245`). Pinning LEFT
  comments to that value would be wrong whenever base diverged from the merge-base. (Spec 10
  already fixed this for `github_pr` targets: their `base_sha` is the merge-base.)

For PR targets this spec **depends on spec 10** (`target.base_sha` populated with the merge-base,
plus the lazy backfill). Local targets work standalone.

## Decisions (locked)

- **Migration 0008 = `0008_comment_anchored_base_sha.sql`**, exactly one statement:
  `ALTER TABLE comment ADD COLUMN anchored_base_sha TEXT`. Append-only per house rules.
- **The base pin is backend-resolved, no new `add_comment` parameter.** When `side == "LEFT"`,
  the insert path reads the review's `target.base_sha` under the same lock/connection and stores
  it as `anchored_base_sha`; RIGHT comments store NULL. Rationale: zero API churn
  (`api.addComment`, `src/lib/api.ts:88-97`, unchanged), and the pin is read from the same DB
  snapshot `ensure_draft` just saw. (`anchored_head_sha` stays frontend-supplied as today,
  `ReviewView.tsx:681` / `FileViewPane.tsx:152` — for LEFT rows it is stored but no longer
  consulted by any predicate.)
- **`target.base_sha` for local targets becomes the diff's true old side**: `git merge-base
  base_ref head_ref` when `three_dot`, `rev_parse(base_ref)` when two-dot — in both
  `get_or_create_local_target` and `refresh_target_shas`. Rationale: the invariant this whole
  spec rests on is "*`anchored_base_sha` pins the coordinate system LEFT line numbers live in*";
  spec 10 established it for PRs (merge-base), this completes it for local three-dot targets.
  Side benefit: `file_source`'s LEFT expansion stops being offset on diverged three-dot local
  targets. When base is an ancestor of head (the overwhelmingly common case) the value is
  identical to today.
- **No new remap algorithm and no `remap_left_line` variant.** `anchor::remap_right_line`
  (`src-tauri/src/anchor.rs:89-127`) already maps an **old-revision line to the new revision**
  through a patch's hunks — its input is matched against `hunk.old_start`/`old_len`. "right" in
  the name refers to the comment's PR side in its first use case, not the patch side. **Rename it
  to `remap_line`** (mechanical: callers live only in `review.rs` and `anchor.rs` tests) with a
  doc comment fixing the direction contract. See the worked example below.
- **Diff direction is always pin → current**: `(anchored_base_sha, target.base_sha)` as
  `(old, new)` — same argument order as the RIGHT pass (`review.rs:341`, `review.rs:350-355`).
- **`reanchor_review_comments` becomes two passes over one shared helper** (RIGHT/head pass then
  LEFT/base pass), parameterized by side. `ReanchorResult` keeps its exact shape and aggregates
  both passes — no API/UI churn, the existing toast (`ReviewView.tsx:237`) stays accurate.
- **Missing current SHA = skip, never error.** A pass whose current SHA is NULL (`head_sha`
  unresolved, or `base_sha` NULL on a PR row spec 10 hasn't backfilled yet) counts each of its
  pinned candidates as `skipped_no_change` and touches nothing. (This also makes the RIGHT pass's
  current `head_sha = None` early-return consistent; the count change there is cosmetic.)
- **`is_anchored_to` becomes side-aware** and takes `&Target`: LEFT compares
  `anchored_base_sha` vs `target.base_sha`, RIGHT compares `anchored_head_sha` vs
  `target.head_sha`. A NULL pin is still treated as anchored (legacy rows keep today's
  behaviour, same contract as `review.rs:700-704`). Deliberate behaviour change: LEFT comments
  stop folding into the body just because the **head** moved.
- **Replies ride along, spec-11 style.** Both passes consider roots only
  (`parent_id IS NULL`); every successful root UPDATE cascades one `WHERE parent_id = root.id`
  UPDATE writing the same `line`/`start_line`/pin column. `add_comment_impl`'s reply-inheritance
  column list (spec 11) gains `anchored_base_sha`.
- **Clone-less PR caveat accepted**: the REST compare endpoint is three-dot only, so the
  base→base patch is exact only when the old merge-base is an ancestor of the new one — true
  whenever the base branch simply advances. Exotic rebases may misclassify a comment as
  `Lost`; that degrades to the existing fold-into-body path, never a 422. Local clones use the
  literal two-dot `git::diff_shas_path`.
- **Frontend staleness predicate is extracted to a pure helper** (`src/lib/staleness.ts`,
  vitest-able, mirroring the `threads.ts` precedent from spec 11) and used by both the header
  banner and the per-comment badge. Banner copy generalizes from "head moved" to "target moved".
- **`FreshnessResult` is unchanged** (no `base_moved` field). The banner already recomputes from
  comment rows after the `["review", id]` invalidation; the refresh toast stays head-only.

## Design — direction worked example (the crux)

A LEFT comment's `line` is valid in the file at `anchored_base_sha` (call it **B1**). The base
advances to **B2** (`target.base_sha` after refresh). We fetch the patch **B1 → B2**; the
comment's line lives on the **old** side of that patch and we need its position on the **new**
side. That is exactly what `remap_line` (né `remap_right_line`) computes — the function walks
`old_start`/`old_len` for the input and emits `new`-side numbers (`anchor.rs:93-126`).

B2 inserts 3 lines above old line 50; the comment sits at LEFT line 80:

```
git diff B1..B2 →  @@ -50,0 +50,3 @@
                   +a
                   +b
                   +c
```

`remap_line(80, hunks)`: `80 < old_start(50)`? No. Inside the hunk's old range? No
(`old_len == 0` covers no old line). Past the hunk: `delta += new_len - old_len = +3` →
`Shifted(83)`. Correct — inserting 3 lines above line 80 pushes it to 83, so we UPDATE
`line = 83, anchored_base_sha = B2`.

**The trap** is fetching the diff reversed (`B2..B1`): the same edit then reads
`@@ -50,3 +50,0 @@` (three deletions), 80 would map to 77, and new lines 50–52 would falsely be
`Lost`. Hence the locked pin→current argument order, asserted by a dedicated test.

Per-line behaviours (all already implemented by `remap_line`, re-used verbatim):

| Old-side line L | Result |
|---|---|
| before all hunks | `Shifted(L)` |
| in an untouched gap after k net-added lines | `Shifted(L + k)` |
| context line inside a hunk | `Shifted` to its tracked new line |
| deleted/replaced line | `Lost` |
| after all hunks | `Shifted(L + total delta)` |
| range (`start_line`,`line`): either endpoint `Lost` | whole comment `Lost` (`review.rs:373-392` pattern) |

## Design — files & contracts

### 1. NEW `src-tauri/src/db/migrations/0008_comment_anchored_base_sha.sql`

```sql
-- The base/merge-base SHA a LEFT-side comment's line numbers are valid against
-- (the old side of the diff the user was looking at). NULL on RIGHT comments and
-- on rows created before this column existed; NULL is treated as "anchored".
ALTER TABLE comment ADD COLUMN anchored_base_sha TEXT;
```

Append to `MIGRATIONS` (`src-tauri/src/db/mod.rs:16-23`, after spec 12's
`0007_comment_resolved.sql`):

```rust
include_str!("migrations/0008_comment_anchored_base_sha.sql"),
```

### 2. `src-tauri/src/db/models.rs` — carry the column

`Comment` (`models.rs:92-111`) gains `pub anchored_base_sha: Option<String>` next to
`anchored_head_sha`; `from_row` (`models.rs:114-132`) adds
`anchored_base_sha: row.get("anchored_base_sha")?`. All comment reads use `SELECT *` + named
columns (`review.rs:93-95`, `review.rs:608-616`), so no SQL changes.

### 3. `src-tauri/src/git.rs` — merge-base helper

```rust
/// First merge-base of two revs (`git merge-base a b`).
pub fn merge_base(repo: &Path, a: &str, b: &str) -> AppResult<String> {
    Ok(run_git(repo, &["merge-base", a, b])?.trim().to_string())
}
```

### 4. `src-tauri/src/commands/review.rs` — local `base_sha` = the diff's old side

Factor one resolver used by both writers:

```rust
/// The SHA whose file contents the LEFT side of this local diff shows: the
/// merge-base for three-dot (GitHub-PR semantics), the base tip for two-dot.
fn local_old_side_sha(repo: &Path, base_ref: &str, head_ref: &str, three_dot: bool) -> Option<String> {
    if three_dot {
        git::merge_base(repo, base_ref, head_ref).ok()
    } else {
        git::rev_parse(repo, base_ref).ok()
    }
}
```

- `get_or_create_local_target` (`review.rs:126`): `let base_sha = local_old_side_sha(...)`.
- `refresh_target_shas` local arm (`review.rs:244-246`): same substitution (it has
  `target.three_dot` in scope). The UPDATEs (`review.rs:139-142`, `review.rs:248-251`) are
  unchanged.

### 5. `src-tauri/src/anchor.rs` — rename, doc, no behaviour change

`remap_right_line` → `remap_line`; module doc (`anchor.rs:1-2`) and fn doc state the contract:
*"maps a line valid in the patch's OLD revision to the NEW revision; used for RIGHT comments
through a head→head patch and LEFT comments through a base→base patch."* Update the two
`review.rs` call sites (`review.rs:376`, `review.rs:384`) and the test call sites. `Remap`,
`FileHunks`, `parse_file_patch` untouched.

### 6. `src-tauri/src/commands/review.rs` — two-pass re-anchor

```rust
/// Which anchor a re-anchor pass operates on: the comment side it selects, the
/// pin column it reads/advances, and the target SHA it maps onto.
enum AnchorPass { Head, Base }

impl AnchorPass {
    fn comment_side(&self) -> &'static str;                      // "RIGHT" | "LEFT"
    fn pin<'a>(&self, c: &'a Comment) -> Option<&'a str>;        // anchored_head_sha | anchored_base_sha
    fn current<'a>(&self, t: &'a Target) -> Option<&'a str>;     // head_sha | base_sha
    fn root_update_sql(&self) -> &'static str;                   // UPDATE … WHERE id = ?5
    fn reply_update_sql(&self) -> &'static str;                  // UPDATE … WHERE parent_id = ?5 (spec 11 cascade)
}

fn reanchor_pass(
    conn: &Connection,
    detail: &ReviewDetail,
    pass: AnchorPass,
    result: &mut ReanchorResult,
) -> AppResult<()>;

fn reanchor_review_comments(conn: &Connection, detail: &ReviewDetail) -> AppResult<ReanchorResult> {
    let mut result = ReanchorResult { reanchored: 0, lost: 0, skipped_no_change: 0 };
    reanchor_pass(conn, detail, AnchorPass::Head, &mut result)?;
    reanchor_pass(conn, detail, AnchorPass::Base, &mut result)?;
    Ok(result)
}
```

`reanchor_pass` is today's `review.rs:292-403` body (plus spec 11's roots-only filter and reply
cascade), generalized:

- Candidates: `c.side == pass.comment_side() && c.subject_type == "line" && c.origin != "file_view"
  && c.parent_id.is_none()`.
- `let Some(current) = pass.current(&detail.target)` — else every **pinned** candidate counts as
  `skipped_no_change`, return Ok (locked decision).
- Bucketing by `(pin, file_path)` with `pin = pass.pin(c)`: `None` or `== current` →
  `skipped_no_change`; else group (`review.rs:309-322` unchanged otherwise).
- Patch fetch per group is **identical code** for both passes — only the SHAs differ:
  clone-less (`detail.repo_path.starts_with("github:")`) → `gh::compare(&owner, &name,
  pinned_sha, current)` (`gh.rs:244-254`, reused as-is); local →
  `git::diff_shas_path(Path::new(&detail.repo_path), pinned_sha, current, file_path)`
  (`git.rs:144-160`). Missing-patch / empty-diff semantics unchanged (`review.rs:358-365`).
- Remap with `anchor::remap_line` exactly as `review.rs:373-392` (range endpoints independently;
  either `Lost` → whole comment `Lost`).
- Root UPDATE per pass:

  ```sql
  -- AnchorPass::Head (today's review.rs:395)
  UPDATE comment SET line = ?1, start_line = ?2, anchored_head_sha = ?3, updated_at = ?4 WHERE id = ?5
  -- AnchorPass::Base
  UPDATE comment SET line = ?1, start_line = ?2, anchored_base_sha = ?3, updated_at = ?4 WHERE id = ?5
  ```

  followed by the same statement with `WHERE parent_id = ?5` (spec 11 cascade). `Lost` roots stay
  untouched, replies included.

Refresh the doc comments that say "RIGHT-side" (`review.rs:289-291`, `review.rs:405-406`).
`reanchor_comments` (`review.rs:407-413`), `refresh_review`, and `publish_review`'s
refresh→reanchor→payload sequence (`review.rs:865-872`) need **no changes** — spec 10's
`refresh_target_shas` already re-resolves `base_sha` for PRs, and step 4 above does for locals,
so the LEFT pass always sees a fresh `current`.

### 7. `src-tauri/src/commands/review.rs` — pin on create, side-aware publish gate

**`add_comment`** (`review.rs:894-921`; post-spec-11 this body lives in `add_comment_impl`):
for root comments with `side == "LEFT"`, resolve the pin before the INSERT —

```rust
let anchored_base_sha: Option<String> = if side == "LEFT" {
    conn.query_row(
        "SELECT t.base_sha FROM target t JOIN review r ON r.target_id = t.id WHERE r.id = ?1",
        params![review_id],
        |r| r.get(0),
    )?
} else {
    None
};
```

and add the column to the INSERT (`review.rs:910-915`). Reply branch (spec 11): add
`anchored_base_sha` to the columns copied from the parent row. `add_file_comment` /
`add_file_view_comment` are untouched (never posted inline, never re-anchored).

**`is_anchored_to`** (`review.rs:702-707`) becomes:

```rust
/// Whether a comment is pinned to the SHA its side's line numbers are valid
/// against (LEFT → target.base_sha, RIGHT → target.head_sha) and so safe to
/// post inline. A NULL pin is treated as anchored — legacy rows keep today's
/// behaviour.
fn is_anchored_to(c: &Comment, target: &Target) -> bool {
    let (pin, current) = if c.side == "LEFT" {
        (c.anchored_base_sha.as_deref(), target.base_sha.as_deref())
    } else {
        (c.anchored_head_sha.as_deref(), target.head_sha.as_deref())
    };
    match pin {
        None => true,
        Some(sha) => Some(sha) == current,
    }
}
```

Both call sites pass `&detail.target` instead of `detail.target.head_sha.as_deref()`:
`build_publish_payload` (`review.rs:722`) and `body_with_file_comments`' lost-comments filter
(`review.rs:802`). Net effect: stale-LEFT comments (pin ≠ current base) fold into the
"Comments that could not be re-anchored" section; fresh-LEFT comments post inline with
`side: "LEFT"` even when the head moved.

### 8. Frontend

**`src/lib/types.ts`** — `Comment` (`types.ts:46-62`) gains
`anchored_base_sha: string | null;` after `anchored_head_sha`.

**NEW `src/lib/staleness.ts`** (pure; vitest like spec 11's `threads.ts`):

```ts
import type { Comment } from "./types";

/** The pin SHA relevant to this comment's side (LEFT → base, RIGHT → head). */
export function anchorPin(c: Comment): string | null {
  return c.side === "LEFT" ? c.anchored_base_sha : c.anchored_head_sha;
}

/** True when the comment was anchored to a SHA its side has since moved past,
 *  so its line may no longer point at the code it was written against. */
export function isCommentOutdated(
  c: Comment,
  baseSha: string | null,
  headSha: string | null,
): boolean {
  const pin = anchorPin(c);
  const current = c.side === "LEFT" ? baseSha : headSha;
  return !!pin && !!current && pin !== current;
}
```

**`src/components/ReviewView.tsx`**:

- Header banner predicate (`:244-249`) →
  `const anchorsStale = detail.comments.some((c) => isCommentOutdated(c, target.base_sha, target.head_sha));`
  Banner (`:286-306`): badge text `⚠ target moved`, title "The base or head has moved since some
  comments were written…". Button/copy otherwise unchanged.
- `CommentItem` (`:1022-1132`; rendered via spec 11's `ThreadItem`) gains a
  `baseSha: string | null` prop next to `headSha`; the `outdated` const (`:1052-1055`) becomes
  `isCommentOutdated(comment, baseSha, headSha)`, and the badge (`:1067-1077`) shows
  `anchorPin(comment)!.slice(0, 7)`.
- Thread `baseSha={detail.target.base_sha}` alongside every existing
  `headSha={detail.target.head_sha}`: the widget loop's `LineWidget`/`ThreadItem` (`:711`), the
  file-comments block (`:788`), `FileBody`'s prop (`:814`) down to its orphan rendering, and spec
  11's `ThreadItem` signature.

**`src/components/FileViewPane.tsx`**: pass `baseSha={detail.target.base_sha}` at the
`LineWidget` (`:171`) and orphan `CommentItem` (`:227-230`) sites for prop completeness (the pane
is RIGHT-only, `:100`, so behaviour is unchanged).

Badge sketch (only the pin source changes for LEFT):

```
┌ comment-item (side=LEFT) ────────────────────────────────┐
│ [outdated · b1c2d3e]  body text …                  [🗑]  │
│      ▲ anchored_base_sha, shown only while it differs    │
│        from target.base_sha; cleared by Re-anchor        │
└──────────────────────────────────────────────────────────┘
```

### 9. `ROADMAP.md`

Rewrite the §2 bullet (`ROADMAP.md:35-38`): LEFT re-anchoring ships; keep only the
"content/context-based anchoring as an alternative to line-mapping" idea
(`docs(roadmap): drop items shipped …` convention).

## Tasks

1. **git.rs:** `merge_base()` + temp-repo test. Buildable alone.
2. **Migration:** `0008_comment_anchored_base_sha.sql`, `MIGRATIONS` append, `models.rs`
   `Comment` field + `from_row`.
3. **review.rs:** `local_old_side_sha` used by `get_or_create_local_target` +
   `refresh_target_shas`; tests (ancestor base unchanged, diverged three-dot → merge-base).
4. **anchor.rs:** rename `remap_right_line` → `remap_line`, doc fix, direction worked-example
   test; mechanical call-site updates in `review.rs`.
5. **review.rs:** LEFT pin in `add_comment_impl` (root resolve + reply inheritance); extend the
   `seed_comment_anchored` test-helper family with an `anchored_base_sha` variant; tests.
6. **review.rs:** `AnchorPass` + `reanchor_pass` refactor (RIGHT pass = pure refactor, existing
   tests stay green), then the Base pass + reply cascade; tests.
7. **review.rs:** side-aware `is_anchored_to(&Comment, &Target)` + both call sites; payload
   tests.
8. **Frontend:** `types.ts` field; `staleness.ts` + vitest; `ReviewView.tsx` banner +
   `CommentItem`/`ThreadItem` `baseSha` threading; `FileViewPane.tsx` pass-through.
9. **ROADMAP.md** §2 rewrite.

Each step is an independently committable unit; 1–3 are prerequisites for 5–7.

## Test matrix

### Rust — `anchor.rs` (renamed function; existing matrix already covers before/inside/after
hunks, deleted line, multi-hunk shifts, ranges — `anchor.rs:129-218`)

| Test | Asserts |
|---|---|
| (renames) `insertion_above`, `deletion_above`, `inside_replaced_is_lost`, `context_inside_hunk`, `multi_hunk_delta`, `past_all_hunks`, `multi_line_range_caller_treats_as_lost` | unchanged behaviour under the new `remap_line` name |
| `old_to_new_direction_worked_example` | patch `@@ -50,0 +50,3 @@` (B1→B2): `remap_line(80) == Shifted(83)`; the reversed patch `@@ -50,3 +50,0 @@` gives `Shifted(77)` — documents why the caller must always diff pin→current |

### Rust — `git.rs` (temp repos, mirroring `diff_shas_*` tests at `git.rs:339-350`)

| Test | Asserts |
|---|---|
| `merge_base_of_diverged_branches` | branch `feat` off `main`, both advance → `merge_base(main, feat)` == the fork commit; equals `rev_parse(main~1)` of the fixture |

### Rust — `commands/review.rs` (extend `fixture_repo_insert_and_replace`, `review.rs:1633-1648`:
H1 = old base **B1**, H2 = current base **B2**; pin LEFT comments to B1, set `target.base_sha = B2`)

| Test | Asserts |
|---|---|
| `local_target_base_sha_is_merge_base_when_three_dot` | diverged-base fixture: `get_or_create_local_target(three_dot=true)` and `refresh_target_shas` store the merge-base, not the base tip; two-dot stores the tip |
| `add_comment_left_pins_target_base_sha` | `add_comment_impl(side="LEFT")` → `anchored_base_sha == target.base_sha`; RIGHT → `None` |
| `reply_inherits_anchored_base_sha` | reply to a LEFT root copies the pin (spec 11 inheritance list) |
| `left_reanchor_shifts_comment_and_advances_base_sha` | LEFT line 2 ("beta") pinned B1 → `line == 3`, `anchored_base_sha == B2`, `reanchored == 1`; `anchored_head_sha` untouched |
| `left_reanchor_replaced_line_is_lost` | LEFT line 3 ("gamma") → row untouched, pin stays B1, `lost == 1` |
| `left_reanchor_range_moves_both_endpoints` | LEFT `start_line=1, line=2` → `start_line=1, line=3`; a range with one endpoint on "gamma" → whole comment `Lost` |
| `left_pass_skips_null_pin_and_current_pin` | pin `None` and pin `== B2` both count `skipped_no_change`, rows untouched |
| `left_pass_noop_when_target_base_sha_null` | `target.base_sha = NULL` + LEFT comment pinned B1 → no error, `skipped_no_change == 1`, row untouched |
| `passes_do_not_cross_sides` | one stale RIGHT (head pass moves it) + one current LEFT in the same review → only the RIGHT row changes; counts aggregate across passes |
| `left_reanchor_moves_replies_with_root` | LEFT root+reply pinned B1 → both rows at the new line with `anchored_base_sha == B2`; `reanchored == 1` (roots only, spec 11 contract) |
| `publish_gate_is_side_aware` | `build_publish_payload`: stale-LEFT comment (pin ≠ `target.base_sha`) absent from `comments[]` and listed in the "could not be re-anchored" body section; fresh-LEFT included with `"side": "LEFT"`; a LEFT comment with a **stale `anchored_head_sha` but current base pin still posts inline** (the behaviour change) |
| `migrations_apply_in_order` (existing, `db/mod.rs:72`) | passes with 8 entries — no new test needed, just must stay green |

### vitest — `src/lib/staleness.test.ts`

| Case | Asserts |
|---|---|
| RIGHT stale head | `isCommentOutdated` true when `anchored_head_sha !== headSha` |
| LEFT stale base | true when `anchored_base_sha !== baseSha`, **regardless of head** |
| LEFT current base, stale head | false (head irrelevant to LEFT) |
| NULL pin / NULL current | false in all combinations; `anchorPin` picks the side-correct column |

## Gates

1. `pnpm exec tsc --noEmit`
2. `pnpm build`
3. `pnpm test` (vitest run)
4. `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
5. `cargo test --manifest-path src-tauri/Cargo.toml`

Spec-specific: the diff adds exactly one migration file (`0008_…`) and one `MIGRATIONS` entry;
`grep -rn "remap_right_line" src-tauri/src` returns nothing.

## Manual verify

Local target (standalone, no GitHub needed):

1. Scratch repo: `main` with a multi-line file; branch `feature` off it that deletes/edits some
   lines. `pnpm tauri dev`, add the repo, open the virtual PR `main...feature` (three-dot).
2. Comment on a **LEFT** (red/deleted) line. Check the pin:
   `sqlite3 ~/.local/share/com.codereview.app/codereview.db "SELECT side, line, anchored_base_sha FROM comment;"`
   → non-NULL `anchored_base_sha` equal to `git merge-base main feature`.
3. On `main`, commit an insertion **above** the commented region; in the app hit **Refresh** →
   the "⚠ target moved" banner appears and the LEFT comment shows `outdated · <B1-prefix>`.
4. **Re-anchor comments** → the LEFT comment's badge clears and it sits beside the same code
   (line number shifted by the insertion); the SELECT now shows the new merge-base.
5. Repeat with a `main` commit that **rewrites the commented line** → comment stays put, toast
   reports `1 could not be moved`, badge persists.
6. Reply check (post-spec-11): add a reply to a LEFT comment before step 3 — after re-anchoring,
   root and reply moved together.
7. RIGHT regression: a head-side comment still re-anchors exactly as before.

PR target (needs `gh auth login` + a scratch PR you can push to):

8. Open a PR review, comment on a LEFT line, push a commit to the PR's **base branch**, Refresh →
   banner; Re-anchor → LEFT comment moves; verify `anchored_base_sha` equals
   `gh api repos/<o>/<n>/compare/<base>...<head_sha> --jq .merge_base_commit.sha`.
9. Publish with one un-anchorable LEFT comment → the GitHub review body carries it under
   "Comments that could not be re-anchored"; anchored LEFT comments appear inline on the old
   side; no 422.

## Out of scope

- **Content/context-based anchoring** (matching `diff_hunk` text instead of line mapping) — stays
  on the ROADMAP as the remaining §2 idea.
- **Backfilling `anchored_base_sha` for pre-0008 LEFT rows.** NULL pins are treated as anchored
  (same legacy contract as `anchored_head_sha`); they heal naturally only if re-created.
- **Two-dot REST compare for clone-less base→base diffs** — the API is three-dot only; the
  ancestor caveat is accepted (see Decisions).
- **`FreshnessResult` changes / a base-moved toast** — the banner self-computes from comments.
- **`add_file_comment` / `add_file_view_comment` pins** — never posted inline, never re-anchored.
- **Per-side counts in `ReanchorResult`** and any other API-shape change.
- **Spec 10's lazy `base_sha` backfill in `file_source`** — reused, not modified.
