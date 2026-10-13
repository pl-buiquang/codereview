# Spec 10 — Focus review

Status: **spec / research** — nothing implemented yet.

## 1. Context & goals

Large PRs mix code that needs careful review with noise: lockfiles, generated code, mechanical
renames, fixtures, boilerplate. **Focus** is an AI-assisted lens that shows only the hunks worth
reviewing, in the normal diff view, with a digest explaining what was hidden and why.

Goals:
- Looks and behaves like a normal review: same diff view, same comments, same publish.
- Hunks are classified by importance once; you can tighten or loosen the filter without another AI call.
- Nothing is silently dropped: every hidden hunk or file is listed with a reason and can be shown in one click.
- Generation works from the terminal (Claude Code skill) and from the app (button), driven by **one
  prompt** versioned in this repo.

Non-goals: replacing review judgment, publishing AI output, live re-classification while a PR evolves.

Alternative names considered: Fast review, Digest, Triage, Skim, Express. **Focus** won because it
describes looking at what matters, while "fast" suggests a lower-quality review.

## 2. Concepts

| Term | Meaning |
|------|---------|
| **Focus** | An overlay attached to one review: per-hunk tiers, an AI summary and AI notes. At most one per review. |
| **Hunk** | A git diff hunk as produced by the review's diff. This is the unit of selection. |
| **Hunk id** | A stable short hash of `file_path` + the hunk's changed (`+`/`-`) lines. Context lines are excluded so the id survives unrelated line shifts. |
| **Tier** | `critical` · `review` · `skim` · `trivial`. |
| **Level** | A visibility threshold over tiers (see below). Switchable in-app. |
| **Override** | A manual hide/unhide on a hunk or a whole file. It takes precedence over the tier. |
| **Hidden file** | A file whose hunks are all hidden at the current level. |

Levels:

| Level | Shows |
|-------|-------|
| Off | the full diff (Focus disabled) |
| Thorough | everything except `trivial` |
| **Focus** (default) | `critical` + `review` |
| Critical-only | `critical` |

Effective visibility of a hunk = the hunk override, else the file override, else `tier ≥ threshold(level)`.

## 3. Key decisions

1. **Lens, not a separate review.** Focus is attached to a normal review. Comments you write while
   Focus is on are ordinary review comments, so there is nothing to transfer. Publish, GitHub sync,
   export and re-anchoring are unchanged. Focus content (summary, notes, tiers) is **never published**.
2. **Virtual filter, no synthetic branch.** The real diff is rendered and hidden hunks collapse into a
   bar. Line numbers are identical to the real diff, so the comment anchoring contract
   (`file_path, side, line`) is untouched, and PR targets without a local clone work.
   A synthetic branch was rejected for four reasons:
   - line numbers shift when earlier hunks are dropped;
   - it needs a clone;
   - it pollutes git;
   - comments would need remapping back to the real diff.
3. **One-shot.** A focus is pinned to the target's `head_sha`/`base_sha` at generation time. It is
   not regenerated automatically (§6.4).
4. **Classify once, filter by level.** A single AI run tiers every hunk. Changing the level is a UI-only filter.
5. **Rules first, then AI.** `cr` deterministically tiers obvious noise as `trivial`, so the AI only
   sees the rest. The AI may override a rule tier.
6. **Fail safe.** Any hunk without a tier (missed by the AI, or new since generation) is treated as `review`, i.e. visible.
7. **One prompt, two runners.** `skills/focus/SKILL.md` is the single source of truth. It is used by
   Claude Code in the terminal and embedded in the app for the in-app runner. Both runners hand back
   results through `cr focus set`.

## 4. User workflows

### 4.1 Terminal (Claude Code skill)
```
/focus https://github.com/o/r/pull/123 [--level focus|thorough|critical]
/focus o/r#123
/focus branch feature-x [--base main] [--repo <path>]
/focus <review-id>
```
1. The skill resolves the target and reuses the **latest draft review** on it, or creates one
   (`cr create … --reuse-draft`). A review id targets that review directly.
