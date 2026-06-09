# Spec 05 — CI checks workflow (GitHub Actions)

Implements the "run `tsc`, `cargo test`, lints" half of ROADMAP §8 (`ROADMAP.md:114-115`,
"**CI pipeline** — GitHub Actions … run `tsc`, `cargo test`, lints on PRs"). The build-matrix /
release-artifact half of §8 is a separate spec (release workflow on `app-v*` tags) and is **not**
covered here. Depends on Spec 04 (toolchain pins).

## Problem

There is no CI at all: the repo has **no `.github/` directory** (verified 2026-06-09), so nothing
runs on push. The only automated checks are the ones CLAUDE.md tells a developer to run locally
(`pnpm exec tsc --noEmit`, `pnpm build`, `pnpm test`, `cargo clippy`, `cargo test`). Since this is
a solo repo with direct commits to `main` and no review gate, a forgotten local check ships
silently broken code. The gate suite needs to run automatically on every push.

Verified inputs:

- `package.json` scripts: `"build": "tsc && vite build"`, `"test": "vitest run"` — vitest is
  configured (`vitest.config.ts`) and real tests exist (`src/store.test.ts`,
  `src/lib/diff.test.ts`, `src/components/Markdown.test.tsx`, …).
- `pnpm-lock.yaml` and `src-tauri/Cargo.lock` are committed → `--frozen-lockfile` and locked cargo
  resolution both work.
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` **passes clean locally as of
  2026-06-09** (re-verify before landing the workflow — see Tasks step 1).
- The `tauri` crate (v2, `src-tauri/Cargo.toml:24`) links GTK/WebKitGTK on Linux, so even
  `cargo clippy`/`cargo test` on an Ubuntu runner fail at build time unless the system dev
  packages are installed first.

## Decisions (locked)

- **Triggers:** `push` to `main` + `pull_request`. Direct-to-main is the normal flow here
  (CLAUDE.md), but `pull_request` is kept so occasional branches/external PRs get checked too.
- **Concurrency:** one group per ref, `cancel-in-progress: true` — superseded runs are cancelled.
  These are pure checks (no deploy), so cancelling an in-flight `main` run on a newer push is safe.
- **Two jobs, both on `ubuntu-latest`** — `frontend` and `rust` — running the exact gate-suite
  commands verbatim (`--manifest-path src-tauri/Cargo.toml`, not `working-directory`), so CI and
  local invocations are character-identical. No macOS/Windows matrix here; cross-platform builds
  belong to the release workflow spec.
- **Pins come from Spec 04, not from this workflow.** `pnpm/action-setup@v4` is used with **no
  `version` input** (it reads the `packageManager` field Spec 04 adds to `package.json`);
  `actions/setup-node@v4` reads `node-version-file: .nvmrc`; the Rust toolchain comes from
  `rust-toolchain.toml`. This workflow must not hard-code any tool version.
- **`rust-toolchain.toml` must live at the repo root.** rustup resolves the toolchain file from
  the *current working directory*, not from `--manifest-path`; since the cargo commands run from
  the repo root, a pin under `src-tauri/` would be silently ignored. (Cross-spec contract with
  Spec 04 — verify before implementing.)
- **Rust toolchain install step:** plain `rustup toolchain install` (no args; reads
  `rust-toolchain.toml`, supported since rustup 1.28) instead of a third-party toolchain action —
  one less pinned dependency, and GitHub runners ship rustup preinstalled.
- **Caching:** `Swatinem/rust-cache@v2` (with `workspaces: src-tauri -> target`) for Rust;
  setup-node's built-in `cache: pnpm` for the pnpm store. No manual `actions/cache` blocks.
- **Action pinning:** major-version tags (`@v4`, `@v2`), not commit SHAs — solo repo,
  low-maintenance over supply-chain paranoia.
- **Keep the redundant `tsc --noEmit` step** even though `pnpm build` runs `tsc` again: the
  dedicated step fails fast with a clearly-named check, and it keeps 1:1 parity with the local
  gate suite.
- **Least privilege:** `permissions: contents: read` at workflow level.

## Design

### File touched (the only one)

**NEW** `.github/workflows/ci.yml` — full contents:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read

jobs:
  frontend:
    name: frontend (tsc, vitest, build)
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4

      # No `version` input: pnpm/action-setup reads the "packageManager"
      # field from package.json (added by Spec 04).
      - uses: pnpm/action-setup@v4

      - uses: actions/setup-node@v4
        with:
          node-version-file: .nvmrc
          cache: pnpm

      - run: pnpm install --frozen-lockfile
      - run: pnpm exec tsc --noEmit
      - run: pnpm test
      - run: pnpm build

  rust:
    name: rust (clippy, test)
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4

      # Tauri v2 Linux build prerequisites. Without these, the `tauri` crate's
      # sys dependencies (gtk, webkit2gtk-4.1, rsvg, appindicator) fail to
      # compile, so even clippy/test cannot run.
      - name: Install Tauri Linux build dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            libwebkit2gtk-4.1-dev \
            libgtk-3-dev \
            librsvg2-dev \
            libayatana-appindicator3-dev \
            libssl-dev \
            libxdo-dev \
            patchelf \
            build-essential \
            curl \
            wget \
            file

      # Installs the toolchain pinned in rust-toolchain.toml (Spec 04).
      - name: Install pinned Rust toolchain
        run: rustup toolchain install

      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri -> target

      - run: cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
      - run: cargo test --manifest-path src-tauri/Cargo.toml
```

