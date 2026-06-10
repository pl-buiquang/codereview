// Run via the Workflow tool. Wave 2 ONLY — after it merges, do the updater-keygen USER CHECKPOINT
// (tauri signer generate + gh secret set + backup + paste real pubkey; see ORCHESTRATION §7 rule 8
// and Discrepancy #8), then author specs/workflows/wave-3.js. Mirrors the wave-1 run's structure.
export const meta = {
  name: 'wave-2-implementation',
  description: 'Implement specs 17, 11→12, and 06→07→08 in parallel worktrees, then integrate into main',
  phases: [
    { title: 'Implement', detail: '3 parallel worktree tracks: 17 | [11→12] | [06→07→08]' },
    { title: 'Integrate', detail: 'merge 17 → 11/12 → 06/07/08, gates per merge, key-guard greps, push, CI watch' },
  ],
}

const REPO = '/home/paul/projects/codereview'

const COMMON = `
You are an implementation agent working in an ISOLATED GIT WORKTREE of the repo (your working directory — confirm with \`git rev-parse --show-toplevel\` and use that as your root; it is NOT ${REPO}, never edit files under ${REPO} directly).

Protocol (this is specs/ORCHESTRATION.md §7 — read that section to confirm):
1. First create your branch: \`git checkout -b <branch named below>\`.
2. Run \`pnpm install\` before anything else (node_modules is not shared). The pinned toolchains are already on main (rust-toolchain.toml / .nvmrc).
3. Read your spec file(s) COMPLETELY before writing code, plus the "Discrepancies" section of specs/ORCHESTRATION.md for notes that affect you. Implement the spec exactly; where the spec and current code disagree on line anchors, the code wins on location, the spec wins on intent. Note specs 00–03 and Wave-1 specs (04/05/09/10/15/20) are already merged on main — their changes are present; do not re-implement them.
4. Touch ONLY the files your spec lists ("Files touched" in the spec / "Files claimed" in ORCHESTRATION.md §5 wave 2). If you believe you must touch an unclaimed file, STOP work on that point and report it as a deviation instead of editing.
5. Add the tests your spec's Test matrix requires. SKIP the spec's "Manual verify" section (done later) — automated gates only.
6. Gate suite — ALL FIVE must pass in your worktree before you finish (fix failures; do not skip):
   pnpm exec tsc --noEmit
   pnpm build
   pnpm test
   cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
   cargo test --manifest-path src-tauri/Cargo.toml
7. Commit in small steps per the spec's Tasks list, message style \`type(scope): summary\` matching the repo log (feat(review), ci(release), docs(signing), …). Each commit leaves the tree green. Do NOT merge, do NOT push, do NOT delete branches.
8. Your final message is data for the orchestrator: report branch(es), commit shas+titles, gate results, and any deviations from the spec (with reasons).
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
    label: 'impl:17',
    prompt: `${COMMON}
