# 01 — Comment Markdown rendering primitive

**Layer:** Frontend only · **Dependencies:** none · **Wave:** 1

> Read `00-overview.md` first (locked decisions, conventions, anchors).

## Goal

Render comment bodies as Markdown instead of plain text, and provide a reusable `<Markdown>`
component that later specs (PR description, GitHub thread comments) also use. Comments are destined
for Markdown export / GitHub anyway, so authoring them as Markdown and previewing the render is the
natural fit (ROADMAP §1 "Comment Markdown").

## Why this first

Both §3 features render Markdown coming from GitHub (PR body in spec 03, thread comment bodies in
spec 05). This spec delivers the shared, **sanitized** rendering primitive they depend on, and on
its own upgrades local comment display.

## Files to touch

- `package.json` / `pnpm-lock.yaml` — add deps.
- **New:** `src/components/Markdown.tsx` — the rendering component.
- **New:** `src/components/Markdown.test.tsx` — unit test.
- `src/components/ReviewView.tsx` — `CommentItem` (≈ line 883): add display/preview rendering.
- `src/styles.css` (the global stylesheet, imported in `src/main.tsx`; `.comment-item` styles live
  here) — add `.markdown-body` styles.

## Steps

1. **Add dependencies** with pnpm:
   ```bash
   pnpm add react-markdown remark-gfm
   ```
   (Do not add `rehype-raw` — raw HTML must stay escaped for safety.)

2. **Create `src/components/Markdown.tsx`:**
   - Export `function Markdown({ source }: { source: string })`.
   - Render `<ReactMarkdown remarkPlugins={[remarkGfm]}>` wrapped in a `<div className="markdown-body">`.
   - Override the `a` renderer so links open in the OS browser via `api.openUrl` rather than
     navigating the webview:
     ```tsx
     components={{
       a: ({ href, children }) => (
         <a
           href={href}
           onClick={(e) => { e.preventDefault(); if (href) api.openUrl(href); }}
         >{children}</a>
       ),
     }}
     ```
   - Keep it presentational and dependency-light. No `dangerouslySetInnerHTML`.

3. **Add `.markdown-body` CSS** (reuse existing CSS variables for color/spacing). Cover: paragraph
   spacing, `ul/ol`, `code`/`pre` (monospace + subtle background), `blockquote`, `table`, `h1–h4`
   scaled down to fit the narrow comment column, and `img { max-width: 100% }`. Keep it compact —
   comments are small.

4. **Wire into `CommentItem`** (`ReviewView.tsx` ≈ 883):
   - Current behavior: always shows an editable `<textarea>` bound to `body` with 400ms autosave,
     plus the `outdated`/`origin` badges and the delete button. Preserve all of that.
   - New behavior:
     - When `readOnly` (published review): render `<Markdown source={comment.body} />` instead of
       the textarea (no editing affordance).
     - When editable: add a small **Write / Preview** toggle (local `useState`, default "Write").
       "Write" shows the existing textarea (autosave unchanged). "Preview" shows
       `<Markdown source={body} />` of the current draft text.
   - Do **not** change the autosave call, the `outdated` stale-badge logic, the `showOrigin` badge,
     or the delete flow.

5. `FileViewPane` reuses `CommentItem`, so it inherits this automatically — no change needed there.

## Acceptance criteria

- Local comment bodies render GFM: lists, fenced code, links (opening via `api.openUrl`), tables,
  task lists, strikethrough.
- Editing a draft comment still autosaves (debounced) and the Write/Preview toggle flips between the
  textarea and the rendered preview without losing in-progress text.
- Published-review comments render Markdown read-only (no textarea).
- Raw HTML in a body is **escaped, not executed** (e.g. a `<script>` or `<img onerror>` in the body
  renders as text).
- `pnpm exec tsc --noEmit` clean.

## Verification

- `pnpm exec tsc --noEmit`
- `pnpm test` — `Markdown.test.tsx` must assert: (a) `**bold**`/list/link render to the expected DOM;
  (b) a string containing `<script>alert(1)</script>` or `<img src=x onerror=...>` does **not**
  produce a live `<script>`/`onerror` node (it's escaped). Use `@testing-library/react` (already set
  up in `src/test/setup.ts`).
- Manual: `pnpm tauri dev`, add a comment with Markdown, confirm the preview and the saved render.

## Notes / gotchas

- Keep `<Markdown>` free of app state so specs 03 and 05 can drop it in anywhere.
- The export path (`export.rs`) already **emits** Markdown — this spec only affects in-app
  **display**. No backend change, no export change.