Notes for the implementer:

- The apt list is the official Tauri v2 Ubuntu prerequisite set (`libwebkit2gtk-4.1-dev`,
  `build-essential`, `curl`, `wget`, `file`, `libxdo-dev`, `libssl-dev`,
  `libayatana-appindicator3-dev`, `librsvg2-dev`) plus `libgtk-3-dev` (explicit, though webkit
  pulls it in) and `patchelf` (only needed for bundling — harmless here and keeps the step
  copy-pastable into the release workflow). `libwebkit2gtk-4.1-dev` exists on both ubuntu-22.04
  and ubuntu-24.04, so `ubuntu-latest` migrations won't break it.
- The two jobs are independent (no `needs:`) — they run in parallel.
- `cargo test` reuses the build artifacts clippy produced where possible; `rust-cache` keys off
  `Cargo.lock` + `rust-toolchain.toml`, so the first run is slow (~full tauri build) and
  subsequent runs are incremental.

### Data flow

push/PR → workflow dispatch → both jobs in parallel → each job is exactly the local gate suite,
split frontend/backend. A red ✗ on the commit in `gh run list` / the GitHub UI is the only output;
there are no artifacts.

## Tasks

1. **Re-verify the gate suite locally** (clippy was clean on 2026-06-09, but re-check at
   implementation time): run all five gate commands. If `cargo clippy … -- -D warnings` reports
   lints, fix the trivial ones in a separate commit *before* adding the workflow — never land CI
   that is red on arrival.
2. **Verify Spec 04 pins landed**: `package.json` has a `packageManager: "pnpm@…"` field,
   `.nvmrc` exists, `rust-toolchain.toml` exists **at the repo root**. If any is missing, this
   spec is blocked — do not substitute hard-coded versions in the workflow.
3. Add `.github/workflows/ci.yml` exactly as above. If `actionlint` is available locally
   (binary or `docker run --rm -v "$PWD":/repo rhysd/actionlint`), lint the file; otherwise skip.
4. Commit directly to `main` (solo-repo convention) and push; watch the first run to green
   (see Manual verify). Fix forward in follow-up commits if needed.

## Test matrix

No Rust or vitest tests are added — this spec ships only YAML. The "tests" are the workflow runs
themselves:

| Check | Asserts |
|---|---|
| `actionlint` (local, optional) | YAML is well-formed, action inputs valid, shell steps lint clean |
| First `ci.yml` run on `main` | both jobs green; `frontend` resolves pnpm from `packageManager` and node from `.nvmrc`; `rust` compiles the tauri crate (apt deps sufficient) |
| Second run (any later push) | `Swatinem/rust-cache` restores (`rust` job markedly faster); setup-node reports a pnpm cache hit |
| Two pushes in quick succession | first run is cancelled by the concurrency group |

## Gates

The standard 5-gate suite must pass locally before the commit that adds the workflow (the
workflow *is* these gates; it must be born green):

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test`
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

Spec-specific: the first GitHub Actions run of `ci.yml` on `main` completes with both jobs green.

## Manual verify

1. Push the commit to `main`, then:
   ```bash
   gh run watch "$(gh run list --workflow ci.yml --limit 1 --json databaseId -q '.[0].databaseId')"
   ```
   Both `frontend (tsc, vitest, build)` and `rust (clippy, test)` finish green.
2. Open the `rust` job log: confirm the apt step installed `libwebkit2gtk-4.1-dev`, the toolchain
   step printed the version pinned in `rust-toolchain.toml`, and clippy/test ran with
   `-D warnings`.
3. Open the `frontend` job log: confirm pnpm's version matches `packageManager` and node's
   matches `.nvmrc`; vitest reports the existing suites (e.g. `src/lib/diff.test.ts`) passing.
4. Push any trivial follow-up commit; confirm the `rust` job's compile phase is much faster
   (cache restored) and, if pushed while run 1 was still in flight, that run 1 shows "cancelled".

## Out of scope

- The release/build-matrix workflow (macOS/Windows/Linux bundles, `app-v*` tags,
  `tauri-apps/tauri-action`, updater signing) — separate spec.
- Code signing / notarization (locked decision: unsigned builds, docs-only signing readiness).
- Branch protection or required-status-check settings (solo repo, direct pushes to `main`).
- Coverage upload (`pnpm test:coverage` exists but stays local-only).
- Caching `node_modules` directly, `cargo build` release profiles, or any build-artifact upload.
- Scheduled runs, `workflow_dispatch`, dependabot, or commit-SHA action pinning.
