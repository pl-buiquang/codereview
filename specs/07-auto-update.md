# Spec 07 — Auto-update (Tauri updater plugin)

Implements ROADMAP §8 "Auto-update via the Tauri updater plugin". Builds on Spec 06's
`.github/workflows/release.yml` (tauri-action, draft release on `app-v*` tags) — implement Spec 06
first; this spec only *adds* to that workflow file.

## Problem

The app has no update mechanism at all — users who install a build never learn a newer release
exists, and the only upgrade path is a manual re-download:

- `src-tauri/tauri.conf.json` has no `plugins` key and no `bundle.createUpdaterArtifacts`
  (the `bundle` block at `tauri.conf.json:25-40` stops at `icon`).
- `src-tauri/Cargo.toml:23-35` lists `tauri-plugin-opener/-dialog/-fs/-window-state` but no
  `tauri-plugin-updater` or `tauri-plugin-process`; `lib.rs:23-27` registers only those four.
- `src-tauri/capabilities/default.json:6-12` grants no updater/process permissions.
- `package.json` has no `@tauri-apps/plugin-updater` / `@tauri-apps/plugin-process` guest packages,
  and nothing in `src/` ever checks for an update.

## Decisions (locked)

- **Minisign keypair, generated OUTSIDE the repo** (`~/.tauri/codereview.key`). The private key
  exists only there + as the GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`; **only the public
  key is committed** (in `tauri.conf.json`). This is independent of Apple/Windows code signing,
  which stays absent (Spec 06).
- **Endpoint:** the static `https://github.com/pl-buiquang/codereview/releases/latest/download/latest.json`
  produced by tauri-action's `includeUpdaterJson`. No update server, no per-platform URL templates.
- **Releases stay drafts until manually published** (Spec 06). `releases/latest/download/…` only
  resolves *published, non-prerelease* releases, so drafts and `-rc` prereleases are invisible to
  installed apps — that is the desired safety valve, not a bug.
- **Check once at app start, banner UI.** No polling, no menu item, no settings toggle. Rationale:
  smallest useful surface; the app restarts often enough on a desktop.
- **Dev + failure = silent.** `import.meta.env.DEV` skips the check entirely; any check error
  (offline, malformed JSON) is `console.warn`-ed and swallowed. Startup must never toast about
  updates. Install-time failures *do* toast (the user explicitly clicked).
- **Plugins registered unconditionally** in `run()` — this is a desktop-only app; no
  `#[cfg(desktop)]` dance.
- **Dismiss is per-session** (component state only) — the banner returns on next launch. No
  persistence, no "skip this version".
- **Frontend wrapper lives in `src/lib/updater.ts`**, not `src/lib/api.ts` — these are plugin
  calls, not backend `invoke`s; `api.ts` stays the backend-command boundary.

## Design

### 1. One-time keygen + secrets (manual, OUTSIDE the repo)

```bash
mkdir -p ~/.tauri
pnpm tauri signer generate -w ~/.tauri/codereview.key
# Prompts for an optional password; prints the public key (base64, one line).

gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/codereview.key
# Only if you set a password:
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --body '<the password>'
```

**BACKUP — do this immediately and tell the user to confirm it:** copy
`~/.tauri/codereview.key`, `~/.tauri/codereview.key.pub`, and the password into a password manager
or offline backup. The pubkey is baked into every shipped binary; **if the private key is lost, no
installed app can ever accept another update** — the only recovery is every user manually
reinstalling a build signed with a new key. There is no rotation mechanism.

**CRITICAL guard — the private key must NEVER be committed.** The keyfile lives outside the repo by
construction, but before *every* commit in this spec run:

```bash
git grep -qi "untrusted comment" && echo "KEY MATERIAL IN REPO — STOP" || echo clean
git diff --cached | grep -niE "untrusted comment|secret key" && echo "STOP" || echo clean
```

