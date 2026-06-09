# Spec 17 — Capture GitHub comment IDs after publish

Prerequisite for ROADMAP §3 "Reply to existing threads and **resolve** them via the API"
(`ROADMAP.md:42`): to target our own published inline comments later, we must know the GitHub id
GitHub assigned to each one.

## Problem

The schema anticipated this and nothing fills it:

- `comment.github_comment_id INTEGER` has existed since `src-tauri/src/db/migrations/0001_init.sql:54`
  and is carried through `db/models.rs:108`/`:128` and `src/lib/types.ts:59`, but **no code path
  ever writes it** — the only writers set it to `None` (`export.rs:169` import stub, the
  `payload_comment` test fixture at `commands/review.rs:1305`).
- `publish_review` (`commands/review.rs:834-885`) posts the whole review in one
  `gh::post_review` call (`gh.rs:125-139`, REST `POST repos/{o}/{n}/pulls/{n}/reviews`) and stores
  only the **review**-level id (`review.github_review_id`, `review.rs:879-883`). The create-review
  response is a review object — it does not include the per-comment ids GitHub minted for the
  `comments[]` array, so they are simply discarded today.
- Consequence: a published local comment can never be correlated with the `PrThreadComment.database_id`
  the threads view already fetches (`gh.rs:664`, `:741`), and future API calls that need a comment id
  (reply `in_reply_to`, thread resolve) have nothing to work with.

The fix: after a successful post, fetch `GET repos/{o}/{n}/pulls/{number}/reviews/{review_id}/comments`,
match the returned comments back to the local rows we just published, and write
`comment.github_comment_id` — strictly best-effort.

## Decisions (locked)

- **Strictly best-effort.** The review is already published when capture runs; any failure
  (network, parse, zero matches) is logged with `eprintln!("[publish.capture_ids] …")` (same tag
  style as `inbox.rs:507`/`gh.rs:114`) and **never** fails or rolls back the publish. No new
  `AppError` surface, no UI error state.
- **Match key = `(path, side, line, start_line)` with exact-body tiebreak.** Anchor tuple first;
  body equality disambiguates multiple comments on the same anchor. Anything still ambiguous is
  left unmatched — `github_comment_id` stays `NULL`. Never guess.
- **Count mismatches are normal, not errors.** File-level, file-view, and lost (un-re-anchored)
  comments are folded into the review body (`body_with_file_comments`, `review.rs:756-821`) and
  have no inline counterpart; once Spec 11 lands, replies fold into root bodies too, so **only
  roots can match**. The matcher must tolerate locals without remotes and vice versa.
- **Explicit pagination loop, no `gh api --paginate`.** `--slurp` semantics vary by `gh` version;
  mirror the explicit-loop house style of `pr_review_threads` (`gh.rs:791-814`) with
  `per_page=100&page=N` (precedent: `pr_files`, `gh.rs:214`). Rationale: deterministic, testable,
  no `gh` version floor.
- **Matcher compares the body *as posted*, not the raw row.** Extract the inline-comment selection
  + body projection out of `build_publish_payload` into a shared `inline_publish_comments` helper,
  so when Spec 11 (`fold_replies`) or Spec 13 (suggestions) transform bodies at publish time, the
  matcher automatically sees the same text GitHub stored. One source of truth.
- **The capture UPDATE bypasses `ensure_draft` and does not touch `updated_at`.** The review is
  locked (`status='published'`) by design at that point; this is bookkeeping about the publish,
  not a user edit. Direct `conn.execute` is correct here.
- **No migration.** The column shipped in 0001. Migrations 0007 and 0008 stay reserved for specs
  12 and 16 — do not touch `db/mod.rs::MIGRATIONS`.
- **No frontend change.** `Comment.github_comment_id` already flows to `types.ts:59`; nothing
  renders it yet (out of scope).

## Design

### 1. `src-tauri/src/gh.rs` — fetch a review's inline comments

