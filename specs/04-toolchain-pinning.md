# Spec 04 — Reproducible builds: toolchain pinning

Implements the ROADMAP §8 bullet *"Reproducible builds — `Cargo.lock` and `pnpm-lock.yaml` are
committed; pin toolchains (`rust-toolchain.toml`, `.nvmrc`)"* (`ROADMAP.md:116-117`). Spec 05 (CI)
depends on the pin files written here.

## Problem

Dependency versions are locked (`Cargo.lock` and `pnpm-lock.yaml` are committed) but the
*toolchains* are not, so two machines — or local dev vs. the upcoming CI (Spec 05) — can build with
different `rustc`/`node`/`pnpm` versions and produce different results or spurious failures:

- No `rust-toolchain.toml` at the repo root — rustup uses whatever default the machine has.
- No `.nvmrc` — node version is whatever nvm/the OS provides (locally: v23.7.0 via nvm).
- No `"packageManager"` field in `package.json` (`package.json:1-43`) — corepack cannot enforce
  the pnpm version (locally: 10.28.2).
- `ubt.toml` (the Universal Build Tool mapping for this repo) exists at the root but is
  **untracked** (`git status` shows `?? ubt.toml`), so the universal-verb mappings are lost on a
  fresh clone.

Additionally, `CLAUDE.md` is stale about testing and misleads every agent that reads it:

- `CLAUDE.md:24` — `cargo test … # NOTE: no tests exist yet (see ROADMAP §9)`. False: Rust
  `#[cfg(test)]` modules exist in 11 files (e.g. `src-tauri/src/anchor.rs`,
  `src-tauri/src/commands/review.rs`, `src-tauri/src/gh.rs`).
- `CLAUDE.md:27-28` — *"no test suite in either layer — `tsc` and `cargo clippy`/`cargo build` are
  the only automated checks today"*. False: vitest is configured (`vitest.config.ts`,
  `package.json` scripts `test`/`test:watch`/`test:coverage`) with tests in `src/lib/*.test.ts`,
  `src/components/*.test.tsx`, `src/store.test.ts`.

Locally verified versions (2026-06-09): `rustc 1.93.1`, `cargo 1.93.1`, `node v23.7.0`,
`pnpm 10.28.2`.

## Decisions (locked)

- **Rust channel `"1.93"`, not `"1.93.1"`** — rustup resolves a `major.minor` channel to the latest
  patch release of that minor, so the locally installed 1.93.1 satisfies it and future patch fixes
  arrive without a repo edit. Matches the "pin that channel" instruction.
- **`.nvmrc` contains the major only: `23`** — matches the installed node (v23.7.0, the only thing
  guaranteed to keep local dev working); major-only lets nvm and `actions/setup-node`
  (`node-version-file: .nvmrc`, Spec 05) pick up patch releases. *Not* switched to an LTS major —
  changing node out from under a working dev setup is exactly what pinning is meant to prevent.
- **`"packageManager": "pnpm@10.28.2"`** — exact version (corepack requires `name@x.y.z`), matching
  the installed pnpm. No integrity hash (optional, and it would force a repo edit on every pnpm
  patch bump).
- **`components = ["clippy"]` in `rust-toolchain.toml`** — the gate suite runs clippy; declaring it
  makes rustup install it even under CI's `profile = minimal`.
- **Commit `ubt.toml` verbatim** — it already maps the verbs correctly; no content changes.
- **CLAUDE.md fix is part of this spec** — same "make the repo's self-description match reality"
  theme; one commit, no behavior change.
- Direct commits to `main`, no PR (repo convention).

## Design

Four new/changed files at the repo root plus two doc edits. No code, no schema, no UI.

### 1. NEW `/home/paul/projects/codereview/rust-toolchain.toml`

Exact contents:

```toml
[toolchain]
channel = "1.93"
components = ["clippy"]
```