## Your track: spec 17 (ORCHESTRATION.md wave-2 track 2b)
Branch: spec/17-capture-github-comment-ids
Implement specs/17-capture-github-comment-ids.md: after publishing a review, capture the GitHub-assigned id of each inline comment we posted. Files: src-tauri/src/gh.rs (add a \`ReviewComment\` struct + a \`review_comments\` fetch), src-tauri/src/commands/review.rs (EXTRACT the body-as-posted into a helper named \`inline_publish_comments\` — this becomes the single source of truth that specs 11 and 19 build on later; then match posted comments back to local rows and store their ids), ROADMAP.md.
IMPORTANT (Discrepancy #2): you OWN the extraction of \`inline_publish_comments\`. Make it the place where the published inline-comment list is built, so spec 11's \`fold_replies\` and roots-only filter can move INTO this helper at integration time. Keep it cleanly factored.`,
  },
  {
    label: 'impl:11+12',
    prompt: `${COMMON}
## Your track: specs 11 then 12, SEQUENTIAL, same worktree (ORCHESTRATION.md wave-2 track 2a)
Branch: spec/11-threaded-replies
1. Implement specs/11-threaded-replies.md FULLY: render replies under a root comment and let the user reply (the comment.parent_id column already exists). Files: src-tauri/src/commands/review.rs (add_comment supports parent_id; publish folds replies into the body), src-tauri/src/export.rs, NEW src/lib/threads.ts, src/lib/api.ts, src/components/ReviewView.tsx (you own the wave's ReviewView-heavy slot), src/components/FileViewPane.tsx, src/styles.css. Run the gates, commit.
2. Then \`git checkout -b spec/12-resolve-threads\` (off 11's tip) and implement specs/12-resolve-threads.md: mark a comment thread resolved / collapse resolved threads. This adds migration **0007_comment_resolved.sql** — it MUST be the 7th entry in the db/mod.rs MIGRATIONS array (current max on main is 0006). Files: NEW migration 0007, src-tauri/src/db/mod.rs, src-tauri/src/db/models.rs (Comment gets a resolved field — update EVERY Comment {} struct literal incl. export.rs/review.rs test fixtures), commands/review.rs, export.rs, lib.rs (register set_comment_resolved), types.ts, api.ts, NEW src/lib/text.ts, ReviewView.tsx, styles.css. Run the gates, commit.
Report branch "spec/12-resolve-threads" as your tip (it carries both specs' commits; 11's tip is also tagged by its own branch name spec/11-threaded-replies). List BOTH branch names in notes.`,
  },
  {
    label: 'impl:06+07+08',
    prompt: `${COMMON}
## Your track: specs 06 then 07 then 08, SEQUENTIAL, same worktree (ORCHESTRATION.md wave-2 track 2c)
Branch chain: spec/06-release-pipeline → spec/07-auto-update → spec/08-code-signing-readiness
1. Implement specs/06-release-pipeline.md: NEW .github/workflows/release.yml (tag-triggered app-v* matrix build → draft GitHub Release). Gates, commit on spec/06-release-pipeline.
2. \`git checkout -b spec/07-auto-update\` (off 06's tip) and implement specs/07-auto-update.md: Tauri updater plugin + updater artifacts in release.yml. Files: release.yml, src-tauri/tauri.conf.json, src-tauri/Cargo.toml, src-tauri/src/lib.rs (plugin registration), src-tauri/capabilities/default.json, .gitignore (add *.key), package.json, NEW src/lib/updater.ts, NEW src/components/UpdateBanner.tsx, src/App.tsx, src/styles.css.
   *** USER CHECKPOINT — DO NOT generate keys or set secrets *** (ORCHESTRATION §7 rule 8, Discrepancy #8): the minisign keygen, \`gh secret set TAURI_SIGNING_PRIVATE_KEY[_PASSWORD]\`, backup, and pasting the REAL pubkey are a user step AFTER this wave. In tauri.conf.json plugins.updater.pubkey put a clearly-marked placeholder string and a \`TODO(user): replace with real pubkey from 'tauri signer generate' — see specs/08-code-signing-readiness.md / ORCHESTRATION Discrepancy #8\` comment-adjacent note. NEVER create/push RC tags. Gates, commit.
3. \`git checkout -b spec/08-code-signing-readiness\` (off 07's tip) and implement specs/08-code-signing-readiness.md: NEW docs/signing.md, conditional code-signing wiring in release.yml (signs only when cert secrets are set — none are today), README.md. Skip the spec's live throwaway-RC runs (deferred to wave 6). Gates, commit.
KEY-GUARD: before every commit, \`git diff --cached | grep -niE "PRIVATE KEY|untrusted comment|secret key"\` MUST be empty. If actionlint is unavailable, validate each workflow YAML with \`python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"\` and note it.
Report branch "spec/08-code-signing-readiness" as your tip; list all three branch names (06/07/08) in notes.`,
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
  `You are the wave-2 INTEGRATION agent for ${REPO}. Work directly in ${REPO} (the main checkout, branch main). Read specs/ORCHESTRATION.md §5 wave 2, §6, and §8 first and follow them exactly.

Track results from the implementation agents (JSON): ${'```'}${JSON.stringify(done)}${'```'}

The spec branches live in the same repo (worktree branches): spec/17-capture-github-comment-ids, spec/11-threaded-replies, spec/12-resolve-threads, spec/06-release-pipeline, spec/07-auto-update, spec/08-code-signing-readiness.

Steps:
1. \`git -C ${REPO} branch -a\` to confirm the branches exist. Skip any track that reported gatesPassed=false (report it instead of merging).
2. Merge into main IN THIS ORDER with \`git merge --no-ff <branch>\`: spec/17-capture-github-comment-ids → spec/11-threaded-replies → spec/12-resolve-threads → spec/06-release-pipeline → spec/07-auto-update → spec/08-code-signing-readiness. (11 then 12 reproduces the 11→12 chain; 06→07→08 likewise.)
3. THE ONE NON-TRIVIAL MERGE (Discrepancy #2): when spec/11 merges over the already-merged spec/17, its publish-body changes conflict with 17's \`inline_publish_comments\` helper. Resolve by moving 11's \`fold_replies\` call and the roots-only \`parent_id.is_none()\` filter INSIDE \`inline_publish_comments\` (the single body-as-posted source of truth) — NOT in build_publish_payload. Re-read the whole helper after resolving to confirm coherence.
   Other conflicts are appends — keep BOTH sides: src/lib/api.ts & types.ts (wrappers/fields), src-tauri/src/lib.rs (generate_handler! + mod lines, mods alphabetical), src/styles.css, ROADMAP.md/README.md, package.json. db/mod.rs MIGRATIONS: keep 0007 as the 7th (positional) entry — never reorder/gap. Discrepancy #3: spec 11 already adds Composer.submitLabel; if a later spec re-adds it, keep one.
4. After EACH merge run the full gate suite (§6: tsc, pnpm build, pnpm test, clippy -D warnings, cargo test). All green before the next merge. If a merge breaks gates, fix forward with a minimal \`fix(scope): …\` commit and note it.
5. KEY-GUARD (§8.4 / Discrepancy #8): before merging spec/07 and spec/08, \`git diff main..<branch> | grep -niE "PRIVATE KEY|untrusted comment|secret key"\` MUST be empty. After all merges, \`git grep -qi "untrusted comment"\` on the merged tree MUST find nothing. Confirm tauri.conf.json still has the PLACEHOLDER pubkey + TODO(user) (the real keygen is a user step after this wave — do NOT generate keys, set secrets, or push tags).
6. Workflow lint: run actionlint on .github/workflows/release.yml if available, else \`python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"\`. Note the result.
7. Push to origin main after each successful merge+gates. After each push watch CI to green: \`gh run watch "$(gh run list --workflow ci.yml --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status\`. Fix forward, push, re-watch on red. (Note: release.yml is tag-triggered — it will NOT run on these pushes; that's expected. Live RC verification is wave 6.)
8. Do NOT delete branches or worktrees. Do NOT create tags.
9. Report: merges done (order + the inline_publish_comments/fold_replies reconciliation notes), gate results per merge, key-guard + actionlint results, CI run conclusions + URLs, fix-forward commits, anything skipped, and a reminder that the updater-keygen user checkpoint is now due before wave 6.

Commit messages for merges: default git merge messages are fine; fix-forward commits use repo style + the Co-Authored-By: Claude Fable 5 <noreply@anthropic.com> trailer.`,
  { label: 'integrate:wave-2', phase: 'Integrate', schema: {
      type: 'object',
      properties: {
        merged: { type: 'array', items: { type: 'string' } },
        skipped: { type: 'array', items: { type: 'string' } },
        reconciliation: { type: 'string' },
        keyGuard: { type: 'string' },
        ciConclusion: { type: 'string' },
        fixForwardCommits: { type: 'array', items: { type: 'string' } },
        notes: { type: 'string' },
      },
      required: ['merged', 'skipped', 'ciConclusion', 'notes'],
    } }
)

return { tracks: done, integration }
