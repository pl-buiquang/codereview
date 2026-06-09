# Spec 18 — Reply to + resolve existing GitHub PR threads

Implements ROADMAP §3 "**Reply to existing threads** and **resolve** them via the API"
(`ROADMAP.md:42`).

## Problem

GitHub PR review threads are fetched and displayed but cannot be acted on from the app:

- `gh::pr_review_threads` (`src-tauri/src/gh.rs:791-814`) fetches each thread's GraphQL node
  `id`, `isResolved`, and its comments' REST `databaseId` — everything needed to reply and to
  resolve — but `gh.rs` has **no mutation** for either. The only write to GitHub anywhere is
  `post_review` (`gh.rs:125-139`).
- `src/components/GithubThread.tsx:7-9` says it outright: *"Read-only display of an existing
  GitHub PR review thread. Never editable"*. The component renders an `isResolved` badge
  (`GithubThread.tsx:20-22`) with no button next to it, and the thread bottom has no composer.
- `src-tauri/src/commands/gh.rs` exposes only read commands (`pr_meta`, `pr_review_threads`,
  `list_prs`, …); nothing in `lib.rs`'s `generate_handler!` (lines 70-75) mutates GitHub thread
  state.
- Spec 12 (local-comment resolution) explicitly deferred this: *"Resolving **GitHub** PR threads
  via the API (`resolveReviewThread` GraphQL) — ROADMAP §3, separate spec"*
  (`specs/12-resolve-threads.md:340-341`). This is that spec.

So today the user must click "View on GitHub" and finish the conversation in the browser.

## Decisions (locked)

- **Acts on GitHub state directly — nothing is persisted locally.** No migration, no `comment`
  rows, `comment.github_comment_id` stays unwritten. Threads remain ephemeral
  (`["pr-threads", …]` query); after a mutation the frontend simply **invalidates
  `["pr-threads", owner, name, prNumber]`** (same key as `ReviewView.tsx:81` / `:227`) and
  re-renders from the refetch. No optimistic update — the round trip is one `gh` call.
- **Reply = REST** `POST repos/{owner}/{name}/pulls/{number}/comments/{comment_id}/replies`,
  where `comment_id` is the **`databaseId` of the thread's FIRST comment** (already returned by
  `pr_review_threads`; `PrThreadComment.database_id`, `gh.rs:664`). If that `databaseId` is
  `null`, the reply composer is hidden (cannot build the endpoint).
- **Resolve/unresolve = GraphQL** mutations `resolveReviewThread` / `unresolveReviewThread`
  (input: the thread **node id**, `PrThread.id`) through the existing `graphql()` helper
  (`gh.rs:101-121`), inheriting its partial-error tolerance (data + errors → use data, log
  errors).
- **One command, bool flag:** `set_pr_thread_resolved(thread_id, resolved)` rather than two
  commands — mirrors Spec 12's `set_comment_resolved(comment_id, resolved)` so the UI handler is
  one line. The two mutation documents alias their payload to a common `result:` field so a
  single deserializer serves both.
- **Not gated by local review status.** These actions mutate GitHub, not the local draft, so
  they are available even on a `published` (read-only) review — no `ensure_draft`, no `readOnly`
  prop check for thread actions. Rationale: a finished local review is exactly when you go back
  and answer/resolve reviewer threads.
- **Mutations require push access** (resolve) / commenting rights (reply). Acceptable: solo
  repo. Failures surface as the rejected `invoke` value → `toast.error`, like every other
  command; no capability pre-check.
- **Reuse the existing `Composer`** (`ReviewView.tsx:1134-1173`, already exported) for the reply
  box, adding an optional `submitLabel` prop (default `"Add comment"`) so the button can read
  "Reply". One composer style everywhere.
- **Soft relationship to Spec 17 only:** no shared backend code, no ordering dependency; the
  only possible overlap is the `GithubThread.tsx` render surface — whichever lands second
  rebases trivially.

## Design

### 1. Backend — `src-tauri/src/gh.rs`

Add below the `pr_review_threads` section (after line 814), before `mod tests`.

