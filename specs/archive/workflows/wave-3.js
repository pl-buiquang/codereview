// Run via the Workflow tool. Wave 3 ONLY — LEFT-side re-anchoring (16), suggested changes (13),
// GitHub thread reply/resolve (18). After it merges, author specs/workflows/wave-4.js (specs 19, 14).
// Mirrors the wave-2 run's structure: parallel Implement tracks in isolated worktrees, then one
// Integrate agent in the main checkout. Merge order 18 → 13 → 16 (ORCHESTRATION §5 wave 3).
export const meta = {
  name: 'wave-3-implementation',
  description: 'Implement specs 18, 13, 16 in parallel worktrees, then integrate into main (order 18→13→16)',
  phases: [
    { title: 'Implement', detail: '3 parallel worktree tracks: 16 | 13 | 18' },
    { title: 'Integrate', detail: 'merge 18 → 13 → 16, gates per merge, ReviewView/export reconciliation, push, CI watch' },
  ],
}

const REPO = '/Users/pbuiquang/projects/codereview'

const COMMON = `
You are an implementation agent working in an ISOLATED GIT WORKTREE of the repo (your working directory — confirm with \`git rev-parse --show-toplevel\` and use that as your root; it is NOT ${REPO}, never edit files under ${REPO} directly).

Protocol (this is specs/ORCHESTRATION.md §7 — read that section to confirm):
1. First create your branch: \`git checkout -b <branch named below>\`.
2. Run \`pnpm install\` before anything else (node_modules is not shared). The pinned toolchains are already on main (rust-toolchain.toml / .nvmrc).
3. Read your spec file(s) COMPLETELY before writing code, plus the "Discrepancies" section of specs/ORCHESTRATION.md for notes that affect you. Implement the spec exactly; where the spec and current code disagree on line anchors, the CODE wins on location, the SPEC wins on intent. IMPORTANT: specs 00–03, Wave-1 (04/05/09/10/15/20) AND Wave-2 (06/07/08/11/12/17) are already merged on main — their changes are present. In particular: threaded replies + \`add_comment_impl\` + \`export::fold_replies\`/\`replies_by_root\` (spec 11), \`resolved_at\` + migration 0007 (spec 12), and the \`inline_publish_comments\` publish-body helper + \`Composer.submitLabel\` prop + comment-id capture (spec 17/11) ALL EXIST. Do NOT re-implement them; build on them.
4. Touch ONLY the files your spec lists ("Files touched" in the spec / "Files claimed" in ORCHESTRATION.md §5 wave 3). If you believe you must touch an unclaimed file, STOP work on that point and report it as a deviation instead of editing.
5. Add the tests your spec's Test matrix requires. SKIP the spec's "Manual verify" section (done later) — automated gates only.
6. Gate suite — ALL FIVE must pass in your worktree before you finish (fix failures; do not skip):
   pnpm exec tsc --noEmit
   pnpm build
   pnpm test
   cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
   cargo test --manifest-path src-tauri/Cargo.toml
7. Commit in small steps per the spec's Tasks list, message style \`type(scope): summary\` matching the repo log (feat(review), fix(review), docs(roadmap), …) — single-line header, NO body, NO Co-Authored-By trailer. Each commit leaves the tree green. Do NOT merge, do NOT push, do NOT delete branches.
8. Your final message is data for the orchestrator: report branch, commit shas+titles, gate results, and any deviations from the spec (with reasons). Call out anything the integrator must reconcile (shared files: ReviewView.tsx, export.rs, db/models.rs, api.ts/types.ts, styles.css, lib.rs).
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
    label: 'impl:18',
    prompt: `${COMMON}
