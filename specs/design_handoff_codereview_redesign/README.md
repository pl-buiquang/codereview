# Handoff: CodeReview UI Redesign (3-theme design system)

## Overview
A modernized, unified design system for **CodeReview** — a desktop code-review app (Rust Tauri + React frontend). It covers five screens: **Inbox** (triage queue), **Reviews** (saved reviews), **Repository / Virtual PR**, **Review editor · Diff** (split diff with file tree), and **Review editor · Comments** (PR header, inline review threads, comment composer).

The system ships as **three complete visual directions, each with dark and light mode**, all driven by one CSS-custom-property contract. The app should keep all three and expose a **runtime theme switcher** (direction × mode).

- **A · Continuity** — IBM Plex Sans/Mono, blue accent, slate-navy darks. Safe evolution of the current app.
- **B · Modern** — Manrope, indigo accent, near-black darks, soft radii (10–14px).
- **C · Terminal** — JetBrains Mono everywhere, green accent, sharp radii (2–6px), warm-paper light mode.

## About the Design Files
The files in this bundle are **design references created in HTML** — prototypes showing intended look and behavior, **not production code to copy directly**. The task is to **recreate these designs inside the existing React frontend** using its established patterns (components, routing, state). The reference JSX (`reference/cr/*.jsx`) is plain React written for in-browser Babel; treat it as a spec, not a library.

The two files that ARE intended for near-direct use are **`tokens.css`** and **`app.css`** (top level of this bundle):
- `tokens.css` — the entire theme contract. Drop into the app and apply classes `cr cr-{a|b|c} {dark|light}` on the React root element.
- `app.css` — component styles written 100% against the tokens. Use as the starting point for restyling existing components; adapt class names to the app's conventions (CSS modules, Tailwind `@apply`, styled-components, etc. — whatever the codebase uses).

## Fidelity
**High-fidelity.** Colors, typography, spacing, radii, and component states are final. Recreate pixel-perfectly. Layout dimensions in the reference are tuned for a 1440×900 window but everything is flex/grid based and should remain fluid.

## Theme switcher requirement
- Two independent axes: **direction** (`cr-a` / `cr-b` / `cr-c`) and **mode** (`dark` / `light`).
- Apply both as classes on a single root element (e.g. `<div class="cr cr-b dark">` or on `<html>`/`<body>`).
- Persist the choice (localStorage or Tauri settings store). Default: direction A, mode following `prefers-color-scheme`.
- Everything else is automatic — **no component may hardcode a color, font, or radius**; always `var(--token)`.

## Fonts
Load via Google Fonts (or bundle locally for offline Tauri use — preferred):
- IBM Plex Sans 400/500/600/700, IBM Plex Mono 400–700 (direction A)
- Manrope 400–800 (direction B)
- JetBrains Mono 400–700 (directions B code + C everything)

Font roles are tokens: `--font-ui` (all UI text), `--font-display` (page titles, review titles), `--font-mono` (code, counts, branch names, repo paths).

## Design Tokens
`tokens.css` is the source of truth. The contract (identical across all 6 theme combinations):

| Token | Role |
|---|---|
| `--bg`, `--bg-sunken` | App background; sidebar/titlebar/sunken wells |
| `--surface`, `--surface-2`, `--surface-3` | Cards; hover/raised; active/pressed |
| `--border`, `--border-strong` | Hairlines; input & button borders |
| `--text`, `--text-2`, `--text-3` | Primary / secondary / faint text |
| `--accent`, `--accent-hover`, `--on-accent` | Brand action color; hover; text on accent |
| `--accent-soft`, `--accent-border` | Tinted selection bg; focus rings |
| `--success`, `--warning`, `--danger` + `-soft` variants | Status (open/approved, draft, delete/CI-fail) |
| `--diff-add-bg`, `--diff-add-strong`, `--diff-del-bg`, `--diff-del-strong` | Diff line + gutter backgrounds |
| `--shadow-card`, `--shadow-pop` | Card and popover elevation |
| `--titlebar`, `--scrim` | Window chrome bg; modal scrim |
| `--radius-s/m/l` | Per-direction: A 4/6/8 · B 6/10/14 · C 2/4/6 |
| `--sp-1…--sp-6` | Spacing scale 4/8/12/16/20/24px |
| `--font-ui`, `--font-display`, `--font-mono` | Font roles |