```rust
// ---------------------------------------------------------------------------
// PR review-thread mutations (reply, resolve/unresolve)
// ---------------------------------------------------------------------------

/// Shared by post_review and reply_to_thread: pull the integer `id` out of a
/// REST creation response.
fn parse_rest_id(out: &str, what: &str) -> AppResult<i64> {
    let value: serde_json::Value = serde_json::from_str(out)
        .map_err(|e| AppError::Gh(format!("failed to parse {what} response: {e}")))?;
    Ok(value.get("id").and_then(|v| v.as_i64()).unwrap_or_default())
}

/// Reply to an existing PR review thread. `comment_id` is the REST databaseId of
/// the thread's FIRST (root) comment — GitHub attaches the reply to that
/// comment's thread. Clone-less; returns the new reply's database id.
pub fn reply_to_thread(
    owner: &str,
    name: &str,
    number: i64,
    comment_id: i64,
    body: &str,
) -> AppResult<i64> {
    let endpoint = format!("repos/{owner}/{name}/pulls/{number}/comments/{comment_id}/replies");
    let ctx = GhRepo::Remote { owner: owner.to_string(), name: name.to_string() };
    let payload = serde_json::json!({ "body": body }).to_string();
    let out = run_gh_stdin(
        &ctx,
        &["api", &endpoint, "--method", "POST", "--input", "-"],
        &payload,
    )?;
    parse_rest_id(&out, "reply")
}

// Both mutations alias their payload object to `result` so one deserializer
// (ThreadMutationData) serves resolve and unresolve alike.
const RESOLVE_THREAD_MUTATION: &str = r#"
mutation ResolveThread($threadId: ID!) {
  result: resolveReviewThread(input: { threadId: $threadId }) {
    thread { id isResolved }
  }
}
"#;

const UNRESOLVE_THREAD_MUTATION: &str = r#"
mutation UnresolveThread($threadId: ID!) {
  result: unresolveReviewThread(input: { threadId: $threadId }) {
    thread { id isResolved }
  }
}
"#;

#[derive(Debug, Deserialize)]
struct ThreadMutationData {
    result: Option<ThreadMutationResult>,
}

#[derive(Debug, Deserialize)]
struct ThreadMutationResult {
    thread: Option<ThreadStateNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadStateNode {
    #[allow(dead_code)]
    id: String,
    is_resolved: bool,
}

/// Resolve or unresolve a PR review thread by its GraphQL node id (PrThread.id).
/// Requires push access to the repository. Returns the thread's new isResolved.
pub fn set_thread_resolved(thread_id: &str, resolved: bool) -> AppResult<bool> {
    let query = if resolved { RESOLVE_THREAD_MUTATION } else { UNRESOLVE_THREAD_MUTATION };
    let data: ThreadMutationData =
        graphql(query, serde_json::json!({ "threadId": thread_id }))?;
    data.result
        .and_then(|r| r.thread)
        .map(|t| t.is_resolved)
        .ok_or_else(|| AppError::Gh("thread mutation returned no thread".into()))
}
```

Also refactor `post_review` (`gh.rs:125-139`) to call `parse_rest_id(&out, "review")` instead of
its inline `serde_json::from_str` + `get("id")` block — behavior identical.

### 2. Commands — `src-tauri/src/commands/gh.rs`

After `pr_review_threads` (line 37). Both `async` for the same reason documented on `pr_meta`
(`commands/gh.rs:19-22`): keep the network call off the UI thread. No DB lock is taken.

```rust
/// Reply to an existing GitHub review thread. `comment_id` is the databaseId of
/// the thread's first comment. Acts on GitHub directly; nothing touches SQLite.
#[tauri::command]
pub async fn reply_to_thread(
    owner: String,
    name: String,
    number: i64,
    comment_id: i64,
    body: String,
) -> AppResult<i64> {
    gh::reply_to_thread(&owner, &name, number, comment_id, &body)
}

/// Resolve/unresolve a GitHub review thread by node id. Returns new isResolved.
#[tauri::command]
pub async fn set_pr_thread_resolved(thread_id: String, resolved: bool) -> AppResult<bool> {
    gh::set_thread_resolved(&thread_id, resolved)
}
```

Register both in `src-tauri/src/lib.rs`'s `generate_handler!` after
`commands::gh::pr_review_threads` (line 74):

```rust
commands::gh::reply_to_thread,
commands::gh::set_pr_thread_resolved,
```