Minisign key files (both halves) start with `untrusted comment:`; the pubkey committed in
`tauri.conf.json` is base64-encoded and will not match. Also append `*.key` to `.gitignore` as a
belt-and-braces measure.

### 2. Rust side — `src-tauri/Cargo.toml`, `lib.rs`, capabilities

`Cargo.toml` `[dependencies]`:

```toml
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
```

`lib.rs` `run()` — after the `tauri_plugin_window_state` line (`lib.rs:27`):

```rust
.plugin(tauri_plugin_updater::Builder::new().build())
.plugin(tauri_plugin_process::init())
```

`src-tauri/capabilities/default.json` — extend `permissions`:

```json
"updater:default",
"process:default"
```

(`updater:default` = allow-check/download/install/download-and-install; `process:default` =
allow-exit/allow-restart — exactly what `check()`, `downloadAndInstall()`, `relaunch()` need.)

### 3. Config — `src-tauri/tauri.conf.json`

Add to the existing `bundle` block and a new top-level `plugins` key:

```json
"bundle": {
  "createUpdaterArtifacts": true
},
"plugins": {
  "updater": {
    "pubkey": "<paste the one-line base64 public key printed by `tauri signer generate`>",
    "endpoints": [
      "https://github.com/pl-buiquang/codereview/releases/latest/download/latest.json"
    ]
  }
}
```

`createUpdaterArtifacts: true` (the plain v2 form — this app never shipped a Tauri-v1 updater, so
no `"v1Compatible"`).

**Local-build gotcha (document in the README/Spec-06 release doc):** once
`createUpdaterArtifacts` is on, `pnpm tauri build` **fails unless the signing env vars are set**:

```bash
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/codereview.key)" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD='<password if set>' \
pnpm tauri build
```

`pnpm build` / `pnpm tauri dev` are unaffected.

### 4. CI — `.github/workflows/release.yml` (file created by Spec 06)

In the `tauri-apps/tauri-action` step add:

```yaml
        with:
          # …existing Spec 06 inputs (tagName, releaseName, releaseDraft: true, …)
          includeUpdaterJson: true
        env:
          # …existing env
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
```

Each release then carries, per platform, the updater bundle + detached `.sig` (Linux:
`*.AppImage` + `.sig`; macOS: `*.app.tar.gz` + `.sig`; Windows: NSIS `*-setup.exe` + `.sig`) plus
one aggregated `latest.json` listing `version`, `pub_date`, and `platforms.{linux-x86_64,
darwin-x86_64, darwin-aarch64, windows-x86_64}.{url, signature}`.

An update is only *offered* when `latest.json`'s `version` is semver-greater than the installed
app's — so the Spec 06 tag flow's version bump in `tauri.conf.json` is what actually arms the
updater.

### 5. Frontend wrapper — `src/lib/updater.ts` (NEW)

```ts
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type { Update };

/**
 * One-shot update check. Resolves null when up to date, when running in dev,
 * or on ANY failure (offline, bad endpoint, …). Never throws — app startup
 * must not surface updater noise.
 */
export async function checkForUpdate(): Promise<Update | null> {
  if (import.meta.env.DEV) return null;
  try {
    return await check();
  } catch (err) {
    console.warn("update check failed:", err);
    return null;
  }
}

/** Download + verify + install the update, then restart the app. Throws on failure. */
export async function installAndRelaunch(update: Update): Promise<void> {
  await update.downloadAndInstall();
  await relaunch();
}
```

`package.json` `dependencies`: add `"@tauri-apps/plugin-updater": "^2"` and
`"@tauri-apps/plugin-process": "^2"` (`pnpm add` both).

### 6. UI — `src/components/UpdateBanner.tsx` (NEW) + `App.tsx` + `styles.css`

