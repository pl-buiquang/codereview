# Security follow-up

Last reviewed: 2026-10-06. Owner: repository maintainer.

## Scope and completed work

The audit covered application trust boundaries, the current checkout, reachable
public Git history, pull-request content, available Actions logs, and dependency
advisories. It found public work-related reference data and application security
issues. No credentials were detected by the scans; this is not proof that none
were ever exposed. Some older Actions logs had expired and could not be checked.

Implemented for 0.1.2:

- Reject option-like deep-link refs and resolve commit refs with
  `git rev-parse --verify --end-of-options <ref>^{commit}` before running diffs.
  Diff and file helpers disable external diff and text-conversion programs.
  Regression tests verify the original file-overwrite attempt is rejected.
- Fetch authenticated images through an HTTP client with backend HTTPS/host
  validation on the initial URL and every redirect. Credentials are confined to
  GitHub attachment requests and permanently dropped across origin changes.
  Tokens do not appear in process arguments; downloads have size and timeout
  limits. Tests cover disallowed origins, redirects, headers, and streamed sizes.
- Update JavaScript and Rust dependencies and use Node 24 LTS. The local npm
  audit is clean. Remaining Rust findings are documented below.

## Prevention and release gates

- `ci.yml` runs frontend tests/builds and Rust tests/Clippy on Linux, Windows,
  and macOS. It calls `security.yml` on pushes and pull requests.
- `security.yml` also runs weekly and on manual dispatch. It audits all npm
  dependencies, audits the Rust lockfile with warnings treated as failures, and
  scans full fetched Git history with redacted Gitleaks output. Scanner failures
  fail the workflow; they are not treated as clean results.
- Only the two named GTK3 advisories below are temporarily excepted. CI refuses
  to use these exceptions after 2026-11-06 until they are reviewed explicitly.
- Dependabot checks npm, Cargo, and Actions weekly. Coupled React and Vitest
  packages are grouped. Updates still require review and passing CI.
- Third-party Actions are pinned to commit SHAs; Gitleaks is pinned to a release
  and a checked archive digest. Dependabot maintains the Action pins; scanner
  versions must also be reviewed when updating the workflow.
- `release.yml` calls the same CI/security gates for the release tag before
  building macOS arm64/x64, Windows, and Linux artifacts. Builds use the Cargo
  lockfile. Releases remain drafts until the maintainer publishes them.
- Local environment files, private signing containers, and review databases are
  ignored by Git. This does not remove anything already committed.

Secret-pattern scanners do not reliably identify confidential business context.
Review reference exports, fixtures, screenshots, and commit messages before
publishing. Compressed HTML exports require inspecting their decoded assets too.

## P1: remove work-related data from public history

Status: open. Visibility and public history have not been changed by this work.

The design handoff reference bundle contains work-related repository names,
pull-request metadata, contributor identifiers, and application code excerpts.
Some copies are embedded as compressed assets in standalone HTML. Related
examples also occur in settings, tests, CLI documentation, and workflow specs.
The affected material is reachable through both published branches and both
pre-0.1.2 tags. A deletion commit alone will not purge it.

Remediation:

1. Restrict repository visibility while preparing cleanup. Preserve a private
   backup outside the public checkout, including uncommitted work.
2. Replace current examples with synthetic data. Remove the original
   `specs/design_handoff_codereview_redesign/reference/` exports entirely unless
   every embedded asset has been rebuilt from synthetic inputs. Check references
   from the remaining design documentation.
3. In an isolated mirror, use `git-filter-repo` to remove the original exports
   and replace remaining sensitive strings across all affected branches/tags.
   Do not work on the only copy of the checkout. Inspect the rewritten history
   and decode exported assets again before publishing it.
4. Coordinate the force-push of rewritten refs and removal or rebuilding of
   affected release artifacts. Check all release tags, not only the newest one.
