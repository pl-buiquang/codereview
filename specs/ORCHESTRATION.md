# ORCHESTRATION — Multi-agent implementation roadmap for specs 04–21

How to land specs 04–21 with parallel implementation agents in isolated git worktrees, merged
into `main` by an integration step in a fixed order, gated by the standard suite at every step.
Specs 00–03 are already shipped. All decisions below were locked with the user; the **Discrepancies**
section at the end records where a spec's own text disagrees with this plan — do not silently fix
those, resolve them as written there.

## 1. Spec index

| Spec | Title | Size | Primary files |
|---|---|---|---|
| 04 | Toolchain pinning | XS | NEW `rust-toolchain.toml`, NEW `.nvmrc`, `package.json`, `ubt.toml` (commit), `CLAUDE.md`, `ROADMAP.md` |
| 05 | CI checks workflow | S | NEW `.github/workflows/ci.yml` |
| 06 | Release pipeline | M | NEW `.github/workflows/release.yml` |
| 07 | Auto-update (updater plugin) | M | `release.yml`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`, `src-tauri/capabilities/default.json`, `.gitignore`, `package.json`, NEW `src/lib/updater.ts`, NEW `src/components/UpdateBanner.tsx`, `src/App.tsx`, `src/styles.css` |
| 08 | Code-signing readiness | M | NEW `docs/signing.md`, `release.yml`, `README.md` |
| 09 | Diff expand to file top/bottom | M | `src/lib/diff.ts` (+test), `src/components/ReviewView.tsx` |
| 10 | `base_sha` for PR targets | M | `src-tauri/src/gh.rs`, `src-tauri/src/commands/review.rs`, `ReviewView.tsx` (one predicate), `ROADMAP.md` |
| 11 | Threaded replies | L | `commands/review.rs`, `src-tauri/src/export.rs`, NEW `src/lib/threads.ts`, `src/lib/api.ts`, `ReviewView.tsx`, `src/components/FileViewPane.tsx`, `src/styles.css` |
| 12 | Resolve/unresolve local threads | M | NEW migration **0007**, `db/mod.rs`, `db/models.rs`, `commands/review.rs`, `export.rs`, `lib.rs`, `types.ts`, `api.ts`, NEW `src/lib/text.ts`, `ReviewView.tsx`, CSS |
| 13 | Suggested changes | M | `src/lib/diff.ts` (+test), `ReviewView.tsx`, `src/components/Markdown.tsx` (+test), `styles.css`, `export.rs` (test only), `ROADMAP.md` |
| 14 | Keyboard navigation | L | NEW `src/lib/keyboard.ts`, NEW `src/components/ShortcutHelp.tsx`, `src/components/FileJumpList.tsx`, `ReviewView.tsx`, `styles.css`, `README.md`, `ROADMAP.md` |
| 15 | Word-level highlighting (markEdits) | S | `src/lib/diff.ts` (+test) only |
| 16 | LEFT-side re-anchoring | L | NEW migration **0008**, `db/mod.rs`, `db/models.rs`, `src-tauri/src/git.rs`, `src-tauri/src/anchor.rs`, `commands/review.rs`, `export.rs` (fixtures), `types.ts`, NEW `src/lib/staleness.ts`, `ReviewView.tsx`, `FileViewPane.tsx`, `ROADMAP.md` |
| 17 | Capture GitHub comment ids | M | `gh.rs`, `commands/review.rs`, `ROADMAP.md` |
| 18 | Reply/resolve GitHub threads | M | `gh.rs`, `src-tauri/src/commands/gh.rs`, `lib.rs`, `api.ts`, `types.ts`, `src/components/GithubThread.tsx` (+test), `ReviewView.tsx` (threadCtx + Composer), `styles.css` |
| 19 | PENDING (draft) GitHub reviews | L | NEW migration **0009**, `db/mod.rs`, `commands/review.rs`, `gh.rs`, `lib.rs`, `types.ts`, `api.ts`, NEW `src/lib/status.ts`, `ReviewView.tsx`, `src/components/RepoView.tsx`, `src/components/ReviewsView.tsx`, `styles.css` |
| 20 | PR-list refresh + polling | XS | `src/lib/settings.ts` (+test), `src/lib/timeAgo.ts` (+test), `RepoView.tsx` (+test), `styles.css` |
| 21 | Provider trait over gh.rs | M | NEW `src-tauri/src/provider.rs`, `lib.rs`, `commands/gh.rs`, `commands/review.rs`, `ROADMAP.md` |

## 2. Hard-dependency DAG

```
04 ──► 05                       (05 consumes 04's pin files; never hard-code versions)

06 ──► 07 ──► 08                (same release.yml region; sequential, ONE worktree)

11 ──► 12                       (same UI components; sequential, ONE worktree; 0007 needs 11's fixtures)

10 ──► 16                       (16's PR-target base pin needs target.base_sha = merge-base)
11 ──► 16, 12 ──► 16            (16 builds on threads + must follow 0007 in MIGRATIONS)
12 ──► 19, 16 ──► 19            (0009 must be the 9th MIGRATIONS entry, after 0007/0008)

17 ─soft─► 18                   (no code dependency; only the GithubThread.tsx surface — 17 first)

10, 16, 17, 18, 19 ──► 21       (21 wraps the gh:: call surface AS MERGED; strictly last)
```

09, 13, 14, 15, 20 have no hard dependencies (placement is driven by file-conflict rules only).

## 3. File-conflict table & rules

Heavy-contention files and which specs touch them:

| File | Specs | Rule |
|---|---|---|
| `src/components/ReviewView.tsx` (heavy) | **09, 11, 12, 13, 14** | **Max one heavy spec per wave.** (Light touches by 10, 16, 18, 19 are allowed alongside; merge-order handles them.) |
| `src-tauri/src/commands/review.rs` (heavy) | **10, 16, 17, 19** | **Max one per wave.** (11/12 also touch it — see merge-order notes below.) |
| `add_comment` path in review.rs | 11, 16 | **Never the same wave; 11 first** (16 extends `add_comment_impl`, which 11 creates). |
| `build_publish_payload` / publish path | 11, 17, 19 | 17 extracts `inline_publish_comments`; 11 (merged after 17 in wave 2) must put `fold_replies` **inside that helper**, not in `build_publish_payload`. 19 (wave 4) wraps the result. |
| `.github/workflows/release.yml` | 06, 07, 08 | Same `env:`/`with:` region — sequential in one worktree, order 06→07→08. |
| `src/lib/diff.ts` + `diff.test.ts` | 09, 13, 15 | 09+15 share wave 1: disjoint functions (15 rewrites `tokenizeFile`, 09 adds helpers) — merge 15 first, 09 rebases. 13 is wave 3. |
| `src/lib/api.ts`, `src/lib/types.ts` | 11, 12, 16, 18, 19 | Trivial append conflicts — resolve at merge by keeping **both** additions. |
| `src-tauri/src/lib.rs` `generate_handler!` / `mod` list | 07, 12, 18, 19, 21 | Trivial append conflicts — keep all registrations, alphabetical mods. |
| `db/mod.rs` `MIGRATIONS` array | 12, 16, 19 | Append-only, **positional** (index = schema version). Order in array must be 0007, 0008, 0009 — guaranteed by wave order. |
| `db/models.rs` `Comment` + struct-literal fixtures | 12, 16 | Each new field forces edits to every `Comment {}` literal (export.rs/review.rs tests). Different waves — fine. |
| `src/styles.css` | 07, 11, 12, 13, 14, 18, 19, 20 | Pure append blocks — keep both sides at merge. |
| `CommentItem` / `Composer` in ReviewView.tsx | 11, 12, 13, 16, 18 | All additive optional props. Wave 3 has 13+16+18 all touching ReviewView lightly→moderately: merge order 18→13→16 and resolve prop additions additively. |
| `src/components/FileViewPane.tsx` | 11, 16 | Different waves — fine. |
| `src/components/RepoView.tsx` | 19, 20 | Different waves (20 wave 1, 19 wave 4) — fine. |
| `package.json` | 04, 07 | Different waves — fine. |
| `ROADMAP.md`, `README.md` | many | Trivial; resolve at merge. |

## 4. Migration-number reservations

`db::migrate` is **positional** — the array index *is* the schema version. Never skip-number, never
reorder, never edit an applied script.

| Number | File | Spec | Wave |
|---|---|---|---|
| 0007 | `0007_comment_resolved.sql` | 12 | 2 |
| 0008 | `0008_comment_anchored_base_sha.sql` | 16 | 3 |
| 0009 | `0009_review_status_pending.sql` (table rebuild — FK-pragma recipe in spec 19) | 19 | 4 |

Verified against the spec texts: each spec declares exactly the number reserved here, and specs
10, 11, 13, 17, 18, 21 explicitly declare **no** migration. Spec 19's task 1 re-checks 0007/0008
exist in `MIGRATIONS` before adding 0009 — satisfied by wave order.

## 5. Wave plan

Notation: `A | B` = parallel tracks (separate worktrees); `[A→B]` = sequential in one worktree.

### Wave 1 — independent foundations

| Track | Spec(s) | Branch | Files claimed |
|---|---|---|---|
| 1a | 04 | `spec/04-toolchain-pinning` | rust-toolchain.toml, .nvmrc, package.json, ubt.toml, CLAUDE.md, ROADMAP.md |
| 1b | 05 | `spec/05-ci-checks` — **branch off `spec/04-…` tip** (hard dep: needs the pin files) | .github/workflows/ci.yml |
| 1c | 09 | `spec/09-diff-expand-edges` | src/lib/diff.ts(+test) [edge-range helpers], ReviewView.tsx (heavy slot) |
| 1d | 10 | `spec/10-pr-target-base-sha` | gh.rs, commands/review.rs (heavy slot), ReviewView.tsx (canExpand predicate only), ROADMAP.md |
| 1e | 15 | `spec/15-word-level-highlight` | src/lib/diff.ts(+test) [tokenizeFile only] |
| 1f | 20 | `spec/20-pr-list-refresh` | settings.ts(+test), timeAgo.ts(+test), RepoView.tsx(+test), styles.css |

**Merge order: 04 → 05 → 15 → 20 → 09 → 10.**
Known intra-wave overlaps: 15 vs 09 in `diff.ts`/`diff.test.ts` (disjoint functions; 09 rebases over
15); 09 vs 10 in `ReviewView.tsx` (10 is a one-predicate edit; 10 merges last and rebases trivially —
note the 09 edge expanders light up on PR targets the moment 10's predicate lands, by design).

**Wave-1 verification:** gates after every merge (§6). **After 05 merges and is pushed, watch the
first CI run to green** before merging anything else:
`gh run watch "$(gh run list --workflow ci.yml --limit 1 --json databaseId -q '.[0].databaseId')"`.
After 09+10: manual smoke — expand-to-top/bottom on a local review; gap expansion on a GitHub PR.

### Wave 2 — threads, publish-id capture, release pipeline

| Track | Spec(s) | Branch | Files claimed |
|---|---|---|---|
| 2a | [11→12] | `spec/11-threaded-replies` then `spec/12-resolve-threads` (same worktree, 12 starts only after 11's gates pass) | commands/review.rs (add_comment/reanchor/publish-fold), export.rs, threads.ts, text.ts, api.ts, types.ts, ReviewView.tsx (heavy slot), FileViewPane.tsx, migration 0007, db/mod.rs, db/models.rs, lib.rs (register `set_comment_resolved`), styles.css |
| 2b | 17 | `spec/17-capture-github-comment-ids` | gh.rs (`ReviewComment`, `review_comments`), commands/review.rs (heavy slot: `inline_publish_comments` extraction + matcher + capture), ROADMAP.md |
| 2c | [06→07→08] | `spec/06-release-pipeline` carried forward (same worktree, sequential) | release.yml, tauri.conf.json, src-tauri/Cargo.toml, lib.rs (plugin registration), capabilities/default.json, .gitignore, package.json, updater.ts, UpdateBanner.tsx, App.tsx, styles.css, docs/signing.md, README.md |

**Merge order: 17 → 11/12 → 06/07/08.**
The 11/12 merge must reconcile with 17: `fold_replies` is applied inside 17's
`inline_publish_comments` helper (the single body-as-posted source of truth), and the
roots-only filter (`parent_id.is_none()`) moves into that helper's filter. This is the one
non-trivial merge of the whole plan — budget for it.

Track 2c notes (locked):
- **Updater keygen happens OUTSIDE the repo** (`~/.tauri/codereview.key`); the private key reaches
  CI **only** via `gh secret set TAURI_SIGNING_PRIVATE_KEY` (+`_PASSWORD` if set). The pubkey goes
  into `tauri.conf.json`. Back up key + password immediately and **ask the user to confirm the
  backup** (spec 07 §1 — lost key = no installed app can ever update again).
- Before **every** wave-2 merge, integration greps the incoming diff for key material:
  `git diff main..<branch> | grep -niE "PRIVATE KEY|untrusted comment|secret key"` → must be empty,
  and `git grep -qi "untrusted comment"` on the merged tree → nothing.

**Wave-2 verification:** gates after every track and merge; `actionlint` on both workflow files;
spec 08's six local staging-step dry-runs; after the final push, `gh run watch` the CI run. Live RC
verification of the release workflow is deferred to wave 6 unless the user opts into per-spec
throwaway RC tags (see Discrepancies #5). Manual smoke: reply/resolve a local thread, export
nesting, publish a scratch PR and check the `[publish.capture_ids]` stderr line.

### Wave 3 — LEFT re-anchoring, suggestions, GitHub thread actions

| Track | Spec(s) | Branch | Files claimed |
|---|---|---|---|
| 3a | 16 | `spec/16-left-side-reanchoring` | migration 0008, db/mod.rs, db/models.rs (+all Comment fixtures), git.rs, anchor.rs (rename `remap_right_line`→`remap_line`), commands/review.rs (heavy slot), export.rs (fixtures), types.ts, staleness.ts, ReviewView.tsx (banner/CommentItem `baseSha`), FileViewPane.tsx, ROADMAP.md |
| 3b | 13 | `spec/13-suggested-changes` | diff.ts(+test), ReviewView.tsx (heavy slot: Composer/CommentItem/LineWidget/FileReview), Markdown.tsx(+test), styles.css, export.rs (one test), ROADMAP.md |
| 3c | 18 | `spec/18-github-thread-replies-resolve` | gh.rs, commands/gh.rs, lib.rs, api.ts, types.ts, GithubThread.tsx(+test), ReviewView.tsx (threadCtx plumbing, Composer.submitLabel — see Discrepancies #3), styles.css |

**Merge order: 18 → 13 → 16.**
All three touch `ReviewView.tsx`; the heavy spec (13) merges mid-order, and 16's additive
`baseSha` prop on `CommentItem` merges last over 13's additive `suggestionSeed` prop — keep both.
16 also rebases its `is_anchored_to` call-site change onto 17's `inline_publish_comments` (the
filter now lives there, not in `build_publish_payload` — see Discrepancies #4).

**Wave-3 verification:** gates per merge; spec-specific: migration 0008 applies on a 0007 DB;
`grep -rn "remap_right_line" src-tauri/src` empty after 16; manual smoke of LEFT re-anchor
(local three-dot), suggestion fence end-to-end, GitHub thread reply/resolve on a scratch PR.

### Wave 4 — pending reviews, keyboard nav

| Track | Spec(s) | Branch | Files claimed |
|---|---|---|---|
| 4a | 19 | `spec/19-pending-reviews` | migration 0009, db/mod.rs, commands/review.rs (heavy slot: ensure_draft/prepare_publish/3 commands), gh.rs, lib.rs, types.ts, api.ts, status.ts, ReviewView.tsx (header buttons + readOnly), RepoView.tsx, ReviewsView.tsx, styles.css |
| 4b | 14 | `spec/14-keyboard-nav` | keyboard.ts(+test), ShortcutHelp.tsx(+test), FileJumpList.tsx, ReviewView.tsx (heavy slot: keydown effect, FileReview/FileBody plumbing), styles.css, README.md, ROADMAP.md |

**Merge order: 19 → 14.** Both touch `ReviewView.tsx` in mostly disjoint regions (19: header;
14: top-level effect + diff plumbing); 14 rebases over 19.

**Wave-4 verification:** gates per merge; migration 0009 applies on a 0008 DB and
`PRAGMA foreign_key_check` is empty; manual: pending publish→submit / →discard round-trip on a
scratch PR; keyboard matrix (`?`, `]`/`[`, `n`/`p`, `j`/`k`, `c`, Esc) incl. published-review
read-only behaviour.

### Wave 5 — provider seam (solo)

| Track | Spec | Branch | Files claimed |
|---|---|---|---|
| 5a | 21 | `spec/21-provider-abstraction` | NEW provider.rs, lib.rs (`mod provider;`), commands/gh.rs, commands/review.rs (call rewiring only), ROADMAP.md |

Task 1 of the spec is mandatory: `grep -n "gh::" src-tauri/src/commands/*.rs` on the **merged**
tree and reconcile the trait against reality (the merged code wins over the spec's transcription).
Zero behavior change — if any existing test needs editing, stop and re-check.

**Wave-5 verification:** gates; the type-position-only grep
(`grep -rn "gh::" src-tauri/src/commands/ | grep -vE "gh::(PrInfo|PrSummary|PrMeta|PrThread|ComparedFile|ReviewComment|GhRepo)"`
→ empty); `git diff --stat src-tauri/src/gh.rs src-tauri/src/git.rs src-tauri/src/inbox.rs src/`
→ empty; smoke pass over one call per trait-method group.

### Wave 6 — release verification (USER-GATED)

No spec; the runbook is §9. Nothing here is automated past the gates — the user decides the final
`app-v0.1.0` tag.

## 6. Standard gate suite

Run **after every track completes** (in its worktree) and **after every merge into `main`** (in the
main checkout), in this order:

```bash
pnpm exec tsc --noEmit
pnpm build
pnpm test                                                        # vitest run
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test   --manifest-path src-tauri/Cargo.toml
```

All five must pass — including the cargo gates on frontend-only specs (they prove no backend
drift). Spec-specific extra gates are listed per wave above and in each spec's Gates section.

## 7. Worktree protocol (implementation agents)

1. **One worktree per track**: `git worktree add ../codereview-spec-NN spec/NN-<short-name> main`
   (sequential tracks like `[11→12]` reuse the worktree and start the next spec's commits on the
   same branch chain only after the previous spec's gates pass; rename or branch
   `spec/12-resolve-threads` off `spec/11-…` tip for clean history). Track 1b branches off
   `spec/04-toolchain-pinning`'s tip, not `main`.
2. **Branch naming:** `spec/NN-name` (e.g. `spec/09-diff-expand-edges`).
3. **`pnpm install` first** in every fresh worktree (`node_modules` is not shared). Respect the
   pinned toolchains once 04 lands (`rustup` picks up `rust-toolchain.toml` automatically).
4. **No shared `CARGO_TARGET_DIR`.** Each worktree builds into its own
   `src-tauri/target` — do not export a common target dir; parallel cargo builds must not
   contend on one lock.
5. **Touch only the files your spec lists** (the "Files claimed" column above + your spec's
   "Files touched"). If you believe you must touch an unclaimed file, stop and surface it instead
   of editing.
6. **Commit message format** matches the existing log style — `type(scope): summary`, types seen
   in this repo: `feat(review)`, `fix(review)`, `docs(roadmap)`, `docs(specs)`, `ci(release)`,
   `ci(checks)`, `docs(signing)`. One spec may span several commits (specs mark independently
   buildable steps); each commit must leave the worktree green.
7. Run the full gate suite (§6) in the worktree before declaring the track done. Do **not** merge
   or push — that is the integration step's job.
8. Wave-2 track 2c additionally: never write key material into the worktree; run the spec 07
   key-guard greps before every commit; `*.key` goes in `.gitignore`.

## 8. Integration protocol (merge step)

For each wave, after all tracks report green:

1. Merge branches into `main` **in the wave's stated merge order**, one at a time
   (`git merge --no-ff spec/NN-name` from the main checkout).
2. **Append-conflict resolution guidance** — conflicts in these files are expected and resolved by
   keeping *both* sides' additions:
   - `src/lib/api.ts` / `src/lib/types.ts`: keep both new wrappers/fields; order cosmetic.
   - `src-tauri/src/lib.rs`: keep all `generate_handler!` registrations and all `mod` lines
     (mods alphabetical).
   - `db/mod.rs` `MIGRATIONS`: keep all `include_str!` entries **in numeric order** — the array
     is positional; 0007 < 0008 < 0009, never reorder or gap.
   - `src/styles.css`, `ROADMAP.md`, `README.md`: keep both blocks.
   - Anything beyond an append (notably wave 2's `inline_publish_comments` ⇄ `fold_replies`
     reconciliation, wave 3's `CommentItem` props, wave 4's ReviewView regions): resolve per the
     wave notes in §5, then re-read the affected function whole to confirm coherence.
3. **Re-run the full gate suite after each individual merge** before merging the next branch.
4. **Wave 2 only:** before each merge, the PRIVATE-KEY grep
   (`git diff main..<branch> | grep -niE "PRIVATE KEY|untrusted comment|secret key"` → empty);
   after all merges, `git grep -qi "untrusted comment"` → nothing.
5. Push directly to `main` (repo convention — no PRs). **After every push, watch CI:**
   `gh run watch "$(gh run list --workflow ci.yml --limit 1 --json databaseId -q '.[0].databaseId')"`
   (from wave 1's 05-merge onward). A red run is fixed forward immediately before starting the
   next wave.
6. Clean up merged worktrees/branches (`git worktree remove …`, `git branch -d spec/NN-…`).

## 9. Wave 6 — release-verification runbook (user-gated)

Preconditions: waves 1–5 merged and green; updater key generated (wave 2) and **backup confirmed
by the user** — re-confirm now: `~/.tauri/codereview.key`, `.key.pub`, and the password are in a
password manager / offline backup. If the private key is lost, no installed app can ever accept
another update; there is no rotation mechanism.

1. **Local signed build sanity:**
   ```bash
   TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/codereview.key)" \
   TAURI_SIGNING_PRIVATE_KEY_PASSWORD='<password if set>' \
   pnpm tauri build
   ```
   → succeeds; `src-tauri/target/release/bundle/` contains the AppImage **plus** a matching
   `.sig`. Also confirm `pnpm tauri build` *without* the env vars fails (proves
   `createUpdaterArtifacts` is live). Launch the AppImage, open a local review, smoke-test.
2. **Tag the RC:** confirm `tauri.conf.json` version is `0.1.0` (the guard step enforces stem
   equality), then `git tag app-v0.1.0-rc.1 && git push origin app-v0.1.0-rc.1`.
3. **Watch the release workflow:** `gh run watch` (workflow `release.yml`) until all 4 matrix jobs
   (macOS arm64, macOS x64, Windows, Linux) are green. In the logs: the version guard passed, the
   "Stage code-signing secrets" step staged nothing (no certs set), no `Signing`/`codesign` lines.
4. **Verify the draft release assets:**
   `gh release view app-v0.1.0-rc.1 --json assets -q '.assets[].name'` → two macOS `.dmg`
   (+`.app.tar.gz`), `.msi` + `-setup.exe`, `.deb`/`.rpm`/`.AppImage`, **per-platform `.sig`
   files**, and one `latest.json`. Publish the draft **as a prerelease**, then
   `curl -L https://github.com/pl-buiquang/codereview/releases/download/app-v0.1.0-rc.1/latest.json`
   → correct `version` and a `platforms` map with non-empty `signature` per OS.
5. **Prerelease invisibility check (expected behaviour, not a bug):**
   `https://github.com/pl-buiquang/codereview/releases/latest/download/latest.json` must **not**
   serve the rc — `releases/latest/download/…` resolves only published, non-prerelease releases,
   so installed apps cannot see drafts or `-rc.N` prereleases. That is the safety valve.
6. **User decision:** the user inspects the rc artifacts (and optionally installs the AppImage to
   test the update banner against a later release). If satisfied, the user green-lights
   `app-v0.1.0`: publish flow per spec 06 — versions already at 0.1.0, tag, push, watch, publish
   the draft as a **full release** (this one *is* visible to the updater endpoint). If not,
   fix forward and cut `app-v0.1.0-rc.2`.
7. Cleanup of rc artifacts (after the decision):
   `gh release delete app-v0.1.0-rc.1 --yes && git push origin :refs/tags/app-v0.1.0-rc.1 && git tag -d app-v0.1.0-rc.1`.

## Discrepancies (spec text vs. this plan — recorded, not silently fixed)

1. **Spec 21's prerequisite list omits 16.** Spec 21's header says "after specs 10, 17, 18 and 19
   have merged"; the locked plan gates 21 on {10, 16, 17, 18, 19}. Not a contradiction in effect
   (16 merges in wave 3, before 21 regardless), and 16 moves `gh::compare` call sites inside
   `reanchor_pass`, which 21's reconciliation grep must see — the plan's stricter gate is correct.
2. **Spec 11 vs spec 17 both restructure the publish body path in the same wave.** Spec 11 (written
   pre-17) says to fold replies in `build_publish_payload`/`body_with_file_comments` directly;
   spec 17 (merged first) extracts `inline_publish_comments` and mandates transforms live there.
   Resolution (encoded in wave 2): 17 merges first; 11/12's merge moves `fold_replies` and the
   `parent_id.is_none()` filter into `inline_publish_comments`. Spec 17 anticipated this; spec 11's
   line anchors do not.
3. **Spec 11 and spec 18 both add `Composer.submitLabel`.** Spec 18 describes it as a new prop, but
   spec 11 (wave 2) already adds it. By wave 3 the prop exists; 18's implementer must detect this
   and skip the duplicate addition rather than re-introduce it.
4. **Spec 16's `is_anchored_to` call-site anchors predate spec 17.** Spec 16 lists prerequisites
   "after specs 10, 11 and 12" and cites `build_publish_payload` (`review.rs:722`) as a call site —
   but 17 (wave 2, before 16) moves that filter into `inline_publish_comments`. Mechanical: 16
   updates the call inside the helper instead. Spec 16's prerequisite list should have included 17.
5. **Specs 06/07/08 each prescribe their own live throwaway RC runs** (06: `app-v0.1.0-rc.1`
   create+delete; 07: a `0.1.1-rc.1` asset inspection; 08: `app-v0.1.0-rc.2`), while the locked
   plan concentrates live release verification in user-gated wave 6 (which reuses the
   `app-v0.1.0-rc.1` tag name — fine, since any wave-2 RC tags would be deleted per the specs).
   Default in this plan: **defer live RC runs to wave 6**; the integration step may run the
   wave-2 RCs early if the user prefers catching platform-specific build breaks sooner
   (recommended by the specs, cheaper to fix-forward). Needs a user call — see open questions.
6. **Wave-3/4 ReviewView co-tenancy is looser than the headline rule.** The "max one ReviewView-heavy
   spec per wave" rule covers {09, 11, 12, 13, 14}, but specs 16, 18 (wave 3) and 19 (wave 4) also
   edit `ReviewView.tsx` (banner/props, threadCtx, header buttons). Accepted: the touches are
   additive/regional and the merge orders (18→13→16, 19→14) sequence them; not a rule violation,
   but integration should expect non-append conflicts in `ReviewView.tsx` in waves 3 and 4.
7. **Wave-1 `diff.ts` contention is not covered by the trivial-append list.** Specs 09 and 15 both
   edit `src/lib/diff.ts` and `diff.test.ts` in wave 1. The functions are disjoint
   (15: `tokenizeFile`/imports; 09: new exported helpers), so the 15→…→09 merge order resolves it,
   but it is a real (if easy) merge, unlike api.ts/types.ts appends.
8. **Spec 07's keygen/backup is a user-interaction point inside wave 2.** The plan's waves are
   otherwise agent-autonomous; track 2c blocks mid-wave on the user confirming the key backup and
   on `gh secret set` (operator credentials). Schedule track 2c's keygen step early in the wave so
   the wait doesn't serialize the whole wave.