rustup reads this automatically for every `cargo`/`rustc` invocation at or below the repo root —
both local dev and CI runners honor it with zero extra wiring (Spec 05's jobs just run `cargo`).
Note `src-tauri/` is *below* the root, so `cargo --manifest-path src-tauri/Cargo.toml` invoked from
the root picks it up.

### 2. NEW `/home/paul/projects/codereview/.nvmrc`

Exact contents (single line, trailing newline):

```
23
```

Consumed by `nvm use` locally and `actions/setup-node` with `node-version-file: ".nvmrc"` in
Spec 05.

### 3. `package.json` — add `packageManager`

One field, after `"type": "module"` (`package.json:5`):

```json
  "type": "module",
  "packageManager": "pnpm@10.28.2",
  "scripts": {
```

Consumed by corepack locally (if enabled) and by `actions/setup-node`'s pnpm caching / corepack in
Spec 05. pnpm itself warns-or-errors on mismatch depending on config; with the exact installed
version pinned, local dev is unaffected.

### 4. Commit `ubt.toml`

`git add ubt.toml` — content unchanged (the file already at the repo root, starting
`# ubt.toml — Universal Build Tool configuration`). Verify before committing that its commands
still match reality; they do as of today (`build`/`start`/`check`/`lint`/`clean` +
`web-build`/`web`/`test-rs` aliases all reference existing scripts).

### 5. `CLAUDE.md` — Commands section truth-fix

Replace the code block at `CLAUDE.md:17-25` with:

```bash
pnpm install
pnpm tauri dev                                   # launch the app with hot-reload (Vite + Rust)
pnpm tauri build                                 # release binary + macOS .app/.dmg under src-tauri/target/release/
pnpm build                                       # frontend only: tsc && vite build (fast TS-error check)
pnpm exec tsc --noEmit                           # typecheck the frontend without emitting
pnpm test                                        # frontend unit tests (vitest run; also test:watch, test:coverage)
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo test   --manifest-path src-tauri/Cargo.toml [test_name]   # Rust unit/integration tests
```

Replace the prose paragraph at `CLAUDE.md:27-29`:

> There is **no JS linter/formatter configured** and **no test suite** in either layer — `tsc` and
> `cargo clippy`/`cargo build` are the only automated checks today. The GitHub features require `gh`
> authenticated (`gh auth login`); local virtual-PR review needs only `git`.

with:

> There is **no JS linter/formatter configured**; the automated checks are `tsc` and vitest on the
> frontend, `cargo clippy` and `cargo test` on the backend. Toolchains are pinned —
> `rust-toolchain.toml` (rustup picks it up automatically), `.nvmrc`, and the `packageManager`
> field in `package.json`. The GitHub features require `gh` authenticated (`gh auth login`); local
> virtual-PR review needs only `git`.

Also update the sentence at `CLAUDE.md:15` from
*"The Rust backend needs a stable Rust toolchain on `PATH`."* to
*"The Rust backend needs rustup on `PATH` (the toolchain version comes from `rust-toolchain.toml`)."*

### 6. `ROADMAP.md` — trim the shipped bullet

In §8 (`ROADMAP.md:110-117`), delete only the *Reproducible builds* bullet (lines 116-117). Leave
the signing / auto-update / CI-pipeline bullets — those belong to Spec 05. Also fix the stale
cross-reference left in §7's framing if the implementer notices `CLAUDE.md:24` pointed at
"ROADMAP §9" for tests — the correct section for tests is §7; the CLAUDE.md replacement above
already drops that pointer entirely.

## Tasks

Ordered; each step leaves the repo buildable. Plausibly one commit total, but split if preferred:

1. [ ] Add `rust-toolchain.toml` (contents above). Run `rustup show` in the repo to confirm the
       active toolchain resolves to 1.93.x (rustup may download the channel on first use).
2. [ ] Add `.nvmrc` (contents above). Confirm `node --version` major matches (23).
3. [ ] Add `"packageManager": "pnpm@10.28.2"` to `package.json`; run `pnpm install` to confirm
       pnpm accepts the field and the lockfile is unchanged (`git diff --exit-code pnpm-lock.yaml`).
4. [ ] `git add ubt.toml`.
5. [ ] Apply the `CLAUDE.md` edits (§5 above).
6. [ ] Trim the ROADMAP §8 reproducible-builds bullet (§6 above).
7. [ ] Run the full gate suite, commit directly to `main`.

## Test matrix

No new Rust or vitest tests — this spec adds configuration and docs only; the existing suites *are*
the verification that the pinned toolchains build the project. Do not add tests that parse these
config files.

## Gates

Standard 5-gate suite — run **after** all pin files are in place so they execute under the pinned
toolchains:

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test` (vitest run)
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

Spec-specific checks:

- `rustup show active-toolchain` (run at the repo root) reports `1.93.x` and mentions the override
  from `rust-toolchain.toml`.
- `pnpm install --frozen-lockfile` succeeds and `git status --short` shows no lockfile drift.
- `git status --short` shows no remaining `?? ubt.toml`.

## Manual verify

1. `cd /home/paul/projects/codereview && rustup show` → active toolchain `1.93.x …
   (overridden by '/home/paul/projects/codereview/rust-toolchain.toml')`, with `clippy` listed.
2. `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` from the repo root → builds
   with the pinned toolchain, no errors.
3. `bash -lc 'nvm use'` (or open a fresh shell in the repo with nvm auto-use) → switches to a
   Node 23 install without error.
4. `pnpm install` → no `packageManager` mismatch error; `pnpm tauri dev` still launches the app.
5. `cat CLAUDE.md` → Commands section lists `pnpm test`, no "no tests exist yet" note, prose
   mentions vitest + cargo test and the pin files.
6. Fresh-clone smoke (optional but cheap): `git clone . /tmp/codereview-clone && ls
   /tmp/codereview-clone/ubt.toml /tmp/codereview-clone/rust-toolchain.toml
   /tmp/codereview-clone/.nvmrc` → all present.

## Out of scope

- The CI workflow itself (`.github/workflows/*`) — Spec 05 consumes these pins
  (`node-version-file: .nvmrc`; rustup honors `rust-toolchain.toml` natively).
- Adding `rust-version` to `src-tauri/Cargo.toml` (MSRV declaration) — different mechanism,
  different purpose; not needed for reproducibility here.
- Corepack enablement docs / forcing corepack on contributors — solo repo, local pnpm already
  matches.
- Adding a JS linter/formatter (the CLAUDE.md fix keeps saying none is configured — that part is
  still true).
- Any change to `ubt.toml` content, `vitest.config.ts`, lockfiles, or test code.
- Updating the toolchain pins themselves (e.g. moving to a newer Rust minor) — future routine
  maintenance, not this spec.
