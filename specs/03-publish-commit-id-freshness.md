# Spec 03 — `commit_id` freshness on publish

Implements ROADMAP §3. Publishing silently re-fetches the head, re-anchors comments, folds
un-mappable comments into the body, and posts against the fresh `commit_id` — so a PR head that
advanced after the review was opened never causes a 422. See Spec 00 for the shared contract; this
builds on Spec 01 (`reanchor_review_comments`) and Spec 02 (`refresh_target_shas`).

## Current state

`publish_review` (`review.rs:574`) loads `detail`, builds the payload via `build_publish_payload`
(`review.rs:478`), and posts via `gh::post_review`. `build_publish_payload` sets
`payload["commit_id"] = detail.target.head_sha` (`:511`) — the **stored** SHA — and emits every
non-file, non-`file_view` comment inline as `{path, side, line[, start_line, start_side], body}`.

## Change (signature stays `publish_review(review_id, db)` — no `force`)

Inside `publish_review`, after the existing `github_pr` / number / owner-name validation and
**before** building the payload:

1. **Refresh the head** (reuse Spec 02's helper): fetch the live head via `gh::pr_view(&ctx, number)`
   and `UPDATE target SET head_sha=…` when it advanced. (Holding the DB lock across this `gh` call is
   consistent with the existing code — `publish_review` already holds the lock across
   `gh::post_review`. The split-lock variant is fine too.)
2. **Re-anchor** (reuse Spec 01): reload `detail` (now carrying the fresh `head_sha`), then call
   `reanchor_review_comments(&conn, &detail)`. This mutates RIGHT comments in the DB while the review
   is still a draft (so `ensure_draft` is satisfied), advancing those it can re-map to the new head.
3. **Reload `detail`** once more so the payload sees the re-anchored lines + fresh `head_sha`.
4. **Build the payload** and post.

`build_publish_payload` already reads `detail.target.head_sha` for `commit_id`, so once `detail` is
reloaded the `commit_id` is automatically the fresh, verified head. No change to the `commit_id`
line itself.

## Avoiding 422 from un-mappable ("Lost") comments

A RIGHT comment whose `anchored_head_sha` still `!=` the (now fresh) `head_sha` after re-anchoring
is one that re-anchoring **could not** move (it sat in a changed/deleted region). Its stored `line`
may not exist in the fresh commit's diff, so emitting it inline risks the 422 we're trying to avoid.

In `build_publish_payload`, **exclude** such comments from the inline `comments` array and **fold
them into the review body** (reuse the `body_with_file_comments` machinery, `review.rs:521`): add a
section like `## Comments that could not be re-anchored` listing `**{file_path}** {Lxx[-Lyy]}: {body}`
(reuse `line_label`, `review.rs:563`). Concretely, the inline filter becomes:

```rust
.filter(|c| c.subject_type != "file"
         && c.origin != "file_view"
         && is_anchored_to(c, detail.target.head_sha.as_deref())) // anchored_head_sha == fresh head
```

where a comment with `anchored_head_sha == None` is treated as anchored (legacy/local rows keep
today's behaviour). Comments excluded by the new predicate go into the body section.

This keeps the publish atomic and 422-proof: everything inline is anchored to the exact `commit_id`
being posted; anything else degrades gracefully into prose.

## Frontend

No change. The Publish button still calls `api.publishReview(reviewId)`; the refresh + re-anchor +
fold happen entirely server-side and silently (per the locked decision).

## Tests (`review.rs`, no network)

`build_publish_payload` is a pure function over `ReviewDetail` (existing tests live near the bottom
of `review.rs`). Add:

- After setting `target.head_sha` to a new SHA and a comment's `anchored_head_sha` to that same SHA,
  the rebuilt payload's `commit_id` is the new SHA and the comment appears inline.
- A comment whose `anchored_head_sha` differs from `target.head_sha` (simulating "Lost") is **absent**
  from `payload["comments"]` and its text is **present** in `payload["body"]` under the
  could-not-be-re-anchored section.
- Existing payload tests (verdict→event mapping, multi-line `start_line`/`start_side`, file-level
  comments folding) still pass — adjust them if the inline filter predicate changed their inputs
  (give those comments an `anchored_head_sha` matching the target head, or `None`).

The live refresh+re-anchor path inside `publish_review` is exercised indirectly by Spec 01's
`reanchor_review_comments` tests and Spec 02's `refresh_review` tests; the `gh::post_review` network
call itself is not unit-tested (matches the existing no-network test policy).

## Verify (manual, `gh` authed)

Open a PR review, add an inline comment, push a new commit to the PR (advancing the head), then
**Publish** → succeeds with no 422 and the comment lands on the correct line of the new head
(check on github.com). A comment placed on a line that the new commit changed shows up in the review
body under "could not be re-anchored" rather than 422-ing the whole publish.