2. `cr focus hunks <rid> --json` lists the hunks, with rule tiers already applied.
3. The model reads the hunks: `cr focus hunk <rid> <id>` for full text, and Read/Grep in the clone if one is linked.
4. It writes `plan.json` and runs `cr focus set <rid> --file plan.json --level <level>`.
5. `cr open review <rid>` opens or focuses the review in the app, and the overlay appears.

### 4.2 In-app
1. The review header shows a **Generate focus** button with a level picker and a model picker.
   The model defaults to the chat model setting.
2. The app creates a `focus` row with `status='generating'` and runs the model in the background.
   The header shows "Generating focus…" and you can keep reviewing in the meantime.
3. When the run ends, the app emits an event and the review re-renders with the overlay, or shows an error.

### 4.3 Reviewing with Focus on
- Hidden hunks collapse into a bar: `▸ 3 hidden hunks — lockfile, mechanical rename [show]`.
  "Show" unhides them temporarily; the context menu offers to unhide permanently, which is an override.
- Every visible hunk can be hidden ("Hide hunk"), and so can every file ("Hide file").
- AI notes appear as a distinct banner above their hunk, styled as AI-authored. Each has two actions:
  - **Adopt** turns the note into a normal draft comment on the hunk's first changed line: the RIGHT
    side, or the LEFT side if the hunk only deletes.
  - **Dismiss** hides the note.
- The **digest panel** is pinned above the diff and can be collapsed. It contains:
  - the AI summary;
  - a *computed* list of hidden files and hunks, grouped by reason. It is derived from tiers,
    overrides and the current level, so it is always accurate.

  **Copy into summary** inserts the digest as markdown into `review.body`. This is opt-in only.
- File tree: fully hidden files stay listed but are dimmed, with a tier/reason tooltip. While Focus is
  on they don't count toward the "unviewed" total. Viewed state in the DB is unchanged.
- The level selector sits in the review header and is persisted on the focus. "Off" shows the full diff.

### 4.4 Head moves (stale focus)
- Banner: "Focus generated at `abc123`; head is now `def456`. [Regenerate]".
- Current-diff hunks are matched to stored hunks by **hunk id**. Matches keep their tier, reason and note.
- Unmatched hunks (new or changed) are visible and badged "new since focus".
- Regenerating replaces the focus. Overrides on hunk ids that still exist are kept.

## 5. Data model

New migration `src-tauri/src/db/migrations/0016_focus.sql`, appended to `MIGRATIONS` in `db/mod.rs`:

```sql
CREATE TABLE focus (
  id          INTEGER PRIMARY KEY,
  review_id   INTEGER NOT NULL UNIQUE REFERENCES review(id) ON DELETE CASCADE,
  head_sha    TEXT,
  base_sha    TEXT,
  level       TEXT NOT NULL DEFAULT 'focus'
              CHECK (level IN ('off','thorough','focus','critical')),
  summary     TEXT,
  model       TEXT,
  status      TEXT NOT NULL CHECK (status IN ('generating','ready','failed')),
  error       TEXT,
  created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE focus_hunk (
  focus_id       INTEGER NOT NULL REFERENCES focus(id) ON DELETE CASCADE,
  hunk_id        TEXT NOT NULL,
  file_path      TEXT NOT NULL,
  old_start      INTEGER NOT NULL,
  new_start      INTEGER NOT NULL,
  tier           TEXT NOT NULL CHECK (tier IN ('critical','review','skim','trivial')),
  source         TEXT NOT NULL CHECK (source IN ('rule','ai')),
  reason         TEXT,
  note           TEXT,
  note_dismissed INTEGER NOT NULL DEFAULT 0,
  override       TEXT CHECK (override IN ('show','hide')),
  PRIMARY KEY (focus_id, hunk_id)
);

CREATE TABLE focus_file_override (
  focus_id  INTEGER NOT NULL REFERENCES focus(id) ON DELETE CASCADE,
  file_path TEXT NOT NULL,
  override  TEXT NOT NULL CHECK (override IN ('show','hide')),
  PRIMARY KEY (focus_id, file_path)
);
```

Overrides are plain `show`/`hide` rather than re-tiers, so they behave predictably across levels.

## 6. Backend

New module `src-tauri/src/commands/focus.rs`. As elsewhere, `*_impl(conn, …)` functions are shared
by the Tauri commands and `cr`.

