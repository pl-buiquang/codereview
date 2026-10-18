// Run via the Workflow tool. Wave 5 ONLY — the provider seam over gh.rs (spec 21), SOLO track.
// Pure indirection, ZERO behavior change. After it merges, Wave 6 is the USER-GATED release
// verification (ORCHESTRATION §9) — author no further wave script; hand back to the user.
// Mirrors wave-2/3/4: one Implement track in an isolated worktree, then one Integrate agent in main.
export const meta = {
  name: 'wave-5-implementation',
  description: 'Implement spec 21 (ReviewProvider trait over gh.rs) in a worktree, then integrate into main — zero behavior change',
  phases: [
    { title: 'Implement', detail: '1 worktree track: spec 21 provider abstraction' },
    { title: 'Integrate', detail: 'merge 21, gates + the two strict spec-greps (type-position-only + untouched-files), push, CI watch' },
  ],
}

const REPO = '/Users/pbuiquang/projects/codereview'

const COMMON = `
You are an implementation agent working in an ISOLATED GIT WORKTREE of the repo (your working directory — confirm with \`git rev-parse --show-toplevel\` and use that as your root; it is NOT ${REPO}, never edit files under ${REPO} directly).

Protocol (this is specs/ORCHESTRATION.md §7 — read that section to confirm):
1. First create your branch: \`git checkout -b <branch named below>\`.
2. Run \`pnpm install\` before anything else (node_modules is not shared). The pinned toolchains are already on main (rust-toolchain.toml / .nvmrc).
3. Read your spec file COMPLETELY before writing code, plus the "Discrepancies" section of specs/ORCHESTRATION.md. Implement the spec exactly; where the spec and current code disagree on line anchors OR function names, the CODE wins (the spec was transcribed before the prerequisite specs merged). IMPORTANT: Waves 1–4 are ALL merged on main (specs 04/05/06/07/08/09/10/11/12/13/14/15/16/17/18/19/20). The full GitHub call surface this spec wraps is therefore present AS MERGED.
4. Touch ONLY the files your spec lists ("Files touched" in the spec / "Files claimed" in ORCHESTRATION.md §5 wave 5). If you believe you must touch an unclaimed file, STOP and report it as a deviation instead of editing.
5. Add the tests your spec's Test matrix requires. SKIP the spec's "Manual verify" section (done later) — automated gates only.
6. Gate suite — ALL FIVE must pass in your worktree before you finish (fix failures; do not skip):
   pnpm exec tsc --noEmit
   pnpm build
   pnpm test
   cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
   cargo test --manifest-path src-tauri/Cargo.toml
7. Commit in small steps per the spec's Tasks list, message style \`type(scope): summary\` matching the repo log (refactor(provider), feat(provider), docs(roadmap), …) — single-line header, NO body, NO Co-Authored-By trailer. Each commit leaves the tree green. Do NOT merge, do NOT push, do NOT delete branches.
8. Your final message is data for the orchestrator: report branch, commit shas+titles, gate results, the two spec-grep results (below), and any deviations.
`

const TRACK = {
  label: 'impl:21',
  prompt: `${COMMON}
## Your track: spec 21 (ORCHESTRATION.md wave-5 track 5a) — SOLO
Branch: spec/21-provider-abstraction
Implement specs/21-provider-abstraction.md: factor the GitHub call surface behind a \`ReviewProvider\` trait so a second forge could plug in later. PURE INDIRECTION, ZERO BEHAVIOR CHANGE. Files (the ONLY ones you may touch): NEW src-tauri/src/provider.rs (the object-safe \`ReviewProvider\` trait + \`GithubProvider\` unit struct delegating 1:1 to gh.rs + \`provider_for() -> &'static dyn ReviewProvider\` factory + \`fn name(&self) -> &'static str\`), src-tauri/src/lib.rs (add \`mod provider;\` — alphabetical), src-tauri/src/commands/gh.rs (rewire gh:: CALLS to provider_for().method()), src-tauri/src/commands/review.rs (rewire gh:: CALLS only — no logic change), ROADMAP.md.

MANDATORY TASK 1 — RECONCILE AGAINST MERGED REALITY FIRST (the spec's signatures were transcribed before the prerequisites merged; the CODE wins):
  grep -n "gh::" src-tauri/src/commands/review.rs src-tauri/src/commands/gh.rs
Build the trait to mirror EXACTLY the gh:: functions actually CALLED from commands/ today — one trait method per real call, same types (GhRepo, PrInfo, PrSummary, PrMeta, PrThread, ComparedFile, ReviewComment, post_review's \`payload_json: &str\`). Notes from the merged prerequisites: spec 18's mutation is a single \`set_thread_resolved(thread_id, resolved)\` (NOT separate resolve/unresolve); spec 16 moved the \`gh::compare\` call inside \`reanchor_pass\`; spec 17 added \`gh::review_comments\`; spec 19 added \`gh::submit_pending_review\`/\`gh::delete_pending_review\`; spec 10 added \`gh::merge_base_sha\`. If a function the spec lists does not exist (renamed/dropped), follow the code — do NOT stub it.

HARD CONSTRAINTS (these are also the integrator's gates — make them pass yourself before finishing):
  (A) gh.rs bodies UNCHANGED; \`gh::graphql<T>\` stays OUT of the trait (generic → not object-safe). git.rs, inbox.rs, check_environment's ToolEnv, and ALL frontend (src/) stay untouched.
      Verify: \`git diff main..HEAD --stat -- src-tauri/src/gh.rs src-tauri/src/git.rs src-tauri/src/inbox.rs src/\` MUST be EMPTY.
  (B) After rewiring, the ONLY \`gh::\` occurrences left in commands/ are TYPE references (not function calls or glob imports). Verify EMPTY:
      \`grep -rn "gh::" src-tauri/src/commands/ | grep -vE "gh::(PrInfo|PrSummary|PrMeta|PrThread|ComparedFile|ReviewComment|GhRepo)"\`
      (Tip: reference the shared types as \`gh::PrInfo\` inline, or import them WITHOUT a \`gh::{...}\` glob that the grep would catch — structure imports so this grep is empty.)
  (C) ZERO behavior change: the EXISTING test suite is the regression net and must pass UNMODIFIED. If any existing test seems to need editing, STOP and re-check your indirection — that means you changed behavior. Report it as a deviation rather than editing the test. (A new factory smoke test asserting \`provider_for().name() == "github"\` is allowed/encouraged.)
Report both grep results (A and B) verbatim in your final message.`,
}