```
┌──────────────────────────────────────────────────────────────────────┐
│ Update available: v0.2.0          [Install & relaunch]   [Dismiss]  │ ← .update-banner (slim, full-width)
├──────────────────────────────────────────────────────────────────────┤
│ [⌂] [my-repo] [PR #12 ✕]                                   TabBar   │
│ … tab content …                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

Component contract:

```tsx
export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [installing, setInstalling] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => { checkForUpdate().then(setUpdate); }, []);

  if (!update || dismissed) return null;
  // "Update available: v{update.version}"
  // [Install & relaunch] → setInstalling(true); installAndRelaunch(update)
  //     .catch(e => { toast.error(`Update failed: ${e}`); setInstalling(false); })
  //   label becomes "Installing…" and is disabled while installing
  // [Dismiss] → setDismissed(true)
}
```

- Reuse `toast.error` from `src/lib/toast.ts` for install failures (it works outside React state
  flows and matches how mutations report errors elsewhere).
- Mount in `src/App.tsx` as the first child of `<div className="app-shell">` (above `<TabBar />`,
  `App.tsx:44-49`) so it pushes content down rather than overlaying it.
- Styles: add a `.update-banner` block to `src/styles.css` (near `.toast-stack`, `styles.css:1435`)
  — single flex row, small font, accent background, buttons reuse existing button classes.

### Data flow

App start → `UpdateBanner` mounts → `checkForUpdate()` → plugin GETs `latest.json` from the
endpoint, compares `version` against the installed version, picks the current-platform entry →
`Update | null` → banner renders → user clicks Install → `downloadAndInstall()` downloads the
platform bundle, verifies the `.sig` against the `pubkey` baked into the binary, installs →
`relaunch()` restarts into the new version. No backend command, no DB, no `api.ts` involvement.

## Tasks

1. **Keygen + secrets (manual, no commit):** run the §1 keygen, set both `gh secret`s, back up the
   key, and paste the pubkey somewhere transient for task 3. Ask the user to confirm the backup.
2. **Rust plugins:** `Cargo.toml` deps + `lib.rs` registration + `capabilities/default.json`
   permissions. Builds standalone (`cargo clippy`/`cargo test` pass with config not yet present).
3. **Config:** `tauri.conf.json` `createUpdaterArtifacts` + `plugins.updater` (pubkey, endpoint).
   Append `*.key` to `.gitignore`. Run the key-material guard grep.
4. **Guest packages:** `pnpm add @tauri-apps/plugin-updater @tauri-apps/plugin-process`.
5. **Wrapper:** `src/lib/updater.ts` + `src/lib/updater.test.ts`.
6. **UI:** `UpdateBanner.tsx` + `UpdateBanner.test.tsx`, mount in `App.tsx`, `.update-banner` CSS.
7. **CI:** `release.yml` gains `includeUpdaterJson: true` + the two signing env vars; extend the
   Spec 06 release doc with the local-build env-var gotcha (§3) and the key-backup warning (§1).
8. Final guard grep over `git log -p` for the new commits, then push.

## Test matrix

No Rust tests — this spec adds zero backend logic (plugin registration is covered by
`cargo clippy`/`cargo test` compiling `lib.rs`). Vitest only:

`src/lib/updater.test.ts` (mock `@tauri-apps/plugin-updater` + `@tauri-apps/plugin-process` with
`vi.mock`, the established pattern in `src/lib/api.test.ts:1-13`). Note: under vitest
`import.meta.env.DEV` is `true`, so the non-dev paths need `vi.stubEnv("DEV", false)` (vitest's
`stubEnv` covers `import.meta.env`); call `vi.unstubAllEnvs()` in `afterEach`.

| Test | Asserts |
|---|---|
| `skips the check in dev mode` | with default (DEV=true) env, resolves `null` and the mocked `check` was **not** called |
| `returns null when up to date` | DEV stubbed false, `check` resolves `null` → `null` |
| `returns the update when available` | `check` resolves a fake `Update` → same object returned |
| `swallows check failures` | `check` rejects → resolves `null`, `console.warn` called (spy), nothing thrown |
| `installAndRelaunch orders download before relaunch` | both mocked; assert `downloadAndInstall` resolved before `relaunch` was invoked, and rejection of `downloadAndInstall` propagates without calling `relaunch` |

`src/components/UpdateBanner.test.tsx` (mock `../lib/updater`; render with Testing Library, mirror
`Markdown.test.tsx`):

| Test | Asserts |
|---|---|
| `renders nothing when no update` | `checkForUpdate` → `null`; container is empty |
| `shows version and actions when update available` | banner text contains the version; both buttons present |
| `install button installs and disables` | click → `installAndRelaunch` called with the update; button disabled / label "Installing…" while pending |
| `install failure toasts and re-enables` | `installAndRelaunch` rejects → `useToastStore.getState().toasts` contains an error toast; button enabled again |
| `dismiss hides the banner` | click Dismiss → banner gone; `installAndRelaunch` never called |

## Gates

The standard suite:

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test` (vitest run)
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