### 3. Frontend boundary — `src/lib/api.ts`, `src/lib/types.ts`

`api.ts`, GitHub section after `prReviewThreads` (line 68). Tauri converts `commentId` →
`comment_id` / `threadId` → `thread_id` automatically:

```ts
replyToThread: (owner: string, name: string, number: number, commentId: number, body: string) =>
  invoke<number>("reply_to_thread", { owner, name, number, commentId, body }),
setPrThreadResolved: (threadId: string, resolved: boolean) =>
  invoke<boolean>("set_pr_thread_resolved", { threadId, resolved }),
```

`types.ts`, next to `PrThread` (line 153):

```ts
/** Which PR a GitHub thread belongs to — needed by thread mutations and for
 *  invalidating the ["pr-threads", owner, name, number] query. */
export interface PrThreadCtx {
  owner: string;
  name: string;
  number: number;
}
```

### 4. UI — `src/components/GithubThread.tsx`

New optional prop `ctx?: PrThreadCtx | null`. With `ctx == null` (defensive: PR target missing
remote info) the component renders exactly as today. With `ctx` set:

```
┌ github-thread ────────────────────────────────────────────────┐
│ GitHub  [Resolved] [Outdated]  (Hide/…toggle)     [Unresolve] │ ← header; button label flips
│  ┌ comment 1 — author · 2d ago · View on GitHub             │ │
│  │  markdown body                                           │ │
│  ├ comment 2 — …                                            │ │
│  └──────────────────────────────────────────────────────────┘ │
│  [Reply…]                       ← collapsed affordance        │
│  ┌ textarea (Composer) ────────────────┐                      │
│  │                                     │  [Cancel] [Reply]    │
│  └─────────────────────────────────────┘                      │
└────────────────────────────────────────────────────────────────┘
```

Changes (the component already imports `api`; add `useMutation`/`useQueryClient` from
`@tanstack/react-query`, `toast` from `../lib/toast`, `Composer` from `./ReviewView`, and the
`PrThreadCtx` type):

1. **Resolve/Unresolve button** — in `.github-thread-header`, right-aligned after the existing
   badges/toggle (`GithubThread.tsx:16-38`), rendered whenever `ctx` is set:

   ```tsx
   const queryClient = useQueryClient();
   const invalidate = () =>
     queryClient.invalidateQueries({
       queryKey: ["pr-threads", ctx!.owner, ctx!.name, ctx!.number],
     });

   const setResolved = useMutation({
     mutationFn: (resolved: boolean) => api.setPrThreadResolved(thread.id, resolved),
     onSuccess: invalidate,
     onError: (e) => toast.error(`Thread update failed:\n${String(e)}`),
   });
   // …
   <button
     className="github-thread-action"
     disabled={setResolved.isPending}
     onClick={() => setResolved.mutate(!thread.isResolved)}
   >
     {thread.isResolved ? "Unresolve" : "Resolve"}
   </button>
   ```

2. **Reply composer** — after the comment list, only when `expanded` (keep collapsed resolved
   threads quiet), `ctx` is set, **and** `thread.comments[0]?.databaseId != null`:

   ```tsx
   const rootId = thread.comments[0]?.databaseId ?? null;
   const [replying, setReplying] = useState(false);

   const reply = useMutation({
     mutationFn: (body: string) =>
       api.replyToThread(ctx!.owner, ctx!.name, ctx!.number, rootId!, body),
     onSuccess: () => {
       setReplying(false);
       invalidate();
     },
     onError: (e) => toast.error(`Reply failed:\n${String(e)}`),
   });

   // render:
   {!replying ? (
     <button className="github-thread-action" onClick={() => setReplying(true)}>
       Reply…
     </button>
   ) : (
     <Composer
       submitLabel="Reply"
       onSubmit={(text) => reply.mutateAsync(text)}
       onCancel={() => setReplying(false)}
     />
   )}
   ```

   `Composer` already trims and disables submit on empty text (`ReviewView.tsx:1158`), so no
   extra validation. After a successful reply the refetch shows the new comment with the user's
   own login; resolving after a reply is a second click, not bundled.

3. Update the stale doc comment at `GithubThread.tsx:7-9` (no longer "never editable" — still
   never persisted/exported).