### 6.1 Hunk enumeration
- Built on `review_diff_impl` (`commands/review.rs`) and `anchor.rs::parse_file_patch`, extended if
  needed to expose per-hunk `old_start`, `new_start`, header and body.
- `hunk_id = sha1(file_path + "\0" + changed_lines)[..12]`.
- Rule pre-tiering (`source='rule'`, `tier='trivial'`) applies to:
  - lockfiles: `pnpm-lock.yaml`, `package-lock.json`, `yarn.lock`, `Cargo.lock`, `poetry.lock`,
    `uv.lock`, `Gemfile.lock`, `go.sum`, …;
  - binary files;
  - generated files: `linguist-generated` in `.gitattributes` (only when a clone is available), or an
    `@generated` / "Code generated … DO NOT EDIT" marker in the hunk;
  - snapshots: `__snapshots__/`, `*.snap`.

### 6.2 Commands
| Command | Purpose |
|---------|---------|
| `focus_hunks(review_id)` | Current hunks: id, file, header, +/- counts, rule tier, preview |
| `get_focus(review_id)` | Focus row, plus per *current* hunk `{file, old_start, new_start, hunk_id, effective_tier, reason, note, override, is_new}` and file overrides |
| `set_focus_plan(review_id, plan, level?)` | Validate and replace the focus (§7.2); pins current SHAs; `status='ready'` |
| `set_focus_level(review_id, level)` | Persist the level |
| `set_hunk_override(review_id, hunk_id, override?)` / `set_file_override(review_id, file, override?)` | Set or clear an override |
| `adopt_focus_note(review_id, hunk_id)` | Create a draft comment via `add_comment_impl`, then mark the note dismissed |
| `dismiss_focus_note(review_id, hunk_id)` | Dismiss the note |
| `clear_focus(review_id)` | Delete the focus |
| `generate_focus(review_id, level, model?)` | In-app runner (§8) |

None of these is blocked by `ensure_draft()` except `adopt_focus_note`, because it creates a comment.
On a published review the overlay still works as a read-only lens.

The frontend matches react-diff-view hunks to `get_focus` entries by `(file, oldStart, newStart)`.
It does not recompute hashes in TypeScript.

## 7. CLI contract (`cr focus …`)

### 7.1 Commands
```
cr focus hunks <rid> [--json]             compact list; warns above ~500 hunks
cr focus hunk  <rid> <hunk_id>            full hunk text (header + body)
cr focus set   <rid> --file plan.json [--level focus]
cr focus show  <rid> [--json]
cr focus clear <rid>
cr create pr|local … --reuse-draft        latest draft review on the target, else create
```
Also: the deep-link `openReview` handler (`src/lib/deepLink.ts`) must invalidate `["review", id]`
(and `["focus", id]`). Without it, a terminal run won't show up in a review that's already open.

### 7.2 Plan JSON
```json
{
  "summary": "Adds retry with backoff to the uploader; rest is a config rename.",
  "hunks": [
    { "id": "3f9a1c0b2d4e", "tier": "critical", "note": "Retry loop has no max-elapsed bound." },
    { "id": "a01b22c3d4e5", "tier": "trivial",  "reason": "mechanical rename uploadCfg→uploaderConfig" }
  ]
}
```
Validation:
- Unknown ids → error, and nothing is written.
- Missing ids → the rule tier if one exists, else `review`.
- `reason` is recommended for `skim`/`trivial`.
- `note` is only meaningful on visible tiers.

## 8. In-app runner

- Factor a reusable non-chat runner out of `commands/chat.rs::call_claude_turn`. It takes:
  - no session (`--no-session-persistence` if supported);
  - a configurable `--allowedTools`;
  - a system prompt file;
  - JSON output for the error/cost envelope;
  - model resolution as in chat.
- System prompt = `include_str!("../../../skills/focus/SKILL.md")` + a preamble: "App mode. Review id
  = N. Do not create or open reviews. Requested level = L."
- Allowed tools: `Bash(cr focus *) Bash(cr diff *) Bash(cr show *) Read Glob Grep`.
- Working directory: the linked clone (also passed as `--add-dir`, read-only use) if there is one, else
  a temp dir. **No worktree** is needed.