Key accent values — A dark `#4C9EF8` / light `#0B6BD9`; B dark `#828CF8` / light `#4F46E5`; C dark `#3FCF8E` / light `#0E9F6E`. All other hexes: see `tokens.css`.

Base type: 13px UI (12.5px in direction C), 1.45 line-height. Code/diff: 11px mono, 1.7 line-height. Page titles: 19px/700 in `--font-display`.

## Screens / Views
All screens live inside the window chrome. Reference renders: `reference/CodeReview Redesign.html` (canvas of all screens × all themes), components in `reference/cr/`.

### Window chrome (`chrome.jsx` → WindowChrome)
44px titlebar merged with the tab strip: hamburger menu, home button, browser-style tabs (36px tall, top radius `--radius-m`, active tab = `--bg` fill + border, accent 6px dot for the active document, mono 11.5px labels, per-tab close ×), "+" new tab, window controls (min/max/close) right. Background `--titlebar`, bottom hairline `--border`.

### App sidebar (AppSidebar)
212px, `--bg-sunken`, right hairline. Brand row: 22px accent square with "cr" monogram + lowercase wordmark in `--font-display`. Nav items 32px tall, radius `--radius-m`, icon + label + mono count right-aligned; active = `--accent-soft` bg + `--accent` text. Settings pinned bottom.

