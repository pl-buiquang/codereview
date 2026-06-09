# Spec 06 — Release pipeline (GitHub Actions, tag-triggered, draft release)

Implements the **CI pipeline** bullet of ROADMAP §8 (`ROADMAP.md:114-115`): a GitHub Actions
matrix (macOS/Windows/Linux) that builds the app and attaches artifacts to a GitHub Release.
Signing/notarization and auto-update are **not** built here — Spec 07 (updater artifacts) and
Spec 08 (conditional signing env) extend this workflow additively.

## Problem

There is no CI at all:

- No `.github/` directory exists in the repo (verified: `ls .github` → not found).
- The only way to produce a distributable bundle is `pnpm tauri build` on the dev machine — and the
  dev machine is **Linux**, so macOS (`.dmg`/`.app`) and Windows (`.msi`/`-setup.exe`) bundles are
  currently impossible to produce at all.
- `src-tauri/tauri.conf.json:27` already sets `bundle.targets: "all"` and ships a full icon set
  (`src-tauri/icons/` has `.icns`, `.ico`, and the PNG sizes), so the bundler config is
  release-ready; only the automation is missing.
- Remote is `github.com/pl-buiquang/codereview` (public); no tags exist yet.

## Decisions (locked)

- **Trigger:** push of tags matching `app-v*`. Nothing runs on branch pushes (PR/test CI is a
  separate concern, out of scope here).
- **Action:** `tauri-apps/tauri-action@v0` creates/updates a **draft** GitHub Release for the tag
  (`tagName`/`releaseName`/`releaseDraft: true`) and uploads the bundles. The draft is published
  by hand after inspection.
