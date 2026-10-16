# The GitHub mirror is a disposable cache referenced by node id, without enforced FKs

Inbox (`items`), Board (`board_card`) and Review (`target`) each kept their own drifting copy of the same PR facts. They now share one mirror (`gh_pr`, `gh_issue`, `gh_user` and facet tables) that holds **no user state**, so it can be wiped ("Reset mirror") and rebuilt from GitHub. User state (triage, tracking, links, pins) stays in `inbox_entry`, `board_card`, `board_link`, `target` and `review`. Those tables reference the mirror by GitHub node id **with no enforced foreign key**: views LEFT JOIN, render a placeholder for missing facts and enqueue a sync. That way a reset or GC can never cascade into user data. Mirror rows that nothing references and that haven't synced for 30 days are garbage-collected.

PRs from repos the user never added still need a `repository` row. Those rows are auto-created with `registered_at = NULL`. Only registered repositories appear in the Repositories view and on the Board, and unreferenced auto-added rows are garbage-collected.

## Considered Options

- Enforced FKs from user tables to the mirror. Rejected: Reset and GC would need stub rows or cascade into user state.
- A separate `gh_repo` table instead of auto-created `repository` rows. Rejected in favour of a single repository concept with a registration flag.