```rust
/// One inline comment of a submitted review, from the REST
/// `pulls/{n}/reviews/{review_id}/comments` API. Anchor fields are Option-typed
/// defensively: the matcher skips items missing side/line rather than erroring.
#[derive(Debug, Deserialize)]
pub struct ReviewComment {
    pub id: i64,
    pub path: String,
    #[serde(default)]
    pub side: Option<String>,       // "LEFT" | "RIGHT"
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub start_line: Option<i64>,    // null for single-line comments
    #[serde(default)]
    pub body: String,
}

/// All inline comments belonging to one review. Clone-less (absolute endpoint).
/// Explicit per_page/page loop; stops when a page returns fewer than 100 items.
pub fn review_comments(
    owner: &str,
    name: &str,
    number: i64,
    review_id: i64,
) -> AppResult<Vec<ReviewComment>> {
    let ctx = GhRepo::Remote { owner: owner.to_string(), name: name.to_string() };
    let mut all = Vec::new();
    let mut page = 1u32;
    loop {
        let endpoint = format!(
            "repos/{owner}/{name}/pulls/{number}/reviews/{review_id}/comments?per_page=100&page={page}"
        );
        let out = run_gh(&ctx, &["api", &endpoint])?;
        let batch: Vec<ReviewComment> = serde_json::from_str(&out)
            .map_err(|e| AppError::Gh(format!("failed to parse review comments: {e}")))?;
        let n = batch.len();
        all.extend(batch);
        if n < 100 { break; }
        page += 1;
    }
    Ok(all)
}
```

Place it next to `post_review` (`gh.rs:125-139`). Note: GitHub's docs label this endpoint's items
"legacy review comments"; current responses carry `side`/`line`/`start_line`. If live testing ever
shows them absent, the drop-in replacement is `GET repos/{o}/{n}/pulls/{number}/comments` filtered
on `pull_request_review_id == review_id` — same `ReviewComment` shape, matcher unchanged.

### 2. `src-tauri/src/commands/review.rs` — shared selection helper

Extract from `build_publish_payload` (`review.rs:709-750`); the payload builder becomes a thin map
over this:

```rust
/// The comments publish posts inline, each paired with the exact body text sent
/// to GitHub. Single source of truth shared by build_publish_payload and the
/// post-publish id capture (Spec 17), so the matcher compares like-for-like.
/// Today the posted body is `c.body` verbatim; Spec 11's fold_replies and Spec
/// 13's suggestion handling must apply their transforms HERE.
fn inline_publish_comments(detail: &ReviewDetail) -> Vec<(&Comment, String)> {
    detail
        .comments
        .iter()
        .filter(|c| {
            c.subject_type != "file"
                && c.origin != "file_view"
                && is_anchored_to(c, detail.target.head_sha.as_deref())
        })
        .map(|c| (c, c.body.clone()))
        .collect()
}
```

`build_publish_payload`'s `comments` block (`review.rs:716-739`) becomes
`inline_publish_comments(detail).iter().map(|(c, body)| { …same json!… "body": body … })` — the
filter literal appears exactly once in the file. Existing payload tests must stay green unchanged.

### 3. `src-tauri/src/commands/review.rs` — pure matcher

```rust
/// Normalized anchor of a posted inline comment. Locals normalize
/// `start_line == line` to None because build_publish_payload only sends
/// start_line for true ranges (review.rs:731-736) and GitHub returns null for
/// single-line comments.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct AnchorKey {
    path: String,
    side: String,
    line: i64,
    start_line: Option<i64>,
}

impl AnchorKey {
    fn local(c: &Comment) -> Self {
        Self {
            path: c.file_path.clone(),
            side: c.side.clone(),
            line: c.line,
            start_line: c.start_line.filter(|s| *s != c.line),
        }
    }
    /// None when the remote item lacks side/line (defensive; such items are skipped).
    fn remote(rc: &gh::ReviewComment) -> Option<Self> { /* path + side? + line? + start_line */ }
}

/// Pure matcher pairing local posted comments with the remote review comments
/// GitHub minted for them. `local` items are (comment id, anchor, body-as-posted).
/// Returns (local_comment_id, github_comment_id) pairs; each remote id is
/// consumed at most once; anything ambiguous is left out (caller leaves NULL).
fn match_review_comments(
    local: &[(i64, AnchorKey, String)],
    remote: &[gh::ReviewComment],
) -> Vec<(i64, i64)>;
```

Algorithm (per locked precedence):

