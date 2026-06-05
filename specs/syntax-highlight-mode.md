# Spec: syntax-highlight mode within the diff

## Summary / motivation

Highlighting is currently inferred from the file extension only. Extensionless files (e.g. a
shell script named `build`, a `Dockerfile` variant), embedded languages, or misdetected files get
no highlighting or the wrong grammar. This adds per-file controls to (a) toggle highlighting
on/off and (b) override the highlighting language, independent of the extension.

## Current state

- **Language detection is extension-only.** `src/lib/diff.ts`:
  ```ts
  export function languageForPath(path: string): string | undefined {
    const name = path.split("/").pop()?.toLowerCase() ?? "";
    const ext = name.includes(".") ? name.split(".").pop()! : name;
    const lang = EXT_LANG[ext];
    return lang && refractor.registered(lang) ? lang : undefined;
  }
  ```
  `EXT_LANG` (lines 5–12) maps ~40 extensions to refractor language names. If the file has no
  extension, `ext` becomes the whole filename and almost always misses the map.
- **Tokenization:**
  ```ts
  export function tokenizeFile(file: FileData) {
    const language = languageForPath(fileDisplayPath(file));
    if (!language) return undefined;
    try { return tokenize(file.hunks, { highlight: true, refractor, language }); }
    catch { return undefined; }
  }
  ```
  Called once per file in `FileReview`: `const tokens = useMemo(() => tokenizeFile(file), [file])`
  (`ReviewView.tsx:331`) and passed to `<Diff tokens={tokens} ...>` (line 535). `undefined` tokens
  → react-diff-view renders plain (unhighlighted) text.
- **Controls surface:** `.diff-file-header` (`ReviewView.tsx` line ~440) already hosts per-file UI
  (path, stats, Viewed toggle) — the natural home for these controls.
- `refractor@^5.0.0`; `refractor.registered(lang)` reports whether a grammar is loaded. The set of
  registered languages is what `EXT_LANG` targets today.

## Goals & non-goals

**Goals**
- Per-file toggle: highlighting on/off.
- Per-file language override: pick any registered refractor language, overriding the
  extension-derived guess (useful for extensionless/embedded/misdetected files).

**Non-goals**
- Registering additional refractor grammars (e.g. real Dockerfile support) — that's ROADMAP §9
  ("More syntax grammars"); reference it. This spec only re-routes among **already-registered**
  languages.
- Word-level intra-line highlighting (separate ROADMAP §1 item).
- A global default-language setting (per-file only in v1).

## UX & behavior

- In `.diff-file-header`, a small control group:
  - A highlight on/off toggle.
  - A language `<select>` listing registered languages (label = display name, value = refractor
    id), defaulting to the extension-derived guess (or "Plain" when none).
- Changing either re-renders just that file's diff with new (or no) tokens.
- v1: choices are **ephemeral** (per session, reset on reload). Persistence is an open question
  (below).

## Technical design

**Frontend**
- **Generalize `tokenizeFile`** to take options:
  ```ts
  export function tokenizeFile(
    file: FileData,
    opts?: { language?: string | null; enabled?: boolean },
  ) {
    if (opts?.enabled === false) return undefined;
    const language = opts?.language ?? languageForPath(fileDisplayPath(file));
    if (!language || !refractor.registered(language)) return undefined;
    try { return tokenize(file.hunks, { highlight: true, refractor, language }); }
    catch { return undefined; }
  }
  ```
  Keep the no-arg behavior identical for existing callers (`DiffViewer.tsx` also uses highlighting
  — verify it still compiles / behaves).
- **Expose the language list.** Add a helper, e.g. `availableLanguages(): string[]`, derived from
  the languages the app registers (the values in `EXT_LANG`, deduped, filtered by
  `refractor.registered`). Optionally include a friendly-name map for the dropdown labels.
- **State in `FileReview`.** Add local state `const [highlight, setHighlight] = useState(true)` and
  `const [langOverride, setLangOverride] = useState<string | null>(null)`. Change the memo:
  `const tokens = useMemo(() => tokenizeFile(file, { language: langOverride, enabled: highlight }),
  [file, langOverride, highlight])` (currently keyed on `[file]` only at line 331).
- Render the toggle + select in `.diff-file-header` next to the Viewed toggle.

**Backend**
- None for v1 (ephemeral). If persistence is chosen, see open questions.

**Data**
- None for v1.

**CSS (`src/styles.css`)**
- Style the control group to sit alongside `.diff-stats` / `.viewed-toggle`; compact `<select>`
  using theme variables. Ensure it doesn't crowd long file paths (the header is a flex row).

## Edge cases

- **Override to a language refractor doesn't have registered** → fall back to plain (the
  `refractor.registered` guard already handles it); the dropdown should only list registered langs
  so this shouldn't arise.
- **Tokenize throws** on odd content → caught, returns `undefined` (plain), same as today.
- **Binary files** → no diff body, controls hidden/disabled.
- **Very large files** → toggling highlight off is itself a useful perf escape hatch; note the
  interaction with tokenize-off-main-thread (ROADMAP §5).
- **Expanded context lines** (see diff-context-expansion spec): if that feature lands, tokens must
  be recomputed on the expanded hunks — keep the memo dependency on hunks, not just `file`.

## Phasing

- **v1:** per-file on/off toggle + language override, ephemeral.
- **v1.1:** persist per-(review, file) choice (if desired); friendly language names; remember last
  override for files with the same extension within a session.
- **v2:** global/default language preference; pairs naturally with registering more grammars
  (ROADMAP §9).

## Open questions

- **Persistence:** ephemeral (v1, recommended) vs. a lightweight per-(review, file) store. A store
  would mirror `file_view_state` (`0002_file_view_state.sql` + a `set_file_*` command) — a new
  migration + command + `api.ts` wrapper. Worth it only if users frequently re-open the same
  review and re-set overrides.
- Dropdown scope: only the ~25 languages in `EXT_LANG`, or all `refractor`-registered languages
  the bundle ships?
- Friendly display names vs. raw refractor ids in the dropdown.

## Acceptance criteria & verification

- An extensionless or misdetected file can be highlighted by selecting a language; toggling
  highlight off renders plain text; both affect only that file.
- Default behavior (no override) is unchanged from today for normally-detected files.
- `pnpm exec tsc --noEmit` passes (note `DiffViewer.tsx` still compiles after the `tokenizeFile`
  signature change); verified live in `pnpm tauri dev` on a review containing an extensionless
  file.