5. Ask GitHub Support to assess retained sensitive commit/PR views if needed.
   Support eligibility is determined by GitHub; cache removal is not guaranteed.
6. Re-clone or carefully clean every local checkout/worktree. Merging old history
   can reintroduce removed data. Independently retained downloads cannot be
   recalled.

Acceptance: a fresh remote mirror, every published ref, current files, and
decoded exports contain only synthetic examples. Record any external copies or
GitHub-retained references that could not be removed, without copying sensitive
values into this document.

Reference: [GitHub sensitive-data removal guidance](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/removing-sensitive-data-from-a-repository).

## P1: confine AI chat execution

Status: open. Chat currently preapproves unrestricted Bash and file editing and
executes inside the reviewed project. Reviewed source and project instructions
are untrusted input. `--add-dir` and a worktree do not constitute an OS sandbox.

Required work:

- Default review analysis to read-only access. Editing and command execution
  need an explicit mode with clear user consent.
- Enforce filesystem and network restrictions outside the model. Scope writes
  to the intended worktree; restrict credential access and outbound connections.
- Constrain project-provided hooks/settings and inherited environment variables.
  Preserve authentication deliberately rather than inheriting the entire shell
  environment as an implicit trust decision.
- Require approval for commands with external effects, including pushes,
  publishing reviews, and arbitrary shell execution. A prompt alone is not an
  enforcement mechanism.
- Use securely created temporary prompt files and reliable cleanup on failures.

Acceptance: hostile repository instructions cannot read credentials, write
outside the approved worktree, or publish/exfiltrate data without approval.
Exercise these boundaries with adversarial fixtures containing dummy secrets.

## P2: enforce a Content Security Policy and backend scopes

Status: open. `app.security.csp` remains null.

Define a restrictive production CSP for scripts, connections, images, styles,
fonts, and frames. Permit only the Tauri IPC and assets required by the app.
Handle development-server allowances separately. Preserve Markdown rendering,
syntax highlighting, local fonts, authenticated images, and updater behavior.

Review custom opener/export IPC commands too: enforce allowed URL schemes and
filesystem scope in Rust rather than relying exclusively on frontend checks.

Acceptance: the packaged app has an enforced CSP, injected scripts and unwanted
outbound requests are blocked, and legitimate workflows pass smoke tests.

## P2: replace the Linux GTK3 advisory exceptions

Status: open. Review deadline: **2026-11-06**.

| Advisory | Dependency path | Remaining issue |
| --- | --- | --- |
| RUSTSEC-2024-0429 | Tauri -> gtk 0.18 -> glib 0.18.5 | `VariantStrIter` unsoundness; fixed upstream in glib 0.20 |
| RUSTSEC-2024-0370 | Tauri -> gtk -> gtk3-macros -> proc-macro-error 1.0.4 | Unmaintained macro dependency |

These are Linux/BSD GTK3 dependencies and are outside the macOS dependency path.
The latest compatible Tauri stack still requires GTK3's older crate family.
Adding an unrelated newer glib version does not replace that dependency.
Dependency presence alone does not establish an exploitable application path.

Track the upstream migration, or maintain and test a narrowly scoped backport.
Remove each exception from `.cargo/audit.toml` when resolved. Remove the deadline
gate when both exceptions are gone; any extension must update the rationale and
deadline here, in the audit config, and in the security workflow together. New
advisories must fail CI rather than being folded into these exceptions.

## P2: signing and release verification

Updater signatures use the existing minisign key. OS code signing/notarization
remains unconfigured; see `docs/signing.md`. Back up signing keys outside Git.

Before publishing a draft, verify the platform asset inventory, signed updater
metadata, and that all artifacts come from the intended tag. Smoke-test opening
a review and authenticated images on each supported OS. Cross-platform CI does
not replace packaged GUI testing. Release and install new binaries to receive
the fixes; changing source alone does not update installed applications.

Never describe a release as having zero security risks while these items remain
open. Audit results are a point-in-time check against known advisories.
