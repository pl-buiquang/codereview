# Code signing

Status: **builds are unsigned.** This doc covers (1) what that means for users today,
(2) how to enable macOS signing + notarization, (3) how to enable Windows signing.
CI is pre-wired (.github/workflows/release.yml, "Stage code-signing secrets" step):
setting the repository secrets below is sufficient for macOS; Windows additionally
needs one tauri.conf.json edit.

Nothing in this document has to be done to ship the current unsigned releases — the
release pipeline is green with zero signing secrets set. Read it when you have bought
a certificate and want to flip signing on without touching the workflow.

## 1. Today: installing the unsigned builds

- **macOS** — downloads carry the `com.apple.quarantine` xattr; Gatekeeper shows
  "cannot be opened because the developer cannot be verified" (or "is damaged").
  Fix: `xattr -dr com.apple.quarantine /Applications/CodeReview.app`.
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
   "Developer ID Application: <Name> (<TEAMID>)" so the signing key is included →
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

Once these are set, the next `app-v*` tag produces a signed (and, if the
`APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` trio is present, notarized) macOS
bundle. No workflow edit is required — the "Stage code-signing secrets" step
copies the non-empty secrets into the build environment.

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

## 5. Updater signing (minisign) is separate

The release pipeline also signs the auto-updater artifacts with a **minisign**
keypair (Spec 07) — this is unrelated to OS code signing above. The minisign
signing half lives only in `~/.tauri/codereview.key` and as the GitHub secret
`TAURI_SIGNING_PRIVATE_KEY` (plus `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if you
set a password); only the matching public key is committed, in
`src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.

**Back up the minisign key immediately after generating it.** The pubkey is
baked into every shipped binary; if the signing half is lost, no installed app
can ever accept another update — the only recovery is every user manually
reinstalling a build signed with a new key. There is no rotation mechanism. Copy
`~/.tauri/codereview.key`, `~/.tauri/codereview.key.pub`, and the password into a
password manager or offline backup.

**Local-build gotcha:** once `bundle.createUpdaterArtifacts` is on (it is),
`pnpm tauri build` **fails unless the minisign signing env vars are set**:

```bash
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/codereview.key)" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD='<password if set>' \
pnpm tauri build
```

`pnpm build` / `pnpm tauri dev` are unaffected. In CI the two secrets are passed
to tauri-action via the `env:` block in `.github/workflows/release.yml`.
