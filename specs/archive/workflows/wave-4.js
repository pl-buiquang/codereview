// Run via the Workflow tool. Wave 4 ONLY — PENDING/draft GitHub reviews (19) + keyboard nav (14).
// After it merges, author specs/workflows/wave-5.js (spec 21, the provider seam — solo).
// Mirrors wave-2/3: parallel Implement tracks in isolated worktrees, then one Integrate agent in
// the main checkout. Merge order 19 → 14 (ORCHESTRATION §5 wave 4).
export const meta = {
  name: 'wave-4-implementation',
  description: 'Implement specs 19 (pending reviews) and 14 (keyboard nav) in parallel worktrees, then integrate into main (order 19→14)',
  phases: [
    { title: 'Implement', detail: '2 parallel worktree tracks: 19 | 14' },
    { title: 'Integrate', detail: 'merge 19 → 14, gates per merge, migration-0009 FK check, ReviewView reconciliation, push, CI watch' },
  ],
}

const REPO = '/Users/pbuiquang/projects/codereview'

const COMMON = `
You are an implementation agent working in an ISOLATED GIT WORKTREE of the repo (your working directory — confirm with \`git rev-parse --show-toplevel\` and use that as your root; it is NOT ${REPO}, never edit files under ${REPO} directly).

Protocol (this is specs/ORCHESTRATION.md §7 — read that section to confirm):
1. First create your branch: \`git checkout -b <branch named below>\`.
2. Run \`pnpm install\` before anything else (node_modules is not shared). The pinned toolchains are already on main (rust-toolchain.toml / .nvmrc).
3. Read your spec file(s) COMPLETELY before writing code, plus the "Discrepancies" section of specs/ORCHESTRATION.md for notes that affect you. Implement the spec exactly; where the spec and current code disagree on line anchors, the CODE wins on location, the SPEC wins on intent. IMPORTANT: Waves 1–3 are already merged on main (specs 04/05/06/07/08/09/10/11/12/13/15/16/17/18/20). Present and relevant: \`build_publish_payload\`/\`inline_publish_comments\`/\`ensure_draft\`/\`reanchor_pass\` in review.rs, \`gh::post_review\`, the \`status\` column ('draft'|'published'), migrations through **0008** (0007 resolved_at, 0008 anchored_base_sha), and on the frontend \`FileReview\`/\`FileJumpList\`/\`Composer\`/selection plumbing + \`statusLabel\`-less badges in ReviewView/RepoView/ReviewsView. Do NOT re-implement merged work; build on it.
4. Touch ONLY the files your spec lists ("Files touched" in the spec / "Files claimed" in ORCHESTRATION.md §5 wave 4). If you believe you must touch an unclaimed file, STOP work on that point and report it as a deviation instead of editing (a forced one-line fixture/type edit to keep a gate green is acceptable if reported as a deviation).
5. Add the tests your spec's Test matrix requires. SKIP the spec's "Manual verify" section (done later) — automated gates only.
6. Gate suite — ALL FIVE must pass in your worktree before you finish (fix failures; do not skip):
   pnpm exec tsc --noEmit
   pnpm build
   pnpm test
   cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
   cargo test --manifest-path src-tauri/Cargo.toml
7. Commit in small steps per the spec's Tasks list, message style \`type(scope): summary\` matching the repo log (feat(review), fix(review), docs(roadmap), …) — single-line header, NO body, NO Co-Authored-By trailer. Each commit leaves the tree green. Do NOT merge, do NOT push, do NOT delete branches.
8. Your final message is data for the orchestrator: report branch, commit shas+titles, gate results, and any deviations from the spec (with reasons). Call out anything the integrator must reconcile (shared files: ReviewView.tsx, styles.css, ROADMAP.md).
`

