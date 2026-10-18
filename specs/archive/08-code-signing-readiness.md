# Spec 08 — Code-signing readiness (docs + conditional CI wiring, no live signing)

Implements the **code signing & notarization** bullet of ROADMAP §8 (`ROADMAP.md:112`). No Apple
or Windows certificate exists today and none is being bought in this spec — the deliverable is
(a) a `docs/signing.md` that lets the operator go from "bought a cert" to "signed CI builds" with
zero code archaeology, and (b) `.github/workflows/release.yml` wiring that **signs when the
secrets exist and stays unsigned (and green) when they don't**. Builds on Spec 06's `release.yml`;
apply **after Spec 07**, which extends the same file (this spec's edits assume 07's
`TAURI_SIGNING_PRIVATE_KEY` env lines and `includeUpdaterJson` are already present).

## Problem

- Releases are unsigned by design (Spec 06 decision), but the only user-facing mitigation is two
  sentences in the release body (`release.yml` `releaseBody`, Spec 06 §Design). There is no
  document explaining what unsigned means per-OS, nor what it takes to turn signing on.
- `release.yml` carries only a placeholder: `# Spec 08 adds the conditional Apple/Windows signing
  env here.` (in the tauri-action `env:` block, Spec 06 §Design). Nothing references the signing
  secrets yet.
- No `docs/` directory exists in the repo at all (verified: `ls docs` → not found).
- **The naive wiring is a trap.** The obvious move — adding
  `APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}` to the tauri-action `env:` block — breaks
  the macOS build while the secrets are absent. GitHub Actions resolves a missing secret to the
  **empty string** and sets the env var anyway, and tauri-bundler gates signing on *presence*, not
  non-emptiness (verified in source, tauri `dev` branch, 2026-06):
  - `crates/tauri-bundler/src/bundle/macos/sign.rs`, `pub fn keychain()`:
    `if let (Some(certificate_encoded), Some(certificate_password)) = (var_os("APPLE_CERTIFICATE"), var_os("APPLE_CERTIFICATE_PASSWORD"))`
    — `var_os` returns `Some("")` for a set-but-empty var, so the cert branch is entered, an empty
    `.p12` is base64-decoded and fed to `security import`
    (`crates/tauri-macos-sign/src/keychain.rs`, `with_certificate` → `with_certificate_file`),
    which fails and aborts the bundle (`.map_err(Box::new)?`). There is **no `is_empty()` check
    anywhere on this path**.
  - Same hole for `APPLE_SIGNING_IDENTITY`: tauri-cli (`crates/tauri-cli/src/interface/rust.rs:1450-1458`)
    maps a set-but-empty var to `Some("")` → `Keychain::with_signing_identity("")` → `codesign -s ""`
    fails.
  - tauri-action does **not** filter or inspect these vars — the only `APPLE_CERTIFICATE` hit in
    the whole `tauri-apps/tauri-action` repo is an example workflow
    (`examples/publish-to-auto-release-universal-macos-app-with-signing-certificate.yml`); env
    passes through to the spawned `tauri build` untouched.

  So "empty env must not break the build" can only be met by **not setting the vars at all** when
  the secrets are absent — which GH Actions cannot express in a step `env:` block (there is no
  way to conditionally *omit* a key).

## Decisions (locked)

- **Readiness only.** No certificate purchase, no live signing run, no notarization attempt. The
  acceptance bar is: docs complete, CI green and unsigned with zero secrets set, and turning
  signing on later requires only `gh secret set` (macOS) / `gh secret set` + one documented conf
  edit (Windows) — no workflow edits.
- **Conditional staging step, not a plain `env:` block** (my call, forced by the Problem section's
  source evidence): a `bash` step before tauri-action copies each signing secret into
  `$GITHUB_ENV` **only when non-empty**. Subsequent steps (tauri-action and the `tauri build` it
  spawns) inherit exactly the vars that were staged; absent secrets leave the vars genuinely unset
  and tauri-bundler's `keychain()` returns `Ok(None)` → unsigned, no error.
- **Fail fast on partial secret sets** (my call): `APPLE_CERTIFICATE` ⊕ `APPLE_CERTIFICATE_PASSWORD`
  alone, or a partial `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` trio, is always operator error —
  the bundler would either silently skip signing or hard-fail mid-cargo
  (`NotarizeAuthError::MissingTeamId` is a hard error, `crates/tauri-bundler/src/bundle/macos/app.rs:144-145`).
  The staging step errors immediately with a readable message instead.