- **Matrix:** four entries — `macos-latest` twice (one per `--target aarch64-apple-darwin` /
  `x86_64-apple-darwin`), `windows-latest`, `ubuntu-22.04`. No universal macOS binary (two
  separate artifacts is simpler and what tauri-action's own examples do).
- **Unsigned artifacts.** No Apple/Windows certs exist. macOS users must clear quarantine
  (`xattr -dr com.apple.quarantine`); say so in the release body. Spec 08 adds the ready-to-enable
  signing env wiring; do not pre-add secret references here beyond the commented placeholders below.
- **Toolchain pins come from Spec 04's files, not hardcoded versions in YAML:** `pnpm/action-setup`
  reads the `packageManager` field in `package.json`, `setup-node` reads `.nvmrc`, and rustup
  honors `rust-toolchain.toml` automatically. This workflow must not duplicate version numbers.
  (My call, one source of truth — if Spec 04 hasn't landed yet, land it first; this spec depends
  on it.)
- **Linux apt deps:** the same list Spec 05 establishes for ubuntu-22.04 Tauri v2 builds
  (`libwebkit2gtk-4.1-dev` et al., full list in the YAML below). Keep the two in sync.
- **Tag/version guard (my call):** a cheap pre-build step asserts that the tag's `X.Y.Z` stem
  (after stripping the `app-v` prefix and any `-rc.N`/pre-release suffix) equals
  `tauri.conf.json`'s `version` (`src-tauri/tauri.conf.json:4`, currently `0.1.0`). Rationale:
  artifact filenames embed the conf version, not the tag — a mismatch ships a confusingly-named
  release; failing fast on all 4 runners costs nothing. The planned first live test,
  `app-v0.1.0-rc.1`, passes this guard (stem `0.1.0` == conf `0.1.0`).
- **`fail-fast: false`** so one platform's failure doesn't cancel the other three (each artifact is
  independently useful).
- **Single job, no separate create-release job:** tauri-action find-or-creates the release for the
  tag, so concurrent matrix jobs converge on one draft. This is the canonical tauri-action layout
  and keeps Spec 07's `includeUpdaterJson` extension a one-block change.

## Design

### Version/tag policy (document this in the YAML header comment too)

- Real release: bump `version` in **both** `src-tauri/tauri.conf.json` and `package.json` to
  `X.Y.Z`, commit to `main`, then `git tag app-vX.Y.Z && git push origin app-vX.Y.Z`.
- Release-candidate test: tag `app-vX.Y.Z-rc.N` against conf version `X.Y.Z`. The draft release is
  named after the **tag**; the artifact filenames carry the **conf version** (`0.1.0`). That's
  accepted for RCs — delete the draft + tag afterwards.
- The guard step enforces stem equality, nothing more.

### File touched

**NEW `.github/workflows/release.yml`** — the only file this spec creates. Full contents:

```yaml
# Release pipeline (Spec 06).
#
# Trigger: push a tag matching app-v* (e.g. app-v0.1.0, app-v0.1.0-rc.1).
# Output:  a DRAFT GitHub Release for the tag with unsigned bundles for
#          macOS (arm64 + x64 .dmg/.app), Windows (.msi + NSIS -setup.exe),
#          Linux (.deb/.rpm/.AppImage). Publish the draft by hand.
#
# Version policy: the tag's X.Y.Z stem (app-v stripped, pre-release suffix
# stripped) MUST equal `version` in src-tauri/tauri.conf.json — the guard
# step below fails the run otherwise. Artifact filenames use the conf
# version; the release name uses the tag.
#
# Extension points (do not restructure when adding these):
#   Spec 07 — updater: add TAURI_SIGNING_PRIVATE_KEY{,_PASSWORD} to the
#             tauri-action `env:` block and `includeUpdaterJson: true` to
#             its `with:` block.
#   Spec 08 — signing: add the conditional Apple/Windows signing env vars
#             to the same `env:` block.
name: release

on:
  push:
    tags:
      - "app-v*"

permissions:
  contents: write # create the draft release + upload assets

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: macos-latest
            rust-target: aarch64-apple-darwin
            args: --target aarch64-apple-darwin
          - platform: macos-latest
            rust-target: x86_64-apple-darwin
            args: --target x86_64-apple-darwin
          - platform: windows-latest
            rust-target: ""
            args: ""
          - platform: ubuntu-22.04
            rust-target: ""
            args: ""
    runs-on: ${{ matrix.platform }}
    steps:
      - uses: actions/checkout@v4

      - name: Check tag matches tauri.conf.json version
        shell: bash
        run: |
          tag="${GITHUB_REF_NAME#app-v}"   # 0.1.0-rc.1
          stem="${tag%%-*}"                # 0.1.0
          conf="$(node -p "require('./src-tauri/tauri.conf.json').version")"
          if [ "$stem" != "$conf" ]; then
            echo "::error::tag stem $stem != tauri.conf.json version $conf"
            exit 1
          fi

      - name: Install Linux build dependencies
        if: matrix.platform == 'ubuntu-22.04'
        run: |
          sudo apt-get update
          # Keep in sync with specs/05-*.md (Tauri v2 prerequisites, webkitgtk 4.1).
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            build-essential \
            curl \
            wget \
            file \
            libxdo-dev \
            libssl-dev \
            libayatana-appindicator3-dev \
            librsvg2-dev

      - uses: pnpm/action-setup@v4 # version comes from package.json "packageManager" (Spec 04)

      - uses: actions/setup-node@v4
        with:
          node-version-file: .nvmrc # pinned by Spec 04
          cache: pnpm

      - name: Install Rust toolchain (honors rust-toolchain.toml from Spec 04)
        shell: bash
        run: rustup show active-toolchain || rustup toolchain install

      - name: Add macOS cross-compile target
        if: matrix.rust-target != ''
        run: rustup target add ${{ matrix.rust-target }}

      - uses: swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
          key: ${{ matrix.rust-target }} # arm64/x64 mac builds must not share a cache

      - name: Install frontend dependencies
        run: pnpm install --frozen-lockfile

      - name: Build and upload to draft release
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          # Spec 07 adds here:
          #   TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          #   TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
          # Spec 08 adds the conditional Apple/Windows signing env here.
        with:
          tagName: ${{ github.ref_name }}
          releaseName: "codereview ${{ github.ref_name }}"
          releaseBody: |
            Unsigned build — see assets below.

            macOS: the app is not notarized; after installing, run
            `xattr -dr com.apple.quarantine /Applications/codereview.app`
            (or right-click → Open). Windows: SmartScreen will warn; choose
            "Run anyway".
          releaseDraft: true
          prerelease: false
          args: ${{ matrix.args }}
```

### Why each piece is the way it is

- **`rustup show active-toolchain || rustup toolchain install`** — installs whatever
  `rust-toolchain.toml` pins (rustup resolves the file by walking up from cwd; the repo checkout
  root works for both the explicit `rustup target add` and tauri-action's cargo invocation inside
  `src-tauri/`). The subsequent `rustup target add` then attaches the Apple target to that *pinned*
  toolchain — using `dtolnay/rust-toolchain@stable` with a `targets:` input instead would add the
  target to `stable`, which silently diverges if the pin is a specific version. (This is the
  "macOS dual-arch needs rust target add" risk flag: without the `rustup target add` step the
  `x86_64-apple-darwin` job fails at cargo build time on the arm64 `macos-latest` runner.)
- **tauri-action invocation:** it auto-detects pnpm + `src-tauri/`, runs
  `beforeBuildCommand` (`pnpm build`, `tauri.conf.json:9`) and `tauri build ${args}` itself —
  do **not** add a separate `pnpm tauri build` step. With `bundle.targets: "all"`
  (`tauri.conf.json:27`) it uploads: `.dmg` + `.app.tar.gz`-less `.app` archive per mac arch,
  `.msi` + `-setup.exe` on Windows, `.deb`/`.rpm`/`.AppImage` on Linux.
- **`swatinem/rust-cache` `key`:** the two macOS jobs run on the same runner image/OS; without a
  per-target key they'd thrash one cache entry.
- **No `concurrency:` block:** tags are pushed once; re-pushing a tag is the operator's explicit
  re-run.

### Data flow

```
git push origin app-v0.1.0-rc.1
        │
        ▼ (4 parallel jobs)
guard tag↔conf version → apt deps (linux only) → pnpm/node/rust setup
        │
        ▼
tauri-action: pnpm build → cargo/tauri build [--target …] → bundles
        │
        ▼
find-or-create DRAFT release "codereview app-v0.1.0-rc.1" on the tag
        └── each job uploads its platform's assets to the same draft
```

## Tasks

1. Confirm Spec 04's pin files exist on `main` (`package.json` `packageManager` field, `.nvmrc`,
   `rust-toolchain.toml`). If not, **stop — implement Spec 04 first.**