### 5. UI plumbing — `src/components/ReviewView.tsx`

`ReviewDetail` already carries `remote_owner` / `remote_name` (used at `ReviewView.tsx:75-77`)
and `target.github_pr_number`, so `FileReview` (`:469`, has `detail` in scope) computes once:

```ts
const threadCtx: PrThreadCtx | null =
  detail.remote_owner && detail.remote_name && detail.target.github_pr_number != null
    ? {
        owner: detail.remote_owner,
        name: detail.remote_name,
        number: detail.target.github_pr_number,
      }
    : null;
```

- Pass `ctx={threadCtx}` at the inline-widget render site (`<GithubThread key={t.id} thread={t} />`,
  `ReviewView.tsx:722`).
- Add a `threadCtx: PrThreadCtx | null` prop to `FileBody` (props list `:830-868`, call site
  `:805-824`) and pass `ctx={threadCtx}` in the orphan-threads block (`:898`).

`Composer` (`:1134`) gains the optional label:

```ts
submitLabel?: string;            // default "Add comment"
```

with `{submitLabel ?? "Add comment"}` as the primary-button text. All existing call sites are
untouched.

### 6. CSS — `src/styles.css`

Extend the `.github-thread*` block (`styles.css:1091-…`): `.github-thread-action` (small,
muted-bordered button matching `.github-thread-toggle`), `margin-left: auto` on the header
action so it right-aligns, and spacing for the composer inside `.github-thread`.

### Data flow

Click **Resolve** → `invoke("set_pr_thread_resolved", { threadId, resolved: true })` →
`gh api graphql` `resolveReviewThread` → invalidate `["pr-threads", owner, name, number]` →
`pr_review_threads` refetch → thread re-renders with the Resolved badge (and collapses if GitHub
now reports `isCollapsed`). Reply is the same shape through the REST replies endpoint. SQLite is
never touched.

### Files touched

- `src-tauri/src/gh.rs` — `parse_rest_id`, `reply_to_thread`, mutation consts + deserializers,
  `set_thread_resolved`, `post_review` refactor, tests
- `src-tauri/src/commands/gh.rs` — two new `async` commands
- `src-tauri/src/lib.rs` — register both commands
- `src/lib/api.ts`, `src/lib/types.ts` — wrappers + `PrThreadCtx`
- `src/components/GithubThread.tsx` — Resolve/Unresolve button, reply composer, doc comment
- `src/components/ReviewView.tsx` — `threadCtx` plumbing, `Composer.submitLabel`
- `src/styles.css` — `.github-thread-action` + composer spacing
- `src/lib/api.test.ts`, NEW `src/components/GithubThread.test.tsx`

## Tasks

1. `gh.rs`: `parse_rest_id` + refactor `post_review` to use it; unit test. Builds green alone.
2. `gh.rs`: `reply_to_thread`, mutation consts, `ThreadMutationData` structs,
   `set_thread_resolved`; fixture-parse tests.
3. `commands/gh.rs`: `reply_to_thread` + `set_pr_thread_resolved` commands; register in
   `lib.rs`.
4. `api.ts` wrappers + `types.ts` `PrThreadCtx` + `api.test.ts` cases.
5. `Composer.submitLabel` + `GithubThread.tsx` Resolve button + reply composer + CSS.
6. `ReviewView.tsx` `threadCtx` plumbing (widget site + `FileBody` prop) +
   `GithubThread.test.tsx`.

## Test matrix

Rust — `src-tauri/src/gh.rs` `mod tests` (fixture-parse pattern, no network — mirror
`pr_threads_maps_fixture`):

| Test | Asserts |
|---|---|
| `parse_rest_id_extracts_id` | `{"id": 12345, "body": "x"}` → `Ok(12345)`; garbage input → `Err(AppError::Gh)` mentioning the `what` label |
| `thread_mutation_parses_resolved` | `{"result":{"thread":{"id":"T1","isResolved":true}}}` deserializes; `is_resolved == true` |
| `thread_mutation_parses_unresolved` | same shape with `false` → `is_resolved == false` |
| `thread_mutation_null_result_is_none` | `{"result": null}` deserializes with `result == None` (so `set_thread_resolved` maps it to `Err`, not a panic) |