const SCHEMA = {
  type: 'object',
  properties: {
    branch: { type: 'string' },
    commits: { type: 'array', items: { type: 'string' } },
    gatesPassed: { type: 'boolean' },
    gateDetails: { type: 'string' },
    deviations: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
  required: ['branch', 'commits', 'gatesPassed', 'gateDetails', 'deviations'],
}

const TRACKS = [
  {
    label: 'impl:19',
    prompt: `${COMMON}
## Your track: spec 19 (ORCHESTRATION.md wave-4 track 4a)
Branch: spec/19-pending-reviews
Implement specs/19-pending-reviews.md: push the whole local draft to GitHub as a PENDING review (event omitted), then Submit or Discard it. Narrow v1 — no incremental comment sync. Files: NEW migration src-tauri/src/db/migrations/0009_review_status_pending.sql, src-tauri/src/db/mod.rs (push 0009 + an upgrade test), src-tauri/src/commands/review.rs (YOUR HEAVY SLOT: build_pending_payload that strips "event" from build_publish_payload WITHOUT changing build_publish_payload's signature; ensure_draft keeps the review locked while published_pending; the 3 commands publish-pending / submit_pending / discard_pending; block delete_review while pending), src-tauri/src/gh.rs (post_review pending path with event omitted; submit endpoint POST .../reviews/{id}/events; delete-pending DELETE .../reviews/{id}; friendly remap of the one-pending-review-per-PR 422 only when the message contains "pending review" case-insensitively), src-tauri/src/lib.rs (register the 3 new commands), src/lib/types.ts (status gains 'published_pending'), src/lib/api.ts, NEW src/lib/status.ts (statusLabel() helper, used by the three badge sites), src/components/ReviewView.tsx (header Submit/Discard buttons + readOnly while pending), src/components/RepoView.tsx, src/components/ReviewsView.tsx (badges via statusLabel), src/styles.css, ROADMAP.md.
CRITICAL — migration 0009 is a TABLE REBUILD and the single highest-risk change in this wave. \`comment\` and \`file_view_state\` hold \`REFERENCES review(id) ON DELETE CASCADE\`; foreign_keys is ON at open, so a naive DROP TABLE review fires those cascades and DELETES EVERY COMMENT. Follow the spec's recipe EXACTLY: \`PRAGMA foreign_keys = OFF;\` OUTSIDE the transaction (it's a no-op inside one), BEGIN/…/COMMIT the rebuild, copy all rows, re-create the indexes. 0009 MUST be the 9th MIGRATIONS entry (verify 0007+0008 are present: \`ls src-tauri/src/db/migrations\`). Your db/mod.rs test MUST prove: a 0008 DB WITH review+comment rows upgrades to 0009 with the comments STILL PRESENT and \`PRAGMA foreign_key_check\` empty. This test is mandatory.`,
  },
  {
    label: 'impl:14',
    prompt: `${COMMON}
## Your track: spec 14 (ORCHESTRATION.md wave-4 track 4b)
Branch: spec/14-keyboard-nav
Implement specs/14-keyboard-nav.md: keyboard navigation on the review screen. FRONTEND-ONLY — zero Rust changes (the cargo gates must still pass, proving no backend drift). Files: NEW src/lib/keyboard.ts (+test — the exported \`BINDINGS\` array that drives BOTH the dispatch table and the help overlay, \`isEditableTarget\`, and the pure next/prev-pick + cursor index math), NEW src/components/ShortcutHelp.tsx (+test — the \`?\` overlay), src/components/FileJumpList.tsx (expose its existing activeIndex/jumpTo via an imperative control ref — do NOT duplicate the scrollspy), src/components/ReviewView.tsx (YOUR HEAVY SLOT: ONE window keydown listener with a small dispatch table, active-tab-gated via useUIStore activeTabId === \`review-\${reviewId}\`, an input guard ignoring input/textarea/select/[contenteditable] and modifier chords, a registry ref of per-file handles filled by each FileReview, a kbFocusKey line cursor local to FileReview, and \`c\` reusing the existing setSelection→Composer path), src/styles.css, README.md, ROADMAP.md.
Bindings v1: \`]\`/\`[\` next/prev file · \`n\`/\`p\` next/prev comment thread · \`j\`/\`k\` line cursor in the active file · \`c\` comment on focused line · \`?\` toggle help · Escape closes help→composer/selection→cursor. Shortcuts are inert while the file pane is open (filePanePath != null). NO new window listeners other than the single ReviewView one; leave FileViewPane's existing Escape handler as-is.
Note for the integrator (state it in your report): your ReviewView.tsx edits are the top-level keydown effect + FileReview/FileBody registry plumbing; spec 19 (merging BEFORE you) adds header Submit/Discard buttons + a readOnly path to the SAME file — mostly disjoint regions, you rebase over 19.`,
  },
]

phase('Implement')
const results = await parallel(TRACKS.map((t) => () =>
  agent(t.prompt, { label: t.label, phase: 'Implement', isolation: 'worktree', schema: SCHEMA })
))
const done = results.filter(Boolean)
log(`${done.length}/${TRACKS.length} tracks finished`)
const failedTracks = results
  .map((r, i) => (r && r.gatesPassed ? null : TRACKS[i].label))
  .filter(Boolean)
if (failedTracks.length) log(`Tracks with failed/missing gates: ${failedTracks.join(', ')} — integration will skip unmerged-red branches and report`)

phase('Integrate')
const integration = await agent(
  `You are the wave-4 INTEGRATION agent for ${REPO}. Work directly in ${REPO} (the main checkout, branch main). Read specs/ORCHESTRATION.md §5 wave 4, §6, and §8 first and follow them exactly. Waves 1–3 are already merged on main; the base SHA is the current main tip.

Track results from the implementation agents (JSON): ${'```'}${JSON.stringify(done)}${'```'}

The spec branches live in the same repo (worktree branches): spec/19-pending-reviews, spec/14-keyboard-nav.

Steps:
1. \`git -C ${REPO} branch -a\` to confirm the branches exist. Skip any track that reported gatesPassed=false (report it instead of merging).
2. Merge into main IN THIS ORDER with \`git merge --no-ff <branch>\`: spec/19-pending-reviews → spec/14-keyboard-nav. (19 adds the header buttons + readOnly; 14 rebases its keydown effect + diff plumbing over it — ORCHESTRATION §5 wave 4.)
3. EXPECTED CONFLICTS / RECONCILIATION:
   - src/components/ReviewView.tsx — the only real shared surface. 19's region: header Submit/Discard buttons + a readOnly path threaded to the composer/affordances while status==='published_pending'. 14's region: a top-level window keydown effect (active-tab gated), a per-file registry ref, kbFocusKey plumbing through FileReview/FileBody, and \`c\`→setSelection. These are mostly DISJOINT (header vs top-level-effect+diff-plumbing); keep BOTH. After resolving, re-read the FileReview component, the header block, and the new keydown effect WHOLE to confirm coherence (esp. that 14's input-guard/active-tab gating and 19's readOnly both hold).
   - src/styles.css, ROADMAP.md — pure appends; keep BOTH sides.
   - 19-only files (no cross-track conflict, but keep all on merge): db/mod.rs MIGRATIONS (0009 is the 9th positional entry, after 0008 — never reorder/gap), src-tauri/src/commands/review.rs, gh.rs, lib.rs generate_handler! (3 new commands), src/lib/types.ts, api.ts, status.ts, RepoView.tsx, ReviewsView.tsx, the new migration. 14-only files: keyboard.ts(+test), ShortcutHelp.tsx(+test), FileJumpList.tsx, README.md.
4. After EACH merge run the full gate suite (§6: tsc, pnpm build, pnpm test, clippy -D warnings, cargo test). All green before the next merge. If a merge breaks gates, fix forward with a minimal \`fix(scope): …\` commit (single-line header, no body, no Co-Authored-By) and note it.
5. SPEC-SPECIFIC GATES (ORCHESTRATION §5 wave 4 "Wave-4 verification") — verify after the 19 merge:
   - migration 0009 is positionally the 9th include_str! entry in db/mod.rs MIGRATIONS (after 0007/0008), and the 19 agent's upgrade test proves a 0008 DB WITH comments upgrades to 0009 with comments INTACT and \`PRAGMA foreign_key_check\` empty. Run that test by name in isolation and confirm it passes (this is the migration that could silently delete every comment — verify, do not trust).
   - There is NO key material / NO release.yml change this wave — no key-guard needed.
6. Push to origin main after each successful merge+gates. After each push watch CI to green: \`gh run watch "$(gh run list --workflow ci.yml --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status\`. Fix forward, push, re-watch on red. (release.yml is tag-triggered — it will NOT run on these pushes; expected.)
7. Do NOT delete branches or worktrees (the orchestrator cleans up after the wave). Do NOT create tags.
8. Report: merges done (order + the ReviewView.tsx header/keydown reconciliation), gate results per merge, the migration-0009 FK-check spec gate result (explicitly), CI run conclusions + URLs, any fix-forward commits, and anything skipped.

Commit messages for merges: default git merge messages are fine; fix-forward commits use the repo's terse Conventional-Commit style (\`type(scope): summary\`, single-line header, NO body, NO Co-Authored-By trailer).`,
  { label: 'integrate:wave-4', phase: 'Integrate', schema: {
      type: 'object',
      properties: {
        merged: { type: 'array', items: { type: 'string' } },
        skipped: { type: 'array', items: { type: 'string' } },
        reconciliation: { type: 'string' },
        specGates: { type: 'string' },
        ciConclusion: { type: 'string' },
        fixForwardCommits: { type: 'array', items: { type: 'string' } },
        notes: { type: 'string' },
      },
      required: ['merged', 'skipped', 'ciConclusion', 'notes'],
    } }
)

return { tracks: done, integration }
