# The Inbox is a triage queue with one state per entry; the Board owns monitoring

The Inbox predates the Board. It tried to both find new work and monitor it, with three independent timestamps on `inbox_entry` (`engaged_at` Done, `untracked_at` Archive, `closed_at` Closed), six tabs, and a separate Archive page. Tracking an item on the Board did nothing to its inbox state, so tracked work showed in both places, the Closed list grew without bound, and "untrack" meant archive.

An inbox entry now has exactly one `state`: `new`, `tracked` (put on the Board, including via *Open as review*) or `archived` with a cause (`dismissed`, `closed`, `discarded`). One page has two tabs, New and Archive. The Board keeps monitoring.

Two rules are not obvious from the code:

- **Resurfacing needs a new reason.** An archived entry returns to New only when discovery finds a reason it didn't have, or one that lapsed (not found in the last *complete* discovery) and came back, such as a re-requested review. Activity on the item never resurfaces it; the old "Visited comes back on any update" behaviour was the noise being removed. Reasons that stop matching are kept, and the entry shows as *stale* instead of vanishing. Staleness is only evaluated after a discovery in which every search succeeded, because search failures are swallowed.
- **Own PRs are never queued**, at query time rather than by state writes: the Board tracks them automatically, and a state write would race with discovery re-inserting them.

The old timestamp columns stay in the table, unread (dropping columns in SQLite rebuilds the table). Migration 0034 maps them: archived if any was set, tracked if the item has a live Board card, own PRs archived.

Inbox *activity* (CI changes, comment threads on tracked items) is planned as a separate view. It must not reuse `state`: it gets its own event table with its own seen flag, so triage and notification never share a column.

## Considered Options

- Keep Done (resurfaces on activity) and Archive (permanent) as two levels. Rejected: Done existed to find tracked work again, which the Board now does; two dismissals add a decision with no payoff.
- A "Later" state for items worth keeping but not on the Board. Rejected: the Board's Backlog lane already is that pile.
- Move the Inbox into the Board as a lane or sub-page. Deferred: the triage queue and the Board have different lifecycles; revisit when the activity view is specced.
