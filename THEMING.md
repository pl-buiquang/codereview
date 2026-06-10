# Theming

CodeReview ships **3 design directions × 2 modes = 6 fixed themes**, all driven by one
CSS-custom-property contract. There are no user-editable themes.

## How it works

- **One token contract.** Every color, font, radius, spacing step, and the 8 syntax-highlight
  roles is a CSS variable defined in [`src/styles/tokens.css`](src/styles/tokens.css). No
  component may hardcode a color, font, or radius — always `var(--token)`.
- **Theme = classes on `<html>`.** The active theme is the pair of classes
  `cr cr-{a|b|c} {dark|light}` on `document.documentElement` (plus `style.colorScheme`). The
  `.cr-{dir}` class supplies fonts/radii; the `.cr-{dir}.{mode}` block supplies the full palette.
- **Applied by [`src/lib/useApplySettings.ts`](src/lib/useApplySettings.ts).** It reads
  `direction` + `mode` from the settings store and writes the root classes, re-resolving live when
  `mode === "system"` (a `matchMedia` listener follows OS scheme flips). It also sets the single
  inline style var `--diff-font-size`.
- **Persisted** in `localStorage["codereview-settings"]` (zustand persist, **version 2**). The
  axes are `direction: "a"|"b"|"c"` (default `a`) and `mode: "dark"|"light"|"system"` (default
  `system`). See [`src/lib/settings.ts`](src/lib/settings.ts) for the v0/v1 → v2 migration.
- **FOUC guard.** An inline `<script>` in [`index.html`](index.html) reads the persisted settings
  and sets the root classes *before first paint*, so the app never flashes an unthemed frame.
  `useApplySettings` reconciles on mount.

The 6 themes are registered in [`src/lib/themes.ts`](src/lib/themes.ts) (`DIRECTIONS`); the mode
axis is generic across directions.

## Token reference

The contract is identical across all 6 theme combinations (values differ; names don't).

| Token | Role |
|---|---|
| `--bg`, `--bg-sunken` | App background; sidebar/titlebar/sunken wells |
| `--surface`, `--surface-2`, `--surface-3` | Cards; hover/raised; active/pressed |
| `--border`, `--border-strong` | Hairlines; input & button borders |
| `--text`, `--text-2`, `--text-3` | Primary / secondary / faint text |
| `--accent`, `--accent-hover`, `--on-accent` | Brand action color; hover; text on accent |
| `--accent-soft`, `--accent-border` | Tinted selection bg; focus rings |
| `--success`, `--warning`, `--danger` (+ `-soft`) | Status (open/approved, draft, delete/CI-fail) |
| `--diff-add-bg`, `--diff-add-strong`, `--diff-del-bg`, `--diff-del-strong` | Diff line + gutter backgrounds |
| `--shadow-card`, `--shadow-pop` | Card and popover elevation |
| `--titlebar`, `--scrim` | Window chrome bg; modal scrim |
| `--radius-s/m/l` | Per-direction: A 4/6/8 · B 6/10/14 · C 2/4/6 |
| `--sp-1…--sp-6` | Spacing scale 4/8/12/16/20/24px |
| `--font-ui`, `--font-display`, `--font-mono` | Font roles |

Non-theme sizing: `--diff-font-size` (user slider, default 12.5px) is set inline by
`useApplySettings`, not by the theme.

### Syntax highlighting (the 8 `--tok-*` roles)

Highlighting keeps refractor and 8 token roles. Each role is one `--tok-*` var per theme; the CSS
selector lists in `src/styles.css` (`.diff .token.*`) map Prism/refractor classes onto each role —
this mapping is the contract and is structurally fixed:

| Role (`--tok-…`) | Prism/refractor classes it colors |
|---|---|
| `comment` | `comment`, `prolog`, `doctype`, `cdata` |
| `punctuation` | `punctuation` |
| `literal` | `property`, `tag`, `boolean`, `number`, `constant`, `symbol`, `deleted` |
| `string` | `selector`, `attr-name`, `string`, `char`, `builtin`, `inserted` |
| `operator` | `operator`, `entity`, `url` |
| `keyword` | `atrule`, `attr-value`, `keyword` |
| `function` | `function`, `class-name` |
| `variable` | `regex`, `important`, `variable` |

## Adding a direction

1. Add the token blocks to `src/styles/tokens.css`: `.cr-{d} { fonts; radii }`, then
   `.cr-{d}.dark { … }` and `.cr-{d}.light { … }` with the **full token list including the 8
   `--tok-*` roles**.
2. Add a `DIRECTIONS` entry in `src/lib/themes.ts` (`id`, `label`, `blurb`).
3. Widen the `Direction` union in `src/lib/themes.ts`.
4. Bundle any new fonts in `src/styles/fonts.ts` (static `@fontsource/*` weights, **not** the
   `-variable` packages — family names must match the `tokens.css` stacks exactly).

No other code changes are needed — the direction picker, `useApplySettings`, and the FOUC script
all derive from `DIRECTIONS`/the class scheme.

## Rules

- **Never hardcode** a color, font, or radius. Always `var(--token)`.
- For tints, prefer the `-soft` tokens (`--accent-soft`, `--success-soft`, …) or
  `color-mix(... var(--accent) ...)` over a token — never a literal rgba.
- Fonts come only from `--font-ui` / `--font-display` / `--font-mono`.

## Verifying

Cycle all 6 themes in **Settings → General → Appearance** (direction cards × mode segmented).
Each must render with no transparent backgrounds or default-black text, every `--tok-*` role
visibly distinct on a code diff, and light-mode text ≥ 4.5:1 contrast on its background. `System`
mode must follow an OS dark/light flip live, and the chosen direction+mode must persist across an
app restart with no FOUC flash. Compare against
`specs/design_handoff_codereview_redesign/reference/CodeReview Redesign.html`.