phase('Implement')
const result = await agent(TRACK.prompt, {
  label: TRACK.label, phase: 'Implement', isolation: 'worktree', schema: {
    type: 'object',
    properties: {
      branch: { type: 'string' },
      commits: { type: 'array', items: { type: 'string' } },
      gatesPassed: { type: 'boolean' },
      gateDetails: { type: 'string' },
      typePositionGrep: { type: 'string' },
      untouchedFilesGrep: { type: 'string' },
      deviations: { type: 'array', items: { type: 'string' } },
      notes: { type: 'string' },
    },
    required: ['branch', 'commits', 'gatesPassed', 'gateDetails', 'deviations'],
  },
})
log(result && result.gatesPassed ? 'Track 21 finished, gates green' : 'Track 21 finished WITHOUT green gates — integration will report, not merge')

phase('Integrate')
const integration = await agent(
  `You are the wave-5 INTEGRATION agent for ${REPO}. Work directly in ${REPO} (the main checkout, branch main). Read specs/ORCHESTRATION.md §5 wave 5, §6, and §8 first and follow them exactly. Waves 1–4 are already merged on main; the base SHA is the current main tip. This is a SOLO wave — one branch, refactor only.

Track result from the implementation agent (JSON): ${'```'}${JSON.stringify(result)}${'```'}

The spec branch lives in the same repo (worktree branch): spec/21-provider-abstraction.

Steps:
1. \`git -C ${REPO} branch -a\` to confirm the branch exists. If the track reported gatesPassed=false, do NOT merge — report it and stop.
2. Record the current main tip as BASE (\`git rev-parse main\`). Merge with \`git merge --no-ff spec/21-provider-abstraction\`. Conflicts are unlikely (no other track this wave); if any, they are in lib.rs (\`mod provider;\` append — keep alphabetical) / ROADMAP.md (append — keep both).
3. Run the full gate suite (§6: tsc, pnpm build, pnpm test, clippy -D warnings, cargo test). All green. If broken, fix forward with a minimal \`fix(scope): …\` commit (single-line header, no body, no Co-Authored-By) and note it.
4. SPEC-SPECIFIC GATES (ORCHESTRATION §5 wave 5 "Wave-5 verification") — ALL must pass on the merged tree:
   (A) TYPE-POSITION-ONLY grep — MUST be EMPTY:
       \`grep -rn "gh::" src-tauri/src/commands/ | grep -vE "gh::(PrInfo|PrSummary|PrMeta|PrThread|ComparedFile|ReviewComment|GhRepo)"\`
       (If non-empty, every surviving line must be a legitimate type reference the exclusion list missed — inspect each; a function call \`gh::foo(\` surviving means the refactor is incomplete → this is a FAILURE, report it.)
   (B) UNTOUCHED-FILES diff — MUST be EMPTY:
       \`git diff BASE..HEAD --stat -- src-tauri/src/gh.rs src-tauri/src/git.rs src-tauri/src/inbox.rs src/\`
       (gh.rs/git.rs/inbox.rs/all frontend must be byte-for-byte unchanged by this wave.)
   (C) ZERO behavior change: confirm NO existing test file was modified — \`git diff BASE..HEAD --stat\` should show new tests only inside provider.rs (or a small additive smoke test), never edits to pre-existing test fns. If an existing test was changed, STOP and report it as a likely behavior change.
   There is NO key material / NO release.yml change this wave — no key-guard needed.
5. Push to origin main after the merge+gates. Watch CI to green: \`gh run watch "$(gh run list --workflow ci.yml --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status\`. Fix forward, push, re-watch on red. (release.yml is tag-triggered — it will NOT run; expected.)
6. Do NOT delete the branch or worktree (the orchestrator cleans up). Do NOT create tags.
7. Report: merged (yes/no), gate results, the three spec-gate results (A type-position grep verbatim, B untouched-files diff, C no-test-edits) EXPLICITLY, CI conclusion + URL, any fix-forward commits, and anything skipped. Also note that Wave 6 (release verification) is now the only remaining step and is USER-GATED.

Commit messages: default git merge message is fine; fix-forward commits use the repo's terse Conventional-Commit style (single-line header, NO body, NO Co-Authored-By trailer).`,
  { label: 'integrate:wave-5', phase: 'Integrate', schema: {
      type: 'object',
      properties: {
        merged: { type: 'boolean' },
        gateResults: { type: 'string' },
        typePositionGrep: { type: 'string' },
        untouchedFilesDiff: { type: 'string' },
        noTestEdits: { type: 'boolean' },
        ciConclusion: { type: 'string' },
        fixForwardCommits: { type: 'array', items: { type: 'string' } },
        notes: { type: 'string' },
      },
      required: ['merged', 'ciConclusion', 'notes'],
    } }
)

return { track: result, integration }