- **Windows ready path = Azure Trusted Signing** (my call): its three `AZURE_*` env vars fit the
  same staging mechanism and are inert until `bundle.windows.signCommand` is added to
  `tauri.conf.json` (documented, not committed — adding it now with no Azure account would break
  Windows builds). Classic OV `.pfx` certs are **documented only, no CI wiring**: since the 2023
  CA/B-forum rules new OV keys must live on hardware tokens/HSMs, so the old
  base64-pfx-in-a-secret flow rarely applies to newly issued certs.
- **`docs/signing.md` is the single signing document**; `release.yml` comments and the README
  point at it. (First file in a new `docs/` directory.)
- One staging step for all platforms, not per-OS `if:` guards (my call): staging an `APPLE_*` var
  on a Windows runner is harmless dead env; one step is simpler to audit than three.

## Design

### 1. NEW `docs/signing.md`

Full skeleton — implementer fills prose around it, all commands/values below are load-bearing:

````markdown
# Code signing

Status: **builds are unsigned.** This doc covers (1) what that means for users today,
(2) how to enable macOS signing + notarization, (3) how to enable Windows signing.
CI is pre-wired (.github/workflows/release.yml, "Stage code-signing secrets" step):
setting the repository secrets below is sufficient for macOS; Windows additionally
needs one tauri.conf.json edit.

## 1. Today: installing the unsigned builds

- **macOS** — downloads carry the `com.apple.quarantine` xattr; Gatekeeper shows
  "cannot be opened because the developer cannot be verified" (or "is damaged").
  Fix: `xattr -dr com.apple.quarantine /Applications/codereview.app`.
  On macOS ≤ 14, right-click → Open → Open also works; on macOS 15+ the
  right-click bypass is gone — after one failed open, use System Settings →
  Privacy & Security → "Open Anyway". (arm64 binaries are ad-hoc signed by the
  linker automatically, so they run fine once quarantine is cleared.)
- **Windows** — SmartScreen interjects "Windows protected your PC" on the
  installer: click "More info" → "Run anyway".
- **Linux** — no OS gate; nothing to do (updater artifacts are minisign-verified
  separately, see Spec 07).

## 2. macOS: signing + notarization

**Cost:** Apple Developer Program, US$99/year (membership is what grants the
"Developer ID Application" certificate type). Notarization itself is free.

### One-time: create and export the certificate

1. Enroll at developer.apple.com; note your **Team ID** (Membership page).
2. Xcode → Settings → Accounts → Manage Certificates → "+" →
   **Developer ID Application** (or via Certificates, Identifiers & Profiles on
   the website + Keychain Access CSR).
3. Keychain Access → My Certificates → expand
   "Developer ID Application: <Name> (<TEAMID>)" so the private key is included →
   File → Export Items… → `.p12`, choose a strong export password.
4. Create an **app-specific password** for notarization:
   appleid.apple.com → Sign-In & Security → App-Specific Passwords.

### Set the secrets

```bash
base64 -i certificate.p12 | gh secret set APPLE_CERTIFICATE   # macOS base64: no line wrap
# (on Linux: base64 -w0 certificate.p12 | gh secret set APPLE_CERTIFICATE)
gh secret set APPLE_CERTIFICATE_PASSWORD       # the .p12 export password
gh secret set APPLE_SIGNING_IDENTITY           # "Developer ID Application: <Name> (<TEAMID>)"
gh secret set APPLE_ID                         # the Apple-account email
gh secret set APPLE_PASSWORD                   # the APP-SPECIFIC password, not the account password
gh secret set APPLE_TEAM_ID                    # e.g. A1B2C3D4E5
```

### What reads what (tauri v2)

| Secret / env var | Read by | Role |
|---|---|---|
| `APPLE_CERTIFICATE` | tauri-bundler `macos/sign.rs::keychain()` | base64 `.p12`, imported into a throwaway keychain |
| `APPLE_CERTIFICATE_PASSWORD` | same | `.p12` password — **must be set together with the cert** |
| `APPLE_SIGNING_IDENTITY` | tauri-cli → `settings.macos().signing_identity` | optional; cross-checked against the imported cert's identity |
| `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` | tauri-bundler `notarize_auth()` | notarization trio — **all three or none** (ID+password without team ID is a hard build error) |

