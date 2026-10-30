# Image diffs

A review that changes PNG, JPEG or SVG files (screenshots, icons) can show them as **before / after / diff**, like the `snapshot-diff` skill does, from a panel instead of an HTML page. The comparison runs in the background and is cached on disk, so opening a review never waits for it.

## In the review

- **Review actions menu**, first entry, only when the diff changes images:
  - `Generate image diffs (N)` when there are none yet (also after a failed or interrupted run);
  - `Generating image diffs… d/N` while running (opens the panel);
  - `View image diffs`, or `View image diffs (outdated)` when the review's pin moved since they were generated.
- **Inline links.** Under an image file's "Binary file not shown." note, and under the collapsed text diff of an SVG, a link opens the panel on that file (generating first when needed). Every `.svg` text diff is collapsed by default ("Show anyway" renders it): a minified or base64-carrying SVG is one huge line that freezes highlighting.
- **Automatic run.** When a review's diff loads, has a linked clone, and changes at most **20 images and 25 megapixels** in total (sizes read from the headers or the SVG `viewBox`), the diffs are generated without a click, once the clone has fetched the review's commits. It runs again when the pin moves and the set is still small. Bigger sets (a full-page screenshot of a report is 40 MP) and reviews without a clone wait for the menu entry. A toast announces the result with a "View" button.

## The panel

Full width, `Esc` or a click outside closes it. Left: the changed images with a badge each (`0.5%` of pixels differ, `NEW`, `DEL`, `same`, `skipped`). Right, two tabs:

- **Images.** Before | after | diff columns (red = changed, yellow = anti-aliasing, grey = unchanged), with the regions outlined on the diff and listed with their close-ups. Click a picture for a zoom/pan overlay (wheel, drag, `Esc`). Images of different sizes are compared on a white canvas, top-left aligned.
- **Report.** The AI report (below).

Header: progress with Cancel while running, Regenerate otherwise, and the AI report button. A cache generated for an older version of the review stays viewable with a notice.

## AI report

`AI report` runs the built-in routine **Image diff report** (Config > Routines, editable like the others, app default model). It reads the manifest and the close-ups of every changed region, groups regions that are one change, drops rendering noise, and writes a summary plus findings (title, kind, description). The app validates what it wrote, clamps the boxes and cuts a before / after / diff close-up per finding, with context around it. Nothing is posted anywhere and no comment is created.

It is disabled when AI is off in Settings, when `claude` is missing, or when no pixel changed. After an **automatic** generation it also starts on its own under the same conditions (and again after an automatic regeneration).

## How it works

- `src-tauri/src/image_diff/`: `pixelmatch.rs` (a port of pixelmatch's YIQ delta and anti-aliasing detection; anti-aliased pixels are told apart from real changes and don't make regions), `regions.rs` (changed pixels bucketed in 16 px cells, cells within about 48 px joined, at most 30 regions per file), `job.rs` (one file at a time: decode, compare, write full-size, preview and crop PNGs), `blob.rs` (bytes from the local clone, else the GitHub contents API), `runner.rs` (one job at a time app-wide, the auto-run rules, status), `report.rs` (validates the AI report), `store.rs` (the cache layout).
- **Cache**: `<app data dir>/image_diffs/<db file stem>/<review id>/` with `manifest.json` (written last; it records the pin, so a moved pin reads as outdated), `progress.json`, `f<i>/{before,after,diff}.png`, `preview-*.png` for pictures wider than 1000 px, `crop-<region>-*.png`, and `report.final.json` + `report/`. It is deleted with the review (`delete_review_impl`, so the merged-PR cleanup and `cr` too), and wiped on Regenerate.
- **Serving**: the webview loads the PNGs through the private `crimg://` scheme (`lib.rs`), which only serves `.png` files under the review's cache directory.
- Images over 80 MP are skipped; SVGs are rasterized with `resvg` at their own size (capped at 40 MP).
- Events: `image-diff-updated` (generation) and `image-diff-report-updated` (report); the frontend invalidates its queries from them (`src/lib/imageDiff.ts`).