1. Bucket remotes by `AnchorKey::remote` (skip items where it is `None`), preserving API order;
   each entry `(github_id, body, consumed: bool)`.
2. Bucket locals by their `AnchorKey`, preserving input order.
3. For each local bucket with a matching remote bucket:
   - **Pass 1 (body tiebreak):** for each local in order, take the first unconsumed remote in the
     bucket whose `body` is **exactly equal** (byte equality, no trimming) to the local's
     posted body → pair, mark consumed. Identical duplicates therefore zip in order.
   - **Pass 2 (singleton fallback):** if after pass 1 the bucket has **exactly one** unpaired local
     and **exactly one** unconsumed remote, pair them (covers GitHub-side body normalization,
     e.g. line-ending munging, on an otherwise unambiguous anchor).
   - Anything else stays unmatched → `github_comment_id` remains `NULL`.

### 4. `src-tauri/src/commands/review.rs` — capture + wiring

```rust
/// Write matched ids. Plain helper so the SQL is unit-testable without network.
/// Deliberately bypasses ensure_draft (review is already locked) and leaves
/// updated_at alone (bookkeeping, not a user edit).
fn store_comment_id_matches(conn: &Connection, matches: &[(i64, i64)]) -> AppResult<usize> {
    for (comment_id, gh_id) in matches {
        conn.execute(
            "UPDATE comment SET github_comment_id = ?1 WHERE id = ?2",
            params![gh_id, comment_id],
        )?;
    }
    Ok(matches.len())
}

/// Best-effort post-publish capture: fetch the inline comments GitHub created
/// for the just-posted review and store their ids on the matching local rows.
/// Errors bubble to the caller, which logs and swallows them.
fn capture_github_comment_ids(
    conn: &Connection,
    detail: &ReviewDetail,   // the post-reanchor detail the payload was built from
    owner: &str,
    name: &str,
    number: i64,
    gh_review_id: i64,
) -> AppResult<usize> {
    let remote = gh::review_comments(owner, name, number, gh_review_id)?;
    let local: Vec<(i64, AnchorKey, String)> = inline_publish_comments(detail)
        .into_iter()
        .map(|(c, body)| (c.id, AnchorKey::local(c), body))
        .collect();
    let matches = match_review_comments(&local, &remote);
    store_comment_id_matches(conn, &matches)
}
```

In `publish_review` (`review.rs:834-885`), after the `UPDATE review SET status='published' …`
(`review.rs:879-883`) and before the final `get_review_row`:

```rust
match capture_github_comment_ids(&conn, &detail, &owner, &name, number, gh_id) {
    Ok(n) => eprintln!("[publish.capture_ids] stored {n} github comment ids for review {review_id}"),
    Err(e) => eprintln!("[publish.capture_ids] best-effort capture failed for review {review_id}: {e}"),
}
```

`detail` here is already the post-reanchor reload (`review.rs:872`) the payload was built from, so
local anchors equal what was posted. The conn mutex is already held across `gh::post_review`
(`review.rs:869-876`); holding it across this second `gh` call is consistent with existing code.

### Files touched

- `src-tauri/src/gh.rs` — `ReviewComment`, `review_comments()`, fixture test.
- `src-tauri/src/commands/review.rs` — `inline_publish_comments`, `AnchorKey`,
  `match_review_comments`, `store_comment_id_matches`, `capture_github_comment_ids`, the
  `publish_review` wiring, tests.
- `ROADMAP.md` — annotate the §3 reply/resolve bullet: "GitHub comment ids are now captured at
  publish (prerequisite done)".
- No `lib.rs` change (no new command), no migration, no frontend change.

## Tasks

1. **gh.rs:** add `ReviewComment` + `review_comments()` with the page loop; fixture-parse test
   (mirror `compare_parses_fixture`, `gh.rs:1054-1069`).
2. **review.rs:** extract `inline_publish_comments` and refactor `build_publish_payload` over it;
   existing payload tests pass unchanged.
3. **review.rs:** add `AnchorKey` + `match_review_comments` (pure) with the unit-test battery below.
4. **review.rs:** add `store_comment_id_matches` + `capture_github_comment_ids`; wire into
   `publish_review` with the two log lines; DB test for the store helper.
5. **ROADMAP.md:** annotate §3.