2. Add `.github/workflows/release.yml` exactly as above; commit to `main`
   (`ci(release): tag-triggered matrix build via tauri-action`).
3. Lint the workflow (see Gates) and fix any findings.
4. Run the live RC test (Manual verify below); fix-forward any platform-specific build break as
   its own commit.
5. Clean up: delete the RC draft release and the `app-v0.1.0-rc.1` tag (local + remote).

## Test matrix

No Rust or vitest tests — this spec adds only CI YAML, and none of the five gate suites exercise
it. The "tests" are:

| Check | What it asserts |
|---|---|
| `actionlint` on `release.yml` | YAML is a valid Actions workflow; expressions, shells, matrix refs check out |
| Guard-step bash, run locally (see Manual verify step 1) | `app-v0.1.0-rc.1` → stem `0.1.0` matches conf; `app-v0.2.0` → fails against conf `0.1.0` |
| Live `app-v0.1.0-rc.1` run | All 4 jobs green; one draft release containing macOS arm64+x64 `.dmg`, Windows `.msi` + `-setup.exe`, Linux `.deb`/`.rpm`/`.AppImage` |

## Gates

Standard suite (must stay green — trivially, since no app code changes):

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test` (vitest run)
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

Spec-specific:

- `actionlint` over `.github/workflows/release.yml` — e.g.
  `docker run --rm -v "$PWD":/repo -w /repo rhysd/actionlint:latest` (or a downloaded binary).

## Manual verify

1. Dry-run the guard logic locally:
   `GITHUB_REF_NAME=app-v0.1.0-rc.1 bash -c 'tag="${GITHUB_REF_NAME#app-v}"; stem="${tag%%-*}"; conf="$(node -p "require(\"./src-tauri/tauri.conf.json\").version")"; [ "$stem" = "$conf" ] && echo OK || echo MISMATCH'`
   → `OK`. Repeat with `app-v0.9.9` → `MISMATCH`.
2. Push the workflow commit to `main`, then:
   `git tag app-v0.1.0-rc.1 && git push origin app-v0.1.0-rc.1`.
3. `gh run watch` (or `gh run list --workflow=release.yml`) until all 4 matrix jobs finish.
4. `gh release view app-v0.1.0-rc.1` → confirm it is a **draft**, and
   `gh release view app-v0.1.0-rc.1 --json assets -q '.assets[].name'` lists:
   two macOS `.dmg` (one `aarch64`, one `x64` — filenames carry `0.1.0`), a `.msi`, a
   `-setup.exe`, a `.deb`, an `.rpm`, an `.AppImage`.
5. Optionally download the Linux `.AppImage` on the dev machine, `chmod +x`, launch, open a local
   repo review to smoke-test the artifact actually runs.
6. Clean up the test: `gh release delete app-v0.1.0-rc.1 --yes` and
   `git push origin :refs/tags/app-v0.1.0-rc.1 && git tag -d app-v0.1.0-rc.1`.

## Out of scope

- **Updater artifacts / `latest.json`** (`includeUpdaterJson`, minisign keypair, updater plugin) —
  Spec 07. The `env:`/`with:` extension points are already marked in the YAML.
- **Code signing & notarization** (Apple Developer ID, Windows certs, ready-to-enable docs) —
  Spec 08.
- **PR/branch CI** (running `tsc`/`cargo test`/clippy on pushes — the other half of ROADMAP §8's
  CI bullet). Separate workflow, separate spec if wanted.
- **Toolchain pin files themselves** (`rust-toolchain.toml`, `.nvmrc`, `packageManager`) — Spec 04
  owns those; this workflow only consumes them.
- Universal (fat) macOS binaries, Linux arm64 builds, additional bundle formats, release-notes
  generation/changelogs, and publishing the draft automatically.
