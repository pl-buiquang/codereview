# The reviewed commits are pinned on the review, not the target

Several reviews can share one target, but the target used to store `base_sha`/`head_sha` and was overwritten every time the same PR or branch pair was reopened or refreshed. That silently moved the commits under older draft reviews. We moved the pin to `review.head_sha/base_sha`. The target now only names what is reviewed, and the live head comes from the GitHub mirror or the local branch tip. A mismatch shows "N new commits". **Update review** (drafts only) moves the pin, re-anchors draft comments and marks Focus stale. Published reviews never move.

## Considered Options

- Keep the pin on the target and only move it on an explicit update. Rejected: one review's update would still move every other review on that target.
- Never move a pin, and have "update" start a new review. Rejected: it fragments one review effort across several rows.
