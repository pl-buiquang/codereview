# Sync scheduling is owned by a Rust sync engine, not frontend timers

Polling used to be per-view frontend timers (off by default), and several views fetched the same GitHub facts independently. A backend `sync` module now owns scheduling (per-scope TTLs, backoff, focus awareness), deduplicates in-flight work per (entity, facet), and runs a cheap batched probe before any heavy fetch. The frontend only sends `sync_hint` on mount/focus and invalidates queries on `mirror-updated` events. That made it cheap enough to default auto Sync to **periodic**. Full Sync refetches everything without truncating anything.

The `cr` CLI shares the database but not the app process. It runs the same sync code as an in-process one-shot (`sync_now`) and relies on SQLite WAL for concurrent writes. There are no cross-process events: the app's next light sync probes, finds nothing changed and refetches nothing.

## Consequences

- Background `git fetch` is a separate, opt-in concern ("Fetch"), not part of Sync.
- The GitHub notifications `If-Modified-Since` gate for Discovery is deferred (ROADMAP).
- Exception: the open Review tab's PR facets (header/checks and threads) and the repo PR list are still React Query fetches, gated by the same Auto Sync setting. They are per-tab, already write through the mirror (and emit `mirror-updated`), and "always fresh while the Review tab is active" is exactly what a query on the active tab gives.
