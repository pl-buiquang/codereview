# Spec 01 — Robust re-anchoring (RIGHT-side, context-precise)

Implements ROADMAP §2. Maps a comment's `(side, line[, start_line])` from its
`anchored_head_sha` to the target's current `head_sha` using the intervening diff. See Spec 00 for
the dependency graph and shared-primitive contract.

## 1. Pure remap module — `src-tauri/src/anchor.rs` (NEW)

No I/O. Add `mod anchor;` to `lib.rs`. Operates on **one file's** unified-diff `patch` describing
`anchored_head_sha` (OLD = `-` side) → current `head_sha` (NEW = `+` side).

There is **no diff crate** in `Cargo.toml`; hand-write the parser. It must retain each hunk body
line's kind so we can map lines *inside* a hunk, not just gaps between hunks.

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum Remap { Shifted(i64), Lost }

#[derive(Debug)]
struct Hunk {
    old_start: i64,
    old_len: i64,
    new_start: i64,
    new_len: i64,
    // Per body line in order: ' ' context (advances old+new), '-' delete (advances old),
    // '+' add (advances new). Store only the kinds; line text is not needed for remap.
    lines: Vec<u8>, // b' ' | b'-' | b'+'
}

#[derive(Debug, Default)]
pub struct FileHunks { hunks: Vec<Hunk> }

pub fn parse_file_patch(patch: &str) -> FileHunks;
pub fn remap_right_line(line: i64, hunks: &FileHunks) -> Remap;
```

### Parsing

- Ignore everything until the first hunk header. Accept patches that begin at `@@` (the per-file
  `patch` returned by `gh api .../compare`) **and** full `git diff` output (skip `diff --git`,
  `index`, `---`, `+++`, `\ No newline…`, `Binary files…`, rename/mode lines).
- Hunk header regex/parse: `@@ -<oldStart>[,<oldLen>] +<newStart>[,<newLen>] @@ …`. Missing length
  defaults to `1`. (Hand-parse; do not add a regex crate unless one is already a dependency.)
- Body lines until the next `@@`/non-body line: first char `' '`→context, `'-'`→delete, `'+'`→add.

### `remap_right_line(L)` algorithm

Walk hunks in order, tracking `delta` = cumulative `(new_len - old_len)` of **fully-passed** hunks.

1. `L < hunk.old_start` (untouched gap before this hunk) → `Shifted(L + delta)`.
2. `hunk.old_start <= L <= hunk.old_start + hunk.old_len - 1` (inside this hunk's old range):
   walk the hunk body tracking `(old_ln = old_start, new_ln = new_start)`:
   - `' '` → if `old_ln == L` return `Shifted(new_ln)`; else `old_ln++`, `new_ln++`.
   - `'-'` → if `old_ln == L` return `Lost`; else `old_ln++`.
   - `'+'` → `new_ln++`.
3. Otherwise advance `delta += (new_len - old_len)` and continue to the next hunk.
4. Past all hunks → `Shifted(L + delta)`.

`old_len == 0` hunks (pure insertion) cover no old line, so step-1's `L < old_start` / past-hunk
logic handles them via `delta`.

### Multi-line ranges

The caller remaps `start_line` and `line` independently with `remap_right_line`. If **either** is
`Lost`, the whole comment is `Lost` (never half-move a range).

## 2. Two-SHA diff capability

### `git::diff_shas` — `src-tauri/src/git.rs`

```rust
/// Plain two-dot diff between two commits (literal line evolution old→new).
/// Two-dot, NOT three-dot: merge-base semantics are irrelevant here.
pub fn diff_shas(repo: &Path, old_sha: &str, new_sha: &str) -> AppResult<String> {
    run_git(repo, &["diff", "--no-color", &format!("{old_sha}..{new_sha}")])
}
```

### `gh::compare` — `src-tauri/src/gh.rs`

```rust
#[derive(Debug, Deserialize)]
pub struct ComparedFile {
    pub filename: String,
    #[serde(default)]
    pub patch: Option<String>,   // absent for binary/large files
    pub status: String,
}

/// `gh api repos/{owner}/{name}/compare/{base}...{head}` → changed files w/ per-file patches.
pub fn compare(owner: &str, name: &str, base: &str, head: &str) -> AppResult<Vec<ComparedFile>>;
```

Build a `GhRepo::Remote { owner, name }` like `post_review`/`file_at_ref` do, then
`run_gh(&ctx, &["api", &format!("repos/{owner}/{name}/compare/{base}...{head}")])`. Parse the JSON
into `{ files: Vec<ComparedFile> }` (define a private `CompareRaw { files: Vec<ComparedFile> }`).
v1 may ignore `files` pagination (compare returns up to 300 files per page; large PRs are rare —
do not add complexity). Note: `gh api` needs the `...` URL-encoded? No — pass literally; `gh`
handles it. Use the three-dot `base...head` form (GitHub's compare semantics include merge-base),
which is what GitHub's own "files changed since" uses — but since we only read per-file `patch`
hunks and remap by old→new line, two-dot vs three-dot only differs when base also moved; head-only
advances are unaffected. Keep `base...head`.

## 3. Re-anchor helper + command — `src-tauri/src/commands/review.rs`

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReanchorResult {
    pub reanchored: usize,        // moved OR sha-advanced (badge cleared)
    pub lost: usize,              // left flagged
    pub skipped_no_change: usize, // already on current head
}

/// Re-anchor a review's RIGHT-side diff comments from their stored anchored_head_sha
/// to the target's current head_sha. Pure-helper; callers hold the lock.
fn reanchor_review_comments(conn: &Connection, detail: &ReviewDetail) -> AppResult<ReanchorResult>;

#[tauri::command]
pub fn reanchor_comments(review_id: i64, db: State<Db>) -> AppResult<ReanchorResult> {
    let conn = db.0.lock().unwrap();
    ensure_draft(&conn, review_id)?;
    let detail = load_detail(&conn, review_id)?;
    reanchor_review_comments(&conn, &detail)
}
```