Vitest:

| Test | Asserts |
|---|---|
| `api.test.ts`: replyToThread wrapper | `invoke` called with `("reply_to_thread", { owner: "o", name: "n", number: 5, commentId: 1001, body: "hi" })` |
| `api.test.ts`: setPrThreadResolved wrapper | `invoke` called with `("set_pr_thread_resolved", { threadId: "T1", resolved: true })` |
| `GithubThread.test.tsx` (mock `../lib/api`, QueryClientProvider wrapper per `PrMetaPanel.test.tsx:39-44`): resolve click | with `ctx` set and `isResolved: false`, clicking **Resolve** calls `api.setPrThreadResolved(thread.id, true)`; label reads **Unresolve** when `isResolved: true` |
| `GithubThread.test.tsx`: reply flow | click **Reply…**, type, click **Reply** → `api.replyToThread(owner, name, number, comments[0].databaseId, text)`; composer closes on resolve |
| `GithubThread.test.tsx`: no ctx → read-only | `ctx` undefined → no Resolve button, no Reply affordance (today's render) |
| `GithubThread.test.tsx`: null databaseId hides reply | `comments[0].databaseId: null` with `ctx` set → Resolve button present, Reply affordance absent |

## Gates

Standard suite (all must pass):

```bash
pnpm exec tsc --noEmit
pnpm build
pnpm test                                                   # vitest run
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test   --manifest-path src-tauri/Cargo.toml
```

No spec-specific gate (no migration, no schema change).

## Manual verify (real PR; gh-CLI recipe)

Seed a scratch PR with a review thread on `pl-buiquang/codereview` itself:

```bash
cd /home/paul/projects/codereview
git checkout -b scratch/thread-test
printf 'line one\nline two\n' > scratch.txt
git add scratch.txt && git commit -m "scratch: thread reply/resolve test fixture"
git push -u origin scratch/thread-test
gh pr create --title "scratch: thread test" --body "throwaway" \
  --base main --head scratch/thread-test          # note the PR number N
# Create a root review comment => one review thread on scratch.txt:1 (RIGHT):
gh api repos/pl-buiquang/codereview/pulls/N/comments \
  -f body="root comment from gh" \
  -f commit_id="$(git rev-parse HEAD)" \
  -f path="scratch.txt" -f side=RIGHT -F line=1
git checkout main
```

In the app (`pnpm tauri dev`):

1. Open PR `N` (repo's PR list or inbox). On `scratch.txt` line 1, the GitHub thread renders
   with the root comment, a **Resolve** button in its header, and **Reply…** below.
2. Click **Reply…**, type `reply from the app`, click **Reply** → composer closes, thread
   refetches and shows the reply under your login.
3. Click **Resolve** → after refetch the **Resolved** badge appears and the button reads
   **Unresolve**. Click **Unresolve** → badge clears.
4. Resolve again, then verify from the CLI:

```bash
gh api graphql -f query='query { repository(owner:"pl-buiquang", name:"codereview") {
  pullRequest(number: N) { reviewThreads(first: 10) { nodes {
    isResolved comments(first: 10) { nodes { body author { login } } } } } } } }'
```

Expect `isResolved: true` and two comments (`root comment from gh`, `reply from the app`).

5. Open a **published** review on the same PR — thread actions are still present and work
   (locked decision: not gated by local review status).
6. Cleanup: `gh pr close N --delete-branch`.

## Out of scope

- Persisting GitHub threads, replies, or resolution state locally — no `comment` rows, no
  `github_comment_id` writes, no migration.
- Resolving **local** comment threads (Spec 12, `set_comment_resolved`) — different storage,
  different button; nothing shared but the visual pattern.
- Creating **new** standalone GitHub threads outside `publish_review`; pending/draft GitHub
  reviews (ROADMAP §3, separate item).
- Editing or deleting existing GitHub comments; emoji reactions.
- Linking the app's own published comments back to threads (would need `github_comment_id`
  round-tripping from `publish_review` — separate work).
- Pagination of >100-comment threads (existing `comments(first: 100)` cap, `gh.rs:755-756`).
- Anything in Spec 17 — soft relationship only: no shared code, no ordering dependency beyond
  rebasing the `GithubThread.tsx` surface if both touch it.
