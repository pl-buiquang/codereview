# Spec 22 — Design-system redesign (3 directions × dark/light)

Applies the design handoff in `specs/design_handoff_codereview_redesign/` to the whole frontend.
The bundle's `README.md` is the **visual source of truth** (per-screen specs §1–5, interactions,
fidelity rules); this spec is the **engineering plan**: how the token system replaces the current
theming machinery, what gets deleted, what each phase touches, and how it's verified. Where this
spec and the bundle README disagree on *visuals*, the README wins; on *code structure*, this spec
wins.

> **Sequencing vs ORCHESTRATION waves 2–6:** this spec is in heavy contention with the remaining
> feature waves — it rewrites large parts of `src/styles.css` (touched by specs 07, 11, 12, 13,
> 14, 18, 19) and restyles `ReviewView.tsx` (the heavy-slot file of waves 2–4). **Do not run this
> spec concurrently with any wave.** Run it either entirely before wave 2 or entirely after
> wave 5 (user's call). If it runs first, the wave specs' `styles.css` "pure append" rule still
> holds (they append new blocks; the redesigned atoms are class-compatible supersets); if it runs
> after, phases 6–7 restyle whatever wave code has landed.

## Problem

The app has its own theming system (12 UI colors + 8 syntax colors + per-theme code font,
user-editable custom themes, dark/light slots — `src/lib/settings.ts`, applied as inline CSS vars
on `<html>` by `src/lib/useApplySettings.ts`). The design handoff replaces it with a richer
contract: **3 directions (A Continuity / B Modern / C Terminal) × 2 modes = 6 fixed themes**,
~35 tokens each, plus per-direction fonts and radii, defined once in
`specs/design_handoff_codereview_redesign/tokens.css` and applied as root classes
`cr cr-{a|b|c} {dark|light}`. Every screen must be restyled against this contract; no component
may hardcode a color, font, or radius.

## Decisions (locked — do not revisit)

- **Native OS titlebar stays.** No `decorations:false`, no drag regions, no min/max/close or
  hamburger in React. The design's 44px titlebar+tabstrip is adapted onto the existing in-app
  `TabBar` (its tab look, `--titlebar` background, accent dot) below the native chrome. Do NOT
  port the window-chrome styles (`.crw-menu`, `.crw-winctl`, `.crw-new`, `.crw-home`) from the
  bundle's `app.css`.
- **The custom theme editor is removed** — `src/components/settings/` (ThemeSection, ThemeEditor,
  ThemePreview) is deleted, along with custom-theme CRUD and the dark/light theme slots in the
  store. The 6 design themes are the only themes, selected by two axes: **direction**
  (`a`/`b`/`c`, default `a`) and **mode** (`dark`/`light`/`system`, default `system`). A new
  `THEMING.md` at repo root documents how to add a theme.
- **Syntax highlighting keeps refractor and the 8 token roles**, with a dedicated 8-color palette
  per theme (48 values fixed in this spec, §Syntax palettes) shipped as `--tok-*` vars inside each
  theme block. The `.diff .token.*` selector lists in `styles.css` are the role→Prism-class
  contract and stay structurally unchanged.
- **Theme = classes on `<html>`**, not inline vars. `useApplySettings` is rewritten to maintain
  `documentElement.className` (`cr cr-{dir} {mode}`) + `style.colorScheme`. `--diff-font-size`
  remains the only inline style var.
- **`diffFontSize` setting survives unchanged** (slider, default 12.5, `DEFAULT_DIFF_FONT_SIZE`
  untouched — the design's 11px diff text is a suggestion only). The per-theme `codeFont` setting
  is gone: `--font-mono` (per direction) is the only code font.
- **Fonts are bundled locally** via static `@fontsource/*` packages (NOT `-variable` — family
  names must match `tokens.css` strings exactly: `"Manrope"`, not `"Manrope Variable"`).
- **Old→new CSS var bridge: temporary aliases, swept per phase, deleted at the end** (§D3).
- **localStorage settings migrate** via zustand persist `version: 2`; custom themes are
  intentionally dropped, the old mode maps over, all non-theme settings carry forward.
- **Phases run sequentially, one handoff unit each.** Every phase ends with the app compiling,
  tests green, and all 6 themes visually correct for the surfaces that phase owns. No phase
  refactors `ReviewView.tsx` logic — phases 6–7 are CSS-first with surgical JSX edits only.

## Design

### D1. Token CSS → `src/styles/tokens.css`

Vendor `specs/design_handoff_codereview_redesign/tokens.css` near-verbatim to
`src/styles/tokens.css` (new `src/styles/` dir), with two additions:

1. The 48 `--tok-*` values (§Syntax palettes) appended into each of the 6 theme blocks.
2. The COMPAT alias block (§D3) appended at the end.

Import order in `src/main.tsx`: `./styles/fonts` → `./styles/tokens.css` →
`react-diff-view/style/index.css` → `./styles.css`.

The `.cr` base rule in tokens.css (font/color/bg/13px/1.45, 12.5px for `.cr-c`) is kept — it
governs app density once Phase 2 removes the old root `font-size: 14px`. Do NOT port the
`.cr { display:flex }` from the bundle's `app.css`; layout stays on `.app-shell`.

**FOUC guard:** inline `<script>` in `index.html` `<head>` reads
`localStorage["codereview-settings"]` (try/catch; defaults direction `a`, mode from
`prefers-color-scheme`), sets the root classes before first paint. Also fix `<title>`.

### D2. Settings model v2 — `src/lib/settings.ts`

New state: `direction: "a"|"b"|"c"` (default `"a"`), `mode: "dark"|"light"|"system"` (default
`"system"`), plus setters. Keep: `effectiveTheme`, `parseBotLogins`, `diffFontSize`,
`defaultViewType`, `defaultThreeDot`, `botLogins`, `prListPollMs` and their setters.

Delete: `Theme`, `UiColors`, `SyntaxColors`, `ThemePatch`, `TokenRole`, `TOKEN_ROLE_CLASSES`,
`TOKEN_ROLES`, `UI_VAR`, `tokenVar`, `MONO_FONT_PRESETS`, `FALLBACK_CODE_FONT`, `BUILTIN_*`,
`BUILTINS`, `cloneTheme`, `findTheme`, `resolveActiveTheme`, `customThemes`, `darkThemeId`,
`lightThemeId`, `addThemeFrom`, `renameTheme`, `deleteTheme`, `updateTheme`. (The role→Prism-class
mapping from `TOKEN_ROLE_CLASSES` is preserved as documentation in THEMING.md; the CSS already
hardcodes the selector lists.)

New registry `src/lib/themes.ts`:

```ts
export type Direction = "a" | "b" | "c";
export const DIRECTIONS: { id: Direction; label: string; blurb: string }[] = [
  { id: "a", label: "Continuity", blurb: "IBM Plex · blue · slate-navy" },
  { id: "b", label: "Modern",     blurb: "Manrope · indigo · near-black" },
  { id: "c", label: "Terminal",   blurb: "JetBrains Mono · green · sharp" },
];
```

Migration (persist `version: 1` → `2`; v0 and v1 collapse the same way):

```ts
migrate: (persisted, version) => {
  const p = (persisted ?? {}) as Record<string, unknown>;
  const oldMode = version < 1 ? (p.theme as string) : (p.themeMode as string);
  return {
    direction: "a",
    mode: oldMode === "dark" || oldMode === "light" ? oldMode : "system",
    diffFontSize: (p.diffFontSize as number) ?? DEFAULT_DIFF_FONT_SIZE,
    defaultViewType: (p.defaultViewType as DiffViewType) ?? "split",
    defaultThreeDot: (p.defaultThreeDot as boolean) ?? true,
    botLogins: (p.botLogins as string) ?? "",
    prListPollMs: (p.prListPollMs as number) ?? 0,
  } satisfies PersistedSettings;
},
```

`customThemes`/`darkThemeId`/`lightThemeId` are intentionally not carried over. Update
`partialize` to the new keys.

### D3. Var-name bridge (COMPAT aliases)

Same-named tokens (`--bg --border --text --accent --danger --success --warning --diff-add-bg
--diff-del-bg`) cover ~200 of ~250 `var()` uses in `styles.css`, including every
`color-mix(... var(--accent) ...)` — they work the moment tokens land. Exactly four names differ
(measured counts):

```css
/* COMPAT — remove in Phase 8 */
.cr {
  --bg-elev: var(--surface);          /* 19 uses */
  --muted: var(--text-2);             /* 62 uses */
  --code-font: var(--font-mono);      /* 15 uses */
  --review-tab-accent: var(--accent); /*  1 use  */
}
```

Append to `src/styles/tokens.css`; delete the old `:root { … }` var block at the top of
`src/styles.css` in Phase 0. Rationale for aliasing over a one-shot rename: Phase 0 alone makes
the entire app render correctly in all 6 themes (a working visual baseline for every later
phase); the screen phases rewrite most of those rules anyway; every handoff commit stays small
and compiling. Each screen phase sweeps its own CSS sections (`--bg-elev`→`--surface-2` or
`--surface` as appropriate, `--muted`→`--text-2`, `--code-font`→`--font-mono`); Phase 8 deletes
the block behind a grep gate.

### D4. `app.css` adaptation strategy

`specs/design_handoff_codereview_redesign/app.css` is the canonical stylesheet for new
structures. Port it **section-by-section into `src/styles.css`** (keep the single-flat-file
convention), in the phase that owns each screen. Adopt app.css class names (`.btn` family,
`.seg/.seg-item`, `.badge-*`, `.chip`, `.card`, `.cr-rail*`, `.cr-tab`, `.tree-row`, `.pr-card`,
`.rev-row`, `.thread`, `.composer-*`, `.code-chip`, `.avatar-chip`, `.label-tag`, `.banner`,
`.check/.checkbox/.radio`, `.select/.input/.textarea`, `.mono/.muted/.faint/.row`) when the JSX
is rewritten to the new DOM; keep existing class names where the DOM survives and only
colors/radii change.

The reference is a static mock — add the interaction states from the bundle README §Interactions:

- Hover: buttons → `--surface-2` (primary → `--accent-hover`); nav/rail/tree rows → `--surface-2`;
  cards unchanged (no lift).
- Focus: `:focus-visible` 2px ring `var(--accent-border)` on inputs/selects/textareas/buttons.
- `transition: background-color .13s ease, border-color .13s ease` only; no entrance animations.
- `:disabled { opacity: .45; pointer-events: none }`.

**Collision watch** — old `styles.css` already defines `.btn-primary`, `.btn-danger`, `.btn-icon`,
`.composer`/`.composer-actions`, `.mono`, `.muted`, `.checkbox`, `.spinner`. Phase 2 *replaces*
the atom rules; later phases must not re-add old variants.

### D5. Fonts — static @fontsource

```
pnpm add @fontsource/ibm-plex-sans @fontsource/ibm-plex-mono @fontsource/manrope @fontsource/jetbrains-mono
```

New `src/styles/fonts.ts`, imported first in `src/main.tsx`:
IBM Plex Sans 400/500/600/700 · IBM Plex Mono 400/500/600/700 · Manrope 400/500/600/700/800 ·
JetBrains Mono 400/500/600/700. Vite bundles the woff2 — fully offline. Add mono italics
(`/400-italic.css`) only if visual QA shows markdown/em needs them.

### Syntax palettes — the 48 `--tok-*` values

| Role | A dark | A light | B dark | B light | C dark | C light |
|---|---|---|---|---|---|---|
| `--tok-comment` | `#61788C` | `#6B7E90` | `#64647A` | `#8A8A99` | `#586273` | `#8E8B7E` |
| `--tok-punctuation` | `#AEBDCB` | `#45586B` | `#A6A6BB` | `#4B4B58` | `#A8B0C0` | `#54564E` |
| `--tok-literal` | `#79C0FF` | `#0550AE` | `#7DCFFF` | `#0369A1` | `#73DACA` | `#B26209` |
| `--tok-string` | `#A5D6FF` | `#0A3069` | `#6EE7B7` | `#047857` | `#9ECE6A` | `#56783A` |
| `--tok-operator` | `#56D4DD` | `#0E7490` | `#B4BCF9` | `#C026D3` | `#89DDFF` | `#45707A` |
| `--tok-keyword` | `#FF7B72` | `#BF2D3D` | `#A78BFA` | `#7C3AED` | `#BB9AF7` | `#945E80` |
| `--tok-function` | `#D2A8FF` | `#6E40C9` | `#7AA2F7` | `#2563EB` | `#7AA2F7` | `#3A66A3` |
| `--tok-variable` | `#FFA657` | `#953800` | `#FDBA74` | `#C2410C` | `#E0A875` | `#9C5331` |

Rationale: A = GitHub-Primer-adjacent (continuity with today's builtins); B = indigo/violet-led
cool ramp beside accent `#828CF8`/`#4F46E5`; C dark = Tokyo-Night-derived terminal set (the
theme's `--warning`/`--danger` already are Tokyo Night colors); C light = warm gruvbox-paper
tones for the `#F4F2EC` background. Light values target ≥4.5:1 contrast on their bg.

## Phases (one handoff unit each, strictly in order)

Every phase gates on §Gates and a visual pass cycling all 6 themes (via Settings once Phase 1
lands; before that, toggle the root classes in devtools).

### Phase 0 — Token + font foundation (app still old-looking, fully 6-themeable)

1. Add the four @fontsource deps; create `src/styles/fonts.ts` (D5).
2. Create `src/styles/tokens.css` = bundle tokens.css + 48 `--tok-*` + COMPAT block (D1, D3).
3. `src/main.tsx`: imports in D1 order.
4. `src/styles.css`: delete the `:root` var-declaration block (keep the old body
   `font-family`/`font-size` rules until Phase 2 retunes density).
5. `src/lib/useApplySettings.ts`: replace inline-var `applyTheme` with class application;
   hardcode `direction = "a"` until Phase 1 adds the store field; `mode` from the existing
   `themeMode` + `effectiveTheme()` + a `matchMedia("(prefers-color-scheme: light)")` change
   listener; set `style.colorScheme`. Keep the `--diff-font-size` effect.
6. `index.html`: FOUC inline script + `<title>`.

Verify: app renders ~identically in `cr-a dark`; toggling devtools classes through all 6 combos
restyles everything live; no missing-var artifacts (transparent backgrounds, default-black text).

### Phase 1 — Settings v2, editor removal, theme switcher, THEMING.md

1. `src/lib/settings.ts`: new model + v2 migration (D2). New `src/lib/themes.ts`.
2. Delete `src/components/settings/` (3 files).
3. `src/components/SettingsView.tsx`: drop the theme section/import. In General → Appearance:
   - **Direction picker** — 3 preview cards, each rendered with
     `className={"cr cr-" + id + " " + resolvedMode + " theme-card"}` so the token cascade
     self-previews it (swatch row `--bg/--surface/--accent/--success`, "Aa" in `--font-ui`, code
     sample in `--font-mono`; label + blurb from `DIRECTIONS`). Selected card =
     `--accent-border` ring.
   - **Mode picker** — segmented Dark / Light / System (replaces the old `<select>`).
4. `src/lib/useApplySettings.ts`: read `direction` + `mode` from the store.
5. Rewrite `src/lib/settings.test.ts`: defaults (`a`/`system`), setters, v0→v2 and v1→v2
   migrations (assert custom-theme fields dropped; `themeMode:"light"` → `mode:"light"`;
   non-theme settings preserved). Fix any other test importing removed exports.
6. Write **`THEMING.md`** (repo root):
   - *How theming works*: one token contract; classes `cr cr-{dir} {mode}` on `<html>`; applied
     by `useApplySettings`; persisted in localStorage key `codereview-settings` v2; FOUC script.
   - *Token reference*: the ~35 layout/color tokens (table from the bundle README §Design Tokens)
     + the 8 `--tok-*` roles with the Prism classes each colors (preserved from the old
     `TOKEN_ROLE_CLASSES`: comment→comment/prolog/doctype/cdata; punctuation→punctuation;
     literal→property/tag/boolean/number/constant/symbol/deleted;
     string→selector/attr-name/string/char/builtin/inserted; operator→operator/entity/url;
     keyword→atrule/attr-value/keyword; function→function/class-name;
     variable→regex/important/variable).
   - *Adding a direction*: (1) add `.cr-d { fonts/radii }`, `.cr-d.dark { … }`, `.cr-d.light
     { … }` blocks to `src/styles/tokens.css` (full token list incl. the 8 `--tok-*`);
     (2) add a `DIRECTIONS` entry in `src/lib/themes.ts`; (3) widen the `Direction` union;
     (4) bundle any new fonts in `src/styles/fonts.ts`.
   - *Rules*: never hardcode color/font/radius — always `var(--token)`; tints via the `-soft`
     tokens or `color-mix()` over tokens.
   - *Verifying*: cycle all 6 themes in Settings; contrast expectations.

Verify: fresh profile = A/system; paste a v1 localStorage blob in devtools → migrates cleanly;
switcher live-updates; `pnpm test`.

### Phase 2 — Chrome: TabBar + shared atoms (substrate for all later phases)

1. `styles.css` tab-bar section: 44px bar on `var(--titlebar)`, bottom hairline `--border`;
   tabs 36px bottom-aligned, top radius `var(--radius-m)`, active = `--bg` fill + `--border`
   border (sides+top), **accent 6px dot** on the active tab (new `<span className="tab-dot">` in
   `TabItem`), labels `var(--font-mono)` 11.5px, per-tab close ×. Remove the
   `.tab.tab-review.active` `--review-tab-accent` underline rule. Overflow menu → `.card` +
   `--shadow-pop`.
2. New **`src/components/icons.tsx`**: `Icon({ name, size })` with the full inline 16×16 stroke
   SVG set (1.4px, `currentColor`) copied from `CRIcon` in
   `specs/design_handoff_codereview_redesign/reference/cr/chrome.jsx`
   (menu/home/x/plus/inbox/review/archive/repo/gear/refresh/chev/check/back/ext/comment/eye/
   branch/file/folder/bot/person/team/closed/sort/dot). All later phases use this. Swap TabBar's
   text glyphs (✕/⋯/⚙) for icons.
3. Port atom CSS from app.css into a new `/* ===== Design-system atoms ===== */` section near the
   top of `styles.css`: `.btn` family (replacing old `button` base, `.btn-primary`,
   `.btn-danger`, `.btn-icon`), `.seg` (restyle the existing `.view-toggle` with the seg recipe —
   no ReviewView JSX changes yet), `.badge-*`, `.chip`, `.delta-add/del`, `.card`,
   `.input/.select/.textarea`, `.check/.checkbox/.radio`, `.banner`, `.mono/.muted/.faint/.row`,
   plus the global hover/focus/transition/disabled rules (D4). Remove the old root
   `font-size: 14px` (the `.cr` token rule now governs density: 13px, 12.5px for C).

### Phase 3 — Home shell: AppSidebar + Inbox (bundle README §AppSidebar, §1)

1. `src/components/DashboardPanel.tsx` → the design's **AppSidebar**: 212px `--bg-sunken` rail,
   right hairline; brand row (22px accent square "cr" monogram + lowercase wordmark in
   `--font-display`); nav items 32px, radius `--radius-m`, `Icon` + label + mono count
   right-aligned (use a react-query-cached inbox count if cheap; else omit counts initially);
   active = `--accent-soft` bg + `--accent` text; Settings pinned bottom. Port
   `.cr-side*`/`.cr-nav*` CSS replacing `.dashboard-sidebar`/`.nav-*` rules (keep the
   `.dashboard`/`.dashboard-main` layout wrappers).
2. **Inbox** — `InboxView.tsx`, `InboxItemRow.tsx`, `InboxBadges.tsx`: page header
   (`.cr-pagehead`/`.cr-h1`, "Logged in as @user" sub, right "updated…" + primary Refresh);
   `.inbox-tab` → underline-tab recipe with icon + count pill (active = 2px accent underline,
   accent-tinted pill); facet rail → `.cr-rail` recipe (196px, uppercase 10px headers, 26px rows,
   mono counts, `--accent-soft` selection); rows → `.pr-card` recipe (radius `--radius-l`, 30px
   avatar with bot/person `Icon` fallback on `--surface-3`, meta row [PR badge, mono repo+#num,
   open badge, ci chip], 13.5px bold title, checks row, foot row [file count, `+adds`/`−dels` in
   success/danger mono, state, "top files" accent link], right column timestamp + action cluster
   [primary "Open as review", outline "Done", ghost "Untrack"]). Map `InboxBadges` onto
   `.badge-*`/`.chip`.

### Phase 4 — Reviews, Archive, Repositories (README §2; Archive/Repos extrapolated)

1. **`ReviewsView.tsx`**: facet rail STATUS/ORIGIN/REPOSITORIES/VERDICT reusing `.cr-rail`;
   header-right "Sort" + `.select`; rows → `.rev-row` (mono 13px title, `·`-separated meta line
   [repo · PR # · comments · verdict · updated], draft badge, "Open PR ↗" split-button — restyle
   `OpenPrButton.tsx`/`.split-button` as `.btn-split`, ghost ×).
2. **`ArchiveView.tsx`** (not in the design — extrapolate): same page scaffold; search → `.input`;
   rows as `.rev-row` with `--text-2` emphasis.
3. **`RepositoriesView.tsx`** (extrapolate): `.repo-item` → `.card` rows, paths in
   `--font-mono`, add-repo `.btn-primary`.

### Phase 5 — Repo view / Virtual PR (README §3)

`src/components/RepoView.tsx`: mono repo path as page title (`.cr-h1`); underline tabs Virtual
PR / GitHub PRs; "New virtual PR" `.card` (base `.select.mono` ← compare `.select.mono` flex,
`merge-base` `.check`; second row "Preview diff" outline + "Start review" primary, Split/Unified
`.seg` right; helper text `--text-3`); "Reviews" heading + rows reusing Phase 4's `.rev-row`;
GitHub PRs tab rows reuse a `.pr-card`-lite styling.

### Phase 6 — Review editor: diff (README §4) — CSS-first, minimal JSX surgery

Files: `ReviewView.tsx` (1317 lines — do NOT refactor its state/logic; class/icon/span edits
only), `FileJumpList.tsx`, `DiffViewer.tsx`, `FileViewPane.tsx`; CSS sections "Diff files",
"react-diff-view overrides", "Review header", "file jump list", "Full-file pane".

1. Toolbar `.review-header`: Back ghost + back icon, ghost Collapse, mono bold title,
   `.badge-draft`, "Saved" in `--text-3`; right: `.seg` Split/Unified, `.btn-split` Open PR,
   Refresh + Export `.btn`, **Publish `.btn-primary`**, **Delete danger-outline**. Error banner →
   `.banner` (danger-soft bg, danger border/text).
2. Summary textarea → `.textarea`; verdict block → Verdict card with `.radio` recipe
   (Comment / Approve / Request changes).
3. `FileJumpList.tsx` → `.tree` recipe: 248px, 24px mono rows, folder/file `Icon`, 12px/level
   indent, `+n −n` deltas right (`.delta-add/del`), selected `--accent-soft`.
4. `.diff-file` → `.card`-based diff card; header → mono path, deltas, "Comment on file" /
   "View file" / "Open" `.btn-sm`, Viewed `.check`.
5. **react-diff-view overrides** (risk hotspot — restyle only via its documented class hooks,
   never its markup): `.diff` bg `--surface`, `var(--font-mono)` at `var(--diff-font-size)`;
   gutters 40px right-aligned `--text-3` ~10.5px; `.diff-gutter-insert/-delete` backgrounds
   `--diff-add-strong` / `--diff-del-strong` (the spec puts `-strong` on gutters), code rows
   `--diff-add-bg` / `--diff-del-bg`; hunk header/expander → `--bg-sunken`; keep the
   `color-mix(var(--accent)…)` selection vars. Sweep this section's `--bg-elev`→`--surface-2`,
   `--muted`→`--text-2`, `--code-font`→`--font-mono`.
6. Refractor: no JSX change — the `.diff .token.*` rules already read `--tok-*`, now supplied per
   theme. Verify each role is visibly distinct in all 6 themes on a TS + CSS diff.

### Phase 7 — Review editor: comments (README §5) + shared surfaces

1. `PrMetaPanel.tsx` → PR header card: status row (open badge, ci chip, mono `+a −d · n files`,
   "▸ n checks" accent link right); reviewer row ("Approved" in `--success` + 22px `.avatar-chip`
   pills with 16px avatar + name); `.label-tag` pills (accent-tinted / danger-tinted by label);
   Description heading + 12.5px body + "Show more" accent link.
2. `GithubThread.tsx` + ReviewView's `.line-widget`/`.comment-item` → `.thread` recipe:
   `--surface-2` block, 2px accent left edge, radius `--radius-m`; hairline-separated comments
   (18px avatar, bold name, faint timestamp, "View on GitHub" link right); `GitHub` (accent fill)
   and `Outdated` (warning outline) thread badges; inline code as `.code-chip` (mono 10.5px,
   `--surface-3` bg, 4px radius) via `Markdown.tsx`'s `code` renderer or `.comment-body code` CSS.
3. Composer → app.css recipe: card with `--accent-border` border, Write/Preview tab pair
   (existing `.comment-edit-tabs`), mono body, footer right Cancel + "Add comment" primary.
4. Shared leftovers: `Toaster`, `ConfirmDialog`, export modal → `.card` + `--shadow-pop` +
   `--scrim` overlay; `Markdown.tsx` typography against tokens.

### Phase 8 — Settings restyle, cleanup, compat removal

1. Restyle `SettingsView.tsx` (extrapolated): settings nav → `.cr-nav` recipe, groups as
   `.card`s, rows with `.select`/`.check`/`.seg`, range slider `accent-color: var(--accent)`.
2. **Compat sweep + delete**: rewrite remaining `--bg-elev|--muted|--code-font|--review-tab-accent`
   uses; gate `grep -nE -- '--bg-elev|--muted|--code-font|--review-tab-accent' src/styles.css`
   → empty; delete the COMPAT block from `src/styles/tokens.css`.
3. Delete dead `src/App.css` (verified unimported) and orphaned CSS (old theme-editor `.theme-*`
   sections).
4. Full QA matrix (§Manual verify).

## Files touched

- NEW: `src/styles/tokens.css`, `src/styles/fonts.ts`, `src/components/icons.tsx`,
  `src/lib/themes.ts`, `THEMING.md`
- DELETED: `src/components/settings/` (ThemeSection, ThemeEditor, ThemePreview), `src/App.css`
- Heavy: `src/styles.css` (every phase), `src/lib/settings.ts` (+test), `src/lib/useApplySettings.ts`
- Per phase: `src/main.tsx`, `index.html`, `package.json`; `TabBar.tsx`, `DashboardPanel.tsx`,
  `InboxView.tsx`, `InboxItemRow.tsx`, `InboxBadges.tsx`, `ReviewsView.tsx`, `ArchiveView.tsx`,
  `RepositoriesView.tsx`, `RepoView.tsx`, `OpenPrButton.tsx`, `ReviewView.tsx`,
  `FileJumpList.tsx`, `DiffViewer.tsx`, `FileViewPane.tsx`, `PrMetaPanel.tsx`,
  `GithubThread.tsx`, `Markdown.tsx`, `Toaster.tsx`, `ConfirmDialog.tsx`, `SettingsView.tsx`
- No Rust changes. No migrations.

## Risks & mitigations

- **react-diff-view internals** (table layout, `--diff-*-selected-*` custom props): restyle only
  via its class hooks; test split AND unified, plus `FileViewPane`'s single-file unified mode.
- **refractor class drift**: the `.diff .token.*` selector lists are the contract; THEMING.md
  documents the role→class mapping (replacing the deleted `TOKEN_ROLE_CLASSES`).
- **`color-mix()` over old vars**: remaining mixes reference same-named tokens — safe; the one
  `var(--muted)` mix is alias-covered until Phase 7 rewrites it. Prefer the `-soft` tokens when
  rewriting.
- **Density shift** (14px → 13px base, 12.5px for C): unported screens may ripple mid-project —
  acceptable; each screen phase retunes its own section.
- **Tests asserting store shape / class names**: `settings.test.ts` rewritten in Phase 1;
  `RepoView.test.tsx`, `PrMetaPanel.test.tsx`, `RepositoriesView.test.tsx`, `Markdown.test.tsx`
  fixed in the phase that renames their classes.
- **Font family mismatch**: static @fontsource (not `-variable`) chosen so the tokens.css stacks
  resolve exactly; verify rendered fonts in devtools per direction.

## Gates (every phase)

```bash
pnpm exec tsc --noEmit
pnpm build
pnpm test
```

(Frontend-only spec, but run the cargo gates once at the end to prove no backend drift:
`cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings && cargo test --manifest-path src-tauri/Cargo.toml`.)

Phase-specific extra gates: Phase 1 — v1-blob migration check; Phase 8 — the compat grep above
plus `grep -rn "App.css" src/` → empty.

## Manual verify (`pnpm tauri dev`)

- Per phase: cycle all 6 themes (direction × mode in Settings; devtools classes before Phase 1)
  on the surfaces that phase owns, comparing side-by-side against
  `specs/design_handoff_codereview_redesign/reference/CodeReview Redesign.html` (open directly in
  a browser) and the bundle README's §-spec for that screen.
- Phase 8 full matrix: 6 themes × {Inbox, Reviews, Archive, Repositories, Repo (Virtual PR +
  GitHub PRs), Review editor diff (split AND unified), Review editor comments (incl. GitHub
  threads + composer), Settings, modals/toasts/confirm}; diff-font slider; mode=System follows an
  OS scheme flip live; direction+mode persist across app restart; no FOUC flash on launch.

## Out of scope

- Custom window chrome (titlebar, window controls, hamburger) — native decorations stay.
- Custom/user-created themes and any theme-editing UI.
- Any `ReviewView.tsx` logic/state refactor, routing changes, or new features.
- Backend (Rust) changes of any kind.
- The bundle's reference JSX as runtime code — it is spec only.