Alternative notarization auth: an App Store Connect API key via
`APPLE_API_KEY` / `APPLE_API_ISSUER` / `APPLE_API_KEY_PATH` — not wired in CI;
add to the staging step's list if you switch.

Notarization only happens on **signed** builds (it runs inside the
signed-branch of the bundler), and the staging step enforces the pair/trio
rules, so partial configs fail fast in CI rather than mid-cargo.

## 3. Windows: signing

### Option A (recommended): Azure Trusted Signing

**Cost:** ~US$9.99/month (Basic tier; verify current pricing). Needs an Azure
account + identity validation. Good SmartScreen reputation out of the box.

1. Create a Trusted Signing account + certificate profile in Azure; create an
   App Registration with a client secret and the "Trusted Signing Certificate
   Profile Signer" role.
2. Set the secrets (already staged by CI when present):
   ```bash
   gh secret set AZURE_TENANT_ID
   gh secret set AZURE_CLIENT_ID
   gh secret set AZURE_CLIENT_SECRET
   ```
3. **The one conf edit** — add to `src-tauri/tauri.conf.json` `bundle`:
   ```json
   "windows": {
     "signCommand": "trusted-signing-cli -e https://<region>.codesigning.azure.net -a <Account> -c <Profile> -d codereview %1"
   }
   ```
   and prepend a `cargo install trusted-signing-cli` step to the Windows matrix
   job (the upstream Tauri docs now also call it `artifact-signing-cli`; check
   https://v2.tauri.app/distribute/sign/windows/ for the current crate name).
   The `AZURE_*` env vars are read by that CLI, not by tauri — they are inert
   until `signCommand` exists, which is why staging them early is safe.

### Option B: classic OV certificate (documented only, not CI-wired)

~US$200–500/year. Since June 2023, new OV keys must live on a hardware
token/HSM, so the historical "base64 the .pfx into a secret, import with
Import-PfxCertificate, set `certificateThumbprint`/`digestAlgorithm`/`timestampUrl`
in tauri.conf.json" flow only works for cloud-HSM-backed certs with a
signtool-compatible CSP. If you go this route, follow
https://v2.tauri.app/distribute/sign/windows/ and add the import step +
conf fields yourself; SmartScreen reputation still has to be earned.

## 4. How the CI wiring behaves

- No secrets set → staging step stages nothing → tauri-bundler's `keychain()`
  sees no `APPLE_*` vars → unsigned build, green run (today's state).
- macOS secrets set → staged into `$GITHUB_ENV` → tauri-action's `tauri build`
  inherits them → signed (+ notarized if the trio is set). No workflow edit.
- Empty-string secrets are treated as absent **by the staging step** — never
  exported — because tauri-bundler itself would treat a set-but-empty
  `APPLE_CERTIFICATE` as a signing request and fail the build.
- A *bogus* (non-empty, invalid) secret fails the build loudly. That is
  intentional: when signing was requested, failing beats shipping unsigned.
````

### 2. `.github/workflows/release.yml` — two edits

**(a) NEW step**, inserted between `Install frontend dependencies` and the tauri-action step
(Spec 06 §Design YAML, after its line `run: pnpm install --frozen-lockfile`):

```yaml
      # Spec 08: stage code-signing secrets ONLY when non-empty. Do NOT move
      # these into the tauri-action env: block — a missing GH secret resolves
      # to "" and tauri-bundler gates signing on env-var *presence* (var_os,
      # no is_empty), so a set-but-empty APPLE_CERTIFICATE fails the build
      # importing an empty .p12. Unset vars => unsigned build, green run.
      # See docs/signing.md.
      - name: Stage code-signing secrets (unsigned build when absent)
        shell: bash
        env:
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
          APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
          AZURE_TENANT_ID: ${{ secrets.AZURE_TENANT_ID }}
          AZURE_CLIENT_ID: ${{ secrets.AZURE_CLIENT_ID }}
          AZURE_CLIENT_SECRET: ${{ secrets.AZURE_CLIENT_SECRET }}
        run: |
          present() { [ -n "${!1}" ]; }
          stage() {
            name="$1"
            if present "$name"; then
              eof="EOF_${name}_${RANDOM}${RANDOM}"
              { echo "${name}<<${eof}"; echo "${!name}"; echo "${eof}"; } >> "$GITHUB_ENV"
              echo "staged ${name}"
            fi
          }
          count() { local n=0 v; for v in "$@"; do if present "$v"; then n=$((n+1)); fi; done; echo "$n"; }

          # Pair/trio guards — partial sets are operator error; fail fast here
          # with a readable message instead of mid-cargo (or silently unsigned).
          pair="$(count APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD)"
          if [ "$pair" = "1" ]; then
            echo "::error::APPLE_CERTIFICATE and APPLE_CERTIFICATE_PASSWORD must be set together (docs/signing.md §2)"; exit 1
          fi
          trio="$(count APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID)"
          if [ "$trio" != "0" ] && [ "$trio" != "3" ]; then
            echo "::error::set all of APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID, or none (docs/signing.md §2)"; exit 1
          fi
          az="$(count AZURE_TENANT_ID AZURE_CLIENT_ID AZURE_CLIENT_SECRET)"
          if [ "$az" != "0" ] && [ "$az" != "3" ]; then
            echo "::error::set all of AZURE_TENANT_ID / AZURE_CLIENT_ID / AZURE_CLIENT_SECRET, or none (docs/signing.md §3)"; exit 1
          fi
          if [ "$trio" = "3" ] && [ "$pair" = "0" ]; then
            echo "::warning::notarization secrets set without APPLE_CERTIFICATE — notarization only runs on signed builds (docs/signing.md §2)"
          fi

          for v in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY \
                   APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID \
                   AZURE_TENANT_ID AZURE_CLIENT_ID AZURE_CLIENT_SECRET; do
            stage "$v"
          done
```

Implementation notes (keep as YAML comments only where shown above):

- Heredoc `$GITHUB_ENV` syntax handles multi-line values; the randomized delimiter cannot collide
  with base64 content (base64 alphabet has no `_`). Never `echo` a value outside the
  `>> "$GITHUB_ENV"` redirect — GH masks secret values in logs, but don't rely on it.
- `present`/`stage` use bash indirect expansion (`${!1}`); `shell: bash` is explicit so this also
  works on the Windows runner (git-bash). Guard counting uses `if present; then` (not
  `present && n=$((n+1))`) because GH's default `bash -e` would abort on the failing `&&` list.
- Values written to `$GITHUB_ENV` are visible to **all later steps in the job** — after this step
  only tauri-action and post-steps (rust-cache) run; acceptable for a single-job workflow.

**(b) Replace the placeholder comment** in the tauri-action `env:` block. Post-Spec-07 the block
is `GITHUB_TOKEN` + the two `TAURI_SIGNING_*` lines + the line
`# Spec 08 adds the conditional Apple/Windows signing env here.` — replace that one comment line
with:

```yaml
          # Apple/Windows signing env arrives via GITHUB_ENV from the
          # "Stage code-signing secrets" step above — never reference those
          # secrets here directly (empty string != unset for tauri-bundler).
          # See docs/signing.md.
```

### 3. `README.md` — one pointer

In the "Build the final artifact" section (`README.md:82`), after the existing bundle-output
notes, add one line:

```markdown
> Release builds are currently **unsigned** — see `docs/signing.md` for what that
> means on macOS/Windows and how to enable signing.
```

### Data flow

```
gh secret set APPLE_*  (operator, later)
        │
release.yml job:  …deps/setup… → [Stage code-signing secrets]
        │                              │ non-empty? ──no──▶ skip (var stays unset)
        │                              └──yes──▶ $GITHUB_ENV
        ▼
tauri-action → `tauri build` (inherits staged env)
        │
        ├─ no APPLE_* set  → keychain() → Ok(None) → unsigned bundle  (today)
        └─ cert pair set   → import .p12 → codesign → [trio set? notarize] → signed bundle
```

## Tasks

1. Verify preconditions: `.github/workflows/release.yml` exists with Spec 07's
   `TAURI_SIGNING_PRIVATE_KEY{,_PASSWORD}` env lines and `includeUpdaterJson: true`. If Spec 07
   has not landed, **stop and land it first** (both specs edit the same region of the same file;
   the locked order is 06 → 07 → 08).
2. Add `docs/signing.md` per §1 (`docs(signing): document signing readiness + unsigned reality`).
3. Edit `release.yml`: insert the staging step + swap the placeholder comment per §2
   (`ci(release): stage signing secrets conditionally, unsigned when absent`).
4. Add the README pointer per §3 (fold into commit 2 or 3).
5. Run the spec-specific gates (actionlint + local dry-runs below); fix findings.
6. Re-run a throwaway RC tag (Spec 06 Manual verify flow) to prove the workflow is still green
   with zero secrets set; delete the draft + tag afterwards.

## Test matrix

No Rust or vitest tests — no app code changes. The checks are:

| Check | What it asserts |
|---|---|
| `actionlint` on `release.yml` | workflow + embedded bash (shellcheck) still valid after the new step |
| Local dry-run, no vars set | extract the `run:` block to a temp script; `bash -e` it with no `APPLE_*`/`AZURE_*` in env and `GITHUB_ENV=$(mktemp)` → exits 0, `$GITHUB_ENV` file empty |
| Local dry-run, full macOS set | export all 6 `APPLE_*` (dummy values, one containing a newline) → exits 0; `$GITHUB_ENV` contains 6 heredoc blocks; multi-line value round-trips intact |
| Local dry-run, cert without password | export only `APPLE_CERTIFICATE` → exits 1 with the `must be set together` error |
| Local dry-run, partial notarize trio | export `APPLE_ID`+`APPLE_PASSWORD` only → exits 1 with the trio error |
| Local dry-run, empty-string vars | export `APPLE_CERTIFICATE=""` etc. → treated as absent: exits 0, nothing staged (the whole point) |
| Live RC run (task 6) | all 4 matrix jobs green; macOS job log shows the staging step staged nothing and **no** `Signing` line from the bundler |

## Gates

Standard suite (must stay green — trivially, no app code changes):

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test` (vitest run)
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

Spec-specific:

- `actionlint` over `.github/workflows/release.yml` (same invocation as Spec 06's gate).
- The six local dry-runs from the test matrix, run as one script before committing.

## Manual verify

1. Run the local dry-run matrix above (`GITHUB_ENV=$(mktemp)`; inspect the file between runs).
2. Push the commits, then tag `app-v0.1.0-rc.2` (or current conf version + `-rc.N`) and push the
   tag. Watch with `gh run watch`.
3. In each job's log: "Stage code-signing secrets" ran, staged nothing, no error. The macOS jobs'
   tauri-action output contains no `Signing` / `codesign` / keychain activity and the build
   succeeds → bundles attach to the draft as before.
4. `gh release delete app-v0.1.0-rc.2 --yes && git push origin :refs/tags/app-v0.1.0-rc.2 && git tag -d app-v0.1.0-rc.2`.
5. Doc sanity: follow `docs/signing.md` §2 step "Set the secrets" *up to but not including*
   running `gh secret set` (no real cert exists); confirm each command is copy-pasteable and the
   base64 round-trip works on a dummy file (`base64 -w0 /tmp/x.bin | base64 -d | cmp - /tmp/x.bin`).
6. Do **not** test with dummy non-empty secrets — an invalid `APPLE_CERTIFICATE` fails the macOS
   build *by design* (signing requested ⇒ fail loudly, never silently ship unsigned).

## Out of scope

- Buying any certificate, enrolling in the Apple Developer Program, creating an Azure account —
  and therefore any **live** signed/notarized build.
- The `tauri.conf.json` `bundle.windows.signCommand` edit and the `cargo install
  trusted-signing-cli` CI step (documented in `docs/signing.md` §3; committed only at Windows
  go-live, since they break the build without Azure credentials).
- OV `.pfx` CI wiring (`Import-PfxCertificate` step, `certificateThumbprint` conf) — documented
  as option B only.
- App Store Connect API-key notarization wiring (`APPLE_API_KEY*`) — one-line doc mention only.
- Linux signing (no OS gate exists); updater minisign signatures (Spec 07 owns those — the
  `TAURI_SIGNING_*` vars are unrelated to OS code signing and stay in the `env:` block).
- macOS entitlements / hardened-runtime tuning, `--skip-stapling`, universal binaries.
- Auto-detecting or warning about quarantine inside the app itself.