### 1. Inbox (`screen-inbox.jsx`)
Triage queue. Page header ("Inbox" + "Logged in as @user", right: "updated 3d ago" + primary Refresh). Tab row with icons + count pills: Needs you / Authored / Team review / Bots / Visited / Closed — active tab: 2px accent underline, count pill becomes accent-tinted. Below: 196px facet rail (TYPE, REPOSITORIES, USERS; uppercase 10px headers, 26px rows, mono counts, selected = accent-soft) + card list. Each PR card (radius `--radius-l`): 30px avatar (bot/person icon), meta row (PR badge, mono repo + #num, `open` badge, ✓/✕ ci chip), bold 13.5px title, checks row (`review` badge + "by author" + check status), foot row (file count, `+adds`/`−dels` in success/danger mono, state, "top files" accent link). Right column: timestamp + action cluster (primary "Open as review", outline "Done", ghost "Untrack").

### 2. Reviews (`screen-reviews.jsx`)
Same scaffolding; facets: STATUS / ORIGIN / REPOSITORIES / VERDICT. Header right: "Sort" + select ("Last modified"). Rows: mono 13px title, meta line (repo · PR # · comments · verdict · updated), then `draft` badge, "Open PR ↗" split-button, ghost ×.

### 3. Repository · Virtual PR (`screen-repo.jsx`)
No app sidebar (it's a repo tab). Repo path as mono page title. Underline tabs: Virtual PR / GitHub PRs. "New virtual PR" card: base select (mono) ← compare select (mono, flex), `merge-base` checkbox; second row: "Preview diff" outline + "Start review" primary, Split/Unified segmented right. Helper text in `--text-3`. Below: "Reviews" heading + review rows as in screen 2.

### 4. Review editor · Diff (`screen-diff.jsx`)
Toolbar: Back, ghost Collapse, mono bold title, `draft` badge, "Saved", then right: Split/Unified segmented, Open PR split-button, Refresh, Export, **Publish (primary)**, **Delete (danger outline)**. Optional error banner (danger-soft bg, danger border/text). Review summary textarea + Verdict card (radios: Comment / Approve / Request changes). Body: 248px file tree (24px mono rows, folder/file icons, indent 12px/level, `+n −n` deltas right, selected = accent-soft) + diff panel: hint line, file card with header (mono path, deltas, "Comment on file"/"View file"/"Open" small buttons, Viewed checkbox) and **split two-column diff**: 40px right-aligned line-number gutters, `pre` code at 11px mono. Added lines: `--diff-add-bg` row + `--diff-add-strong` gutter; deleted likewise; spacer rows `--bg-sunken` at 60% opacity. Simple syntax tints: tags/keywords `--accent`, attributes `--warning`, strings `--success`.

### 5. Review editor · Comments (`screen-diff-comments.jsx`)
Adds to screen 4: **PR header card** — status row (open badge, ci chip, "Checking…", `+446 −0 · 25 files`, "▸ 37 checks" accent link right), reviewer row ("Approved" in success + avatar chips: 22px pills with 16px avatar + name), label tags (`needs-qa` accent-tinted, `deploy-front-pr` danger-tinted pills), Description heading + 12.5px body + "Show more" accent link. **Review thread** (inside diff card between hunks): `--surface-2` block, 2px accent left edge, radius `--radius-m`; comments separated by hairlines, each with 18px avatar, bold name, faint timestamp, "View on GitHub" link right; bodies use inline `code-chip`s (mono 10.5px, `--surface-3` bg, 4px radius). Thread badges: `GitHub` (accent fill) and `Outdated` (warning outline). **Comment composer**: card with `--accent-border` border, Write/Preview tab pair, mono body, footer right: Cancel + "Add comment" primary.

## Interactions & Behavior
- **Hover**: buttons → `--surface-2` bg (primary → `--accent-hover`); nav/rail/tree rows → `--surface-2`; cards unchanged (no lift).
- **Focus**: 2px ring using `--accent-border` on inputs, selects, textareas, buttons.
- **Active/selected**: `--accent-soft` bg + `--accent` text everywhere (nav, facets, tree, tabs use underline instead).
- **Diff commenting**: click a line to open the composer under it; shift-click to select a range (highlight rows with `--accent-soft`).
- **Transitions**: 120–150ms ease on background-color/border-color only. No entrance animations; this is a dense pro tool.
- **Disabled**: 45% opacity, no pointer events.

## State Management
The app already has data/state; the redesign is visual. New state: `theme = { direction: 'a'|'b'|'c', mode: 'dark'|'light' }`, persisted, applied as root classes. Everything else maps 1:1 onto existing app state (tabs, facets, selected file, composer open/closed, verdict radio).

## Assets
No raster assets. All icons in the references are inline 16×16 stroke SVGs (1.4px, `currentColor`) — see `CRIcon` in `reference/cr/chrome.jsx`; map to the app's existing icon set if one exists (Lucide/Tabler are close matches). Avatars: real user avatars where available, fallback person/bot glyph on `--surface-3` circle.

## Files
- `tokens.css` — **drop-in** theme contract (6 themes). Source of truth for every value.
- `app.css` — component CSS against the tokens; adapt into the codebase's styling approach.
- `reference/CodeReview Redesign.html` — canvas: all 5 screens × 3 directions × dark/light. **Self-contained** — open directly in any browser (double-click works, no server needed).
- `reference/Design System.html` — interactive token/component reference with direction + mode switcher (the same switcher the app needs). Also self-contained.
- `reference/cr/chrome.jsx` — window chrome, sidebar, icons, atoms (Btn, Seg, Badge, Avatar, Check, Radio, SelectBox).
- `reference/cr/screen-inbox.jsx`, `screen-reviews.jsx`, `screen-repo.jsx`, `screen-diff.jsx`, `screen-diff-comments.jsx` — per-screen reference implementations (read as spec; the HTML files above already embed them).

## Suggested Claude Code prompt
> Apply the design system in `design_handoff_codereview_redesign/` to this app. Start by adding `tokens.css` and a root theme switcher (direction a/b/c × dark/light, persisted), then restyle screens one at a time against the README specs and `app.css`, using our existing components. Never hardcode colors/fonts/radii — only `var(--token)`.