Spec-specific:

- **Key-material guard:** `git grep -qi "untrusted comment"` must find nothing in the tree, and
  `git log -p <new commits> | grep -ciE "untrusted comment|secret key"` must be 0.
- `actionlint .github/workflows/release.yml` if available (or at minimum a YAML parse check) after
  task 7.

## Manual verify

1. **Dev untouched:** `pnpm tauri dev` → no banner, no request to github.com/…/latest.json (dev
   gate), app behaves as before.
2. **Signed local build:** run the §3 env-var `pnpm tauri build` → succeeds and
   `src-tauri/target/release/bundle/` contains the AppImage **plus** a matching `.sig` file.
   Then run `pnpm tauri build` *without* the env vars → it must fail with a missing-private-key
   error (proves `createUpdaterArtifacts` is live).
3. **RC release inspection (prereleases are invisible to `latest/download` — inspect the rc
   release's own assets):** bump `tauri.conf.json`/`package.json`/`Cargo.toml` versions to
   `0.1.1-rc.1` on a throwaway commit, tag `app-v0.1.1-rc.1`, push the tag, let the workflow run,
   publish the draft **as a prerelease**, then:
   ```bash
   gh release view app-v0.1.1-rc.1 --repo pl-buiquang/codereview --json assets -q '.assets[].name'
   curl -L https://github.com/pl-buiquang/codereview/releases/download/app-v0.1.1-rc.1/latest.json
   ```
   Assert: assets include per-platform bundles + `.sig`s + `latest.json`; the JSON has the right
   `version` and a `platforms` entry per OS with non-empty `signature`. Confirm
   `https://github.com/pl-buiquang/codereview/releases/latest/download/latest.json` does **not**
   serve the rc (404 or an older release) — that is the documented prerelease behavior.
4. **End-to-end update loop (first two real releases):** install the `0.1.0` Linux AppImage, then
   publish a real `0.1.1` release (non-prerelease, non-draft). Launch the old AppImage → banner
   "Update available: v0.1.1" appears → Dismiss hides it → relaunch the app, banner returns →
   click "Install & relaunch" → app restarts and the banner no longer appears (now on 0.1.1).
5. **Failure path:** with Wi-Fi off, launch the installed app → no banner, no error toast, clean
   console besides the `update check failed` warn.

## Out of scope

- No periodic re-check, no "Check for updates" menu/settings entry — startup check only.
- No settings toggle to disable auto-update checks.
- No download progress UI (`downloadAndInstall`'s progress callback unused; static "Installing…").
- No release-notes rendering in the banner (`update.body` ignored).
- No deb/rpm auto-update — the Tauri updater only handles AppImage on Linux; deb/rpm users
  re-install manually. No attempt to detect the install method.
- No key rotation story beyond the backup warning; no Apple/Windows code signing (Spec 06's
  ready-to-enable docs stand).
- No `api.ts`/backend-command/DB changes of any kind (migrations 0007/0008 stay reserved for
  specs 12/16).