- Runs in `tauri::async_runtime::spawn_blocking`, so the DB lock is not held during the run. On exit it
  sets `status`/`error` (`failed` if `cr focus set` was never called) and emits `focus-updated {review_id}`.
- Frontend: `listen("focus-updated")` → invalidate `["focus", id]` and `["review", id]`.
- `focus` rows left at `generating` by an app crash are marked `failed` on startup.

## 9. Skill (`skills/focus/SKILL.md`)

Versioned in this repo. Install for the terminal with `cp -r skills/focus ~/.claude/skills/` (documented
in the README). The app embeds it at build time, so the installed copy and the app can differ until you
re-copy it.

Outline:
1. **Modes.** Terminal mode resolves or creates the review (§4.1); app mode gets a review id and skips
   creation and opening.
2. **Gather.** `cr focus hunks --json`; fetch full text for non-rule hunks; optionally Read/Grep for
   cross-file impact (callers, tests, config).
3. **Tier rubric.**
   - `critical`: security/auth, data loss or migrations, concurrency, public API/contract changes,
     money/billing, error handling on hot paths, subtle logic changes.
   - `review`: normal behavior changes, new logic, non-trivial tests of new behavior.
   - `skim`: straightforward glue, small refactors, logging, docs-adjacent code, test updates that follow mechanically.
   - `trivial`: formatting, renames/moves without logic change, imports, version bumps, generated
     output, fixtures, lockfiles.
   - When in doubt, tier **up**.
4. **Write.** A summary of 2–5 sentences. A `reason` for every `skim`/`trivial` hunk, grouped by
   consistent wording so the digest groups well. A `note` only where it adds a concrete "check this".
5. **Submit.** Write the plan to a temp file and run `cr focus set`. In terminal mode, also `cr open review`.

## 10. UI changes

- `src/lib/api.ts` / `types.ts`: wrappers and types for §6.2.
- `src/components/ReviewView.tsx`:
  - header: level selector, Generate/Regenerate, status badge, stale banner;
  - digest panel;
  - `FileBody`: filter hunks, insert hidden-run bars and note banners as `<Decoration>` rows around
    `<Hunk>` (where the expand controls are pushed today);
  - "new since focus" badge.
- `src/components/FileJumpList.tsx`: dim fully hidden files, exclude them from the unviewed count.
- Expanding context between hunks keeps working; hidden hunks count as a gap that can be expanded.

## 11. Edge cases

- **Rename-only or mode-only files** have no hunks. Rules tier them `trivial`; the digest lists them as "renamed".
- **Binary files:** rule `trivial`, listed as "binary".
- **Every hunk hidden** at the current level: show an empty-state banner with the digest and a "Show all" action.
- **Comments on hidden hunks:** a hunk with an existing comment is always shown (implicit override), so comments never disappear.
- **Diff search** into a hidden hunk unhides it temporarily, the same way it un-collapses viewed files today.
- **Published review:** the overlay is viewable; Adopt is disabled.
- **`claude` CLI missing:** the Generate button is disabled with a hint. The terminal skill is unaffected.
- **Review deleted:** the focus is removed by the cascade.

## 12. Out of scope / follow-ups

- Moving `local-review` to write focus notes instead of real comments.
- Including focus (digest, tiers) in Markdown/JSON export.
- Sub-hunk (line-range) selection or splitting "mixed" hunks.
- Automatic regeneration when head moves.
- Multi-call batching for very large PRs.

## 13. Open risks

- **Hunk matching:** the Rust parser and react-diff-view must agree on `(file, old_start, new_start)`
  for the same diff text. Needs a test on the shared diff fixtures.
- **Hunk id collisions:** identical changed lines in the same file, e.g. a repeated edit, produce the
  same id. Disambiguate with an occurrence index.
- **Cost and latency** on large PRs. Mitigated by rule pre-tiering and compact hunk listings.
- **Prompt drift** between the installed skill copy and the copy embedded in the app (copy-install is intentional).
- **AI under-tiering** something important. Mitigated by "tier up when in doubt", fail-safe defaults,
  the always-visible digest and one-click unhide.