Register `commands::review::reanchor_comments` in `lib.rs`.

### `reanchor_review_comments` logic

- `current = detail.target.head_sha` (if `None`, return all-zero result — nothing to do).
- Consider only comments with `side == "RIGHT"`, `subject_type == "line"`, `origin != "file_view"`.
- Bucket by `anchored_head_sha`:
  - `None` or `== current` → `skipped_no_change += 1`.
  - else → needs remap.
- For each `(file_path, anchored_sha)` group that needs remap, fetch the file's intervening diff
  **once per (anchored_sha, file)**:
  - clone-less PR (`detail.repo_path.starts_with("github:")`) → `gh::compare(owner, name,
    anchored_sha, current)`, pick the `ComparedFile` whose `filename == file_path`, use its `patch`
    (treat missing file/patch as: all that file's comments are `Lost`).
  - else → `git::diff_shas(Path::new(&detail.repo_path), anchored_sha, current)`, then split the
    multi-file diff into per-file sections (a `diff --git a/… b/…` introduces each file) and pick
    the `file_path` section; an empty/absent section means the file is unchanged between the two
    SHAs → every line is a `Shifted(line)` identity (re-anchor in place, advancing the SHA).
    *(Simplest robust approach: pass the whole `git diff` for just that file by calling
    `git diff --no-color old..new -- <file_path>` — add an optional path arg or a second helper
    `diff_shas_path(repo, old, new, file_path)`; then `parse_file_patch` sees only that file.)*
  - `parse_file_patch(patch)` → `FileHunks`.
- For each comment in the group, remap `line` (and `start_line` if `Some` and `!= line`):
  - both `Shifted` → `UPDATE comment SET line=?, start_line=?, anchored_head_sha=<current>,
    updated_at=now() WHERE id=?`; `reanchored += 1`.
  - any `Lost` → leave the row untouched (do **not** advance the SHA); `lost += 1`.

Recommendation: prefer the per-file `git diff … -- <path>` form so the parser only ever sees one
file — it avoids writing a multi-file splitter. Add `diff_shas` (whole) for completeness/tests and
a path-scoped variant used by the helper.

## 4. Test matrix (Rust only)

### `anchor.rs` unit tests (hand-written patches, no git)

| Case | Patch shape | Assert |
|---|---|---|
| identity | empty patch | `remap_right_line(80)` == `Shifted(80)` |
| insertion above | `@@ -50,0 +50,3 @@` (3 adds) | line 80 → `Shifted(83)` |
| deletion above | `@@ -50,2 +50,0 @@` (2 dels) | line 80 → `Shifted(78)` |
| inside replaced | hunk replacing the target line (`-`then`+`) | target → `Lost` |
| context inside hunk | hunk with adds then a ` ` context line at target | target → `Shifted(correct new_ln)` |
| multi-hunk delta | two hunks each +1 before line 200 | line 200 → `Shifted(202)` |
| past all hunks | one hunk early, line far below | `Shifted(line + delta)` |
| multi-line range | one endpoint in changed region | caller treats whole range `Lost` |

### `review.rs` integration tests (real git via `tempfile`, in-memory DB via `open_memory()`)

Add a fixture that: builds a temp git repo with a file, commits **H1**, edits the file (insert a
line above an existing line, plus one in-place replacement), commits **H2**. Seed an `open_memory()`
DB whose `repository.path` = the temp dir, a local target with `head_sha = H2`, a review, and
comments with `anchored_head_sha = H1`.

- Comment on a line that only shifted → after `reanchor_review_comments`, its `line` moved and
  `anchored_head_sha == H2`; result `reanchored == 1`.
- Comment on the replaced line → unchanged `line`, `anchored_head_sha` still `H1`; `lost == 1`.
- LEFT-side comment → untouched regardless.
- Comment already at `H2` → `skipped_no_change` counted, not touched.
- `git::diff_shas(H1, H2)` returns a diff containing the inserted text (sanity).

Extend the existing `seed_comment` test helper (or add a variant) to set `anchored_head_sha`.

### `gh.rs` fixture-parse test

Static `repos/.../compare` JSON string (a `files` array with `filename`/`status`/`patch`) parsed by
`compare`'s deserializer into `Vec<ComparedFile>`; assert fields. No network (mirror the `pr_meta`
fixture test pattern already in `gh.rs`).