## Your track: spec 18 (ORCHESTRATION.md wave-3 track 3c)
Branch: spec/18-github-thread-replies-resolve
Implement specs/18-github-thread-replies-resolve.md: reply to + resolve/unresolve existing GitHub PR review threads from the app (no local persistence, no migration). Files: src-tauri/src/gh.rs (reply_to_thread REST + resolve/unresolve GraphQL mutations + parse_rest_id helper), src-tauri/src/commands/gh.rs (set_pr_thread_resolved + reply command), src-tauri/src/lib.rs (register the new commands in generate_handler!), src/lib/api.ts, src/lib/types.ts, src/components/GithubThread.tsx (+test — add Reply composer + Resolve/Unresolve button), src/components/ReviewView.tsx (threadCtx plumbing + pass Composer to GithubThread), src/styles.css.
CRITICAL (ORCHESTRATION Discrepancy #3): \`Composer\` in ReviewView.tsx ALREADY has the optional \`submitLabel\` prop (spec 11 added it in wave 2 — verify with \`grep -n submitLabel src/components/ReviewView.tsx\`). DO NOT re-add it; just USE it (pass submitLabel="Reply"). If you find it missing, report as a deviation rather than guessing.
Thread actions are NOT gated by local review status (they mutate GitHub, not the draft) — available even on a published/read-only review; no ensure_draft, no readOnly check for thread actions.`,
  },
  {
    label: 'impl:13',
    prompt: `${COMMON}
## Your track: spec 13 (ORCHESTRATION.md wave-3 track 3b)
Branch: spec/13-suggested-changes
Implement specs/13-suggested-changes.md: GitHub-style \`\`\`suggestion blocks — an "Insert suggestion" affordance in the composer that seeds the current RIGHT-side text, and a labeled "Suggested change" render in Markdown. NO backend behavior change (publish/export already pass the body verbatim); lock that with ONE Rust regression test in export.rs that a suggestion fence survives render_markdown. Files: src/lib/diff.ts (+test — rightLinesText + buildSuggestionSeed/fence helpers, pure & vitest-covered), src/components/ReviewView.tsx (YOU OWN THE WAVE'S ReviewView-HEAVY SLOT: Composer "Insert suggestion" button, CommentItem suggestionSeed prop, LineWidget/FileReview plumbing of hunks→seed), src/components/Markdown.tsx (+test — render language-suggestion fences as a labeled green panel; empty body = "(removes the selected lines)"), src/styles.css, src-tauri/src/export.rs (test only), ROADMAP.md.
Suggestions appear ONLY for side==="RIGHT" && subject_type==="line" && origin==="diff"; hidden (not disabled) when any selected line can't be resolved from rendered hunks or the comment is outdated (anchored_head_sha mismatch). No FileViewPane button. No new deps, no DB change.
Note for the integrator (state it in your report): your CommentItem gains an additive \`suggestionSeed\`-style prop; spec 16 (merging AFTER you) adds an additive \`baseSha\` prop to the same CommentItem — both must survive.`,
  },
  {
    label: 'impl:16',
    prompt: `${COMMON}
## Your track: spec 16 (ORCHESTRATION.md wave-3 track 3a)
Branch: spec/16-left-side-reanchoring
Implement specs/16-left-side-reanchoring.md: re-anchor LEFT/base-side comments when the base/merge-base advances (RIGHT-side already re-anchors). Read the spec's "Decisions (locked)" section in full — it is precise. Files: NEW migration src-tauri/src/db/migrations/0008_comment_anchored_base_sha.sql (single ALTER TABLE comment ADD COLUMN anchored_base_sha TEXT) — it MUST be the 8th entry in the db/mod.rs MIGRATIONS array (current max on main is 0007 from wave 2; verify with \`ls src-tauri/src/db/migrations\`). Also: src-tauri/src/db/mod.rs (push 0008 + an upgrade test), src-tauri/src/db/models.rs (Comment gains \`anchored_base_sha\` — update EVERY \`Comment {}\` struct literal incl. all export.rs/review.rs test fixtures), src-tauri/src/git.rs, src-tauri/src/anchor.rs (RENAME \`remap_right_line\`→\`remap_line\` with a direction doc-comment; callers live only in review.rs + anchor.rs tests), src-tauri/src/commands/review.rs (YOUR HEAVY SLOT: two-pass reanchor over one shared side-parameterized helper; LEFT base-pin resolved backend-side in the insert path; \`is_anchored_to\` becomes side-aware and takes &Target; local target base_sha becomes merge-base when three_dot), src-tauri/src/export.rs (Comment fixtures only), src/lib/types.ts, NEW src/lib/staleness.ts (+ wire the per-comment badge + header banner through it), src/components/ReviewView.tsx (banner predicate + CommentItem \`baseSha\` prop — additive, light touch), src/components/FileViewPane.tsx, ROADMAP.md.
CRITICAL (ORCHESTRATION Discrepancy #4): the publish gate \`is_anchored_to\` is called from inside \`inline_publish_comments\` (spec 17's helper, already on main — NOT in build_publish_payload). Update the call AT THAT SITE. Confirm with \`grep -n "is_anchored_to\\|inline_publish_comments" src-tauri/src/commands/review.rs\` before editing.
After you finish, \`grep -rn "remap_right_line" src-tauri/src\` MUST be empty (the rename is complete).`,
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
  `You are the wave-3 INTEGRATION agent for ${REPO}. Work directly in ${REPO} (the main checkout, branch main). Read specs/ORCHESTRATION.md §5 wave 3, §6, and §8 first and follow them exactly. Wave 2 is already merged on main; the base SHA is the current main tip.

Track results from the implementation agents (JSON): ${'```'}${JSON.stringify(done)}${'```'}

The spec branches live in the same repo (worktree branches): spec/18-github-thread-replies-resolve, spec/13-suggested-changes, spec/16-left-side-reanchoring.

Steps:
1. \`git -C ${REPO} branch -a\` to confirm the branches exist. Skip any track that reported gatesPassed=false (report it instead of merging).
2. Merge into main IN THIS ORDER with \`git merge --no-ff <branch>\`: spec/18-github-thread-replies-resolve → spec/13-suggested-changes → spec/16-left-side-reanchoring. (Order is deliberate: 18 light, 13 ReviewView-heavy mid-order, 16 additive props last — ORCHESTRATION §5 wave 3.)
3. EXPECTED CONFLICTS / RECONCILIATION (all three touch ReviewView.tsx; Discrepancy #6 says expect non-append conflicts there):
   - src/components/ReviewView.tsx — the real merge work. Keep ALL additions: 18's thread reply/resolve plumbing (threadCtx, Composer passed to GithubThread with submitLabel="Reply"), 13's Composer "Insert suggestion" button + CommentItem suggestion-seed prop + LineWidget/FileReview hunk plumbing, and 16's banner predicate (now via src/lib/staleness.ts) + CommentItem additive \`baseSha\` prop. CommentItem must end with BOTH 13's suggestion-seed prop AND 16's baseSha prop. After resolving, re-read CommentItem, Composer, FileReview and the banner predicate WHOLE to confirm coherence.
   - Composer.submitLabel: added once by spec 11 (wave 2). Spec 18 must only USE it. If a branch re-introduced it (duplicate prop/destructure), keep ONE (Discrepancy #3).
   - src-tauri/src/export.rs — spec 16 edits Comment{} test fixtures (new \`anchored_base_sha\` field); spec 13 ADDS a regression test (likely with its own Comment{} literal). Since 16 merges AFTER 13 and adds the field to the struct, ANY Comment{} literal 13 introduced will fail to compile until it also gets \`anchored_base_sha: None\` (or the spec's chosen default). Add the field to 13's fixture during/after the 16 merge — this is the most likely gate breakage; fix it forward.
   - src-tauri/src/db/models.rs — only spec 16 touches the Comment struct; every Comment{} literal across review.rs/export.rs must carry the new field (the 16 agent did this within its own tree; the only stragglers are literals introduced by 13 — see above).
   - db/mod.rs MIGRATIONS: keep 0008 as the 8th positional entry (index 7, after 0007) — never reorder/gap.
   - src-tauri/src/lib.rs (generate_handler! + mod lines), src/lib/api.ts, src/lib/types.ts, src/styles.css, ROADMAP.md — pure appends; keep BOTH sides (mods alphabetical).
4. After EACH merge run the full gate suite (§6: tsc, pnpm build, pnpm test, clippy -D warnings, cargo test). All green before the next merge. If a merge breaks gates, fix forward with a minimal \`fix(scope): …\` commit (single-line header, no body, no Co-Authored-By) and note it.
5. SPEC-SPECIFIC GATES (ORCHESTRATION §5 wave 3 "Wave-3 verification"):
   - After 16 is merged: \`grep -rn "remap_right_line" src-tauri/src\` MUST be empty (rename complete), and \`grep -rn "anchored_base_sha" src-tauri/src/db/migrations\` shows 0008.
   - Confirm migration 0008 is positionally 8th in db/mod.rs MIGRATIONS and the 16 agent's upgrade test (0008 applies on a 0007 DB) is present and green.
   - There is NO key material and NO release.yml change this wave — no key-guard needed.
6. Push to origin main after each successful merge+gates. After each push watch CI to green: \`gh run watch "$(gh run list --workflow ci.yml --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status\`. Fix forward, push, re-watch on red. (release.yml is tag-triggered — it will NOT run on these pushes; expected.)
7. Do NOT delete branches or worktrees (the orchestrator cleans up after the wave). Do NOT create tags.
8. Report: merges done (order + the ReviewView.tsx/CommentItem reconciliation + the export.rs fixture fix), gate results per merge, the remap_right_line/migration-0008 spec gates, CI run conclusions + URLs, any fix-forward commits, and anything skipped.

Commit messages for merges: default git merge messages are fine; fix-forward commits use the repo's terse Conventional-Commit style (\`type(scope): summary\`, single-line header, NO body, NO Co-Authored-By trailer).`,
  { label: 'integrate:wave-3', phase: 'Integrate', schema: {
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