Steps 1–3 are independently buildable commits; 4 depends on 1–3.

## Test matrix

Rust — `gh.rs` `mod tests`:

| Test | Asserts |
|---|---|
| `review_comments_parse_fixture` | static JSON array with one full item (`id`, `path`, `side:"RIGHT"`, `line:5`, `start_line:3`, `body`) and one minimal item (`side`/`line`/`start_line` null) deserializes; `Option` fields land as `None` |

Rust — `commands/review.rs` `mod tests` (reuse `detail_with`/`payload_comment` fixtures at
`review.rs:1262-1308`; matcher tests are pure, no DB/network):

| Test | Asserts |
|---|---|
| `inline_publish_comments_matches_payload_filter` | excludes `subject_type='file'`, `origin='file_view'`, and not-anchored-to-head comments; posted body equals `c.body` |
| `match_singleton_anchor_pairs_despite_body_drift` | 1 local + 1 remote, same anchor, different bodies → paired (pass 2) |
| `match_body_tiebreak_on_shared_anchor` | 2 locals same anchor with bodies A/B, 2 remotes same anchor bodies B/A → each pairs with its body twin |
| `match_identical_duplicates_zip_in_order` | 2 locals same anchor + same body, 2 remotes same → paired in order, distinct gh ids, each remote consumed once |
| `match_ambiguous_bucket_left_unmatched` | 2 locals same anchor, 2 remotes, no body equality → empty result |
| `match_tolerates_count_mismatch` | 2 locals, 1 remote matching the first → exactly one pair; second local unmatched (folded-comment scenario) |
| `match_normalizes_single_line_start` | local `start_line == Some(line)` matches remote `start_line: None` |
| `match_multiline_range_key` | local `start_line=Some(3), line=5` matches remote `start_line:3, line:5`, not a single-line remote at line 5 |
| `match_skips_remote_without_anchor` | remote with `side`/`line` `None` is never paired |
| `store_comment_id_matches_writes_only_matched_rows` | `open_memory()` + `seed_comment` two rows; store one pair → that row's `github_comment_id` set, the other stays `NULL`, `updated_at` unchanged |

The network path (`capture_github_comment_ids`, `review_comments` against live GitHub) is covered
by manual verify only — same policy as `gh::post_review` (Spec 03).

## Gates

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test`
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

Spec-specific: `git diff --stat src-tauri/src/db/` shows no changes (no migration, no model change).

## Manual verify

Needs a scratch PR on a repo you can write to, `gh auth login` done.

1. Run `pnpm tauri dev` **from a terminal** (the capture logs go to stderr).
2. Open the scratch PR as a review; add two inline comments on different lines, one multi-line
   range comment, and one file-level comment. Publish.
3. Terminal shows `[publish.capture_ids] stored 3 github comment ids for review <id>` (the
   file-level comment is body-folded, so 3 not 4).
4. Inspect the DB (Linux dev: `sqlite3 ~/.local/share/com.codereview.app/<db-file>`):
   `SELECT id, file_path, line, start_line, github_comment_id FROM comment WHERE review_id = <id>;`
   — the three inline rows have non-NULL ids, the file-level row stays NULL.
5. Cross-check against GitHub:
   `gh api repos/<o>/<n>/pulls/<pr>/reviews/<github_review_id>/comments --jq '.[] | {id, path, line, start_line}'`
   — ids and anchors line up with step 4.
6. Failure path: publish another review with networking dropped *after* step 2's post would be
   contrived; instead confirm the code path by temporarily pointing `review_comments` at a bogus
   endpoint in a scratch build — publish still succeeds and logs
   `[publish.capture_ids] best-effort capture failed …`. Revert.

## Out of scope

- Any UI surface for `github_comment_id` (badges, links to the GitHub comment).
- The ROADMAP §3 features this enables: replying to existing threads (`in_reply_to`), resolving
  threads via API, PENDING-review flows.
- Backfilling ids for reviews published before this spec (they stay NULL; no retry command).
- Matching file-level / file-view / lost comments or (post-Spec 11) reply rows — none are posted
  inline, by construction.
- Content-similarity or fuzzy matching; switching to the PR-wide comments endpoint (noted fallback
  only).
- Any DB migration (0007/0008 remain reserved for specs 12/16).
