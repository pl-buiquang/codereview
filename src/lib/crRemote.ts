/** Helpers for the Settings → CLI remote-host install commands. */

export const RELEASES_URL = "https://github.com/pl-buiquang/codereview/releases";

/** Platforms the release workflow publishes a standalone `cr` for. */
export const CR_PLATFORMS = [
  { key: "x86_64-unknown-linux-gnu", label: "Linux x86_64", linux: true },
  { key: "aarch64-apple-darwin", label: "macOS Apple silicon", linux: false },
  { key: "x86_64-apple-darwin", label: "macOS Intel", linux: false },
] as const;
export type CrPlatform = (typeof CR_PLATFORMS)[number]["key"];

/** The release tag of a build version: `0.8.1-3+abc123` → `0.8.1` (tags are bare `X.Y.Z`). */
export function releaseTag(version: string): string {
  return version.split(/[-+]/)[0];
}

/** True when the version is exactly a release (not a local or dev build on top of one). */
export function isReleaseVersion(version: string): boolean {
  return !/[-+]/.test(version.replace(/\+[0-9a-f]+$/, ""));
}

export function crAssetUrl(tag: string, platform: CrPlatform): string {
  return `${RELEASES_URL}/download/${tag}/cr-${platform}`;
}

/** Single-quote for a POSIX shell. */
function sq(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

function overSsh(alias: string, script: string): string {
  return `ssh ${alias || "<host>"} ${sq(script)}`;
}

/** Download the release `cr` into `~/.local/bin` on the host and print its version. */
export function installCommand(alias: string, tag: string, platform: CrPlatform): string {
  return overSsh(
    alias,
    `mkdir -p ~/.local/bin && curl -fsSL -o ~/.local/bin/cr ${crAssetUrl(tag, platform)} && chmod +x ~/.local/bin/cr && ~/.local/bin/cr info`,
  );
}

/** First-time daemon setup (Linux: systemd user unit + linger). */
export function workerSetupCommand(alias: string): string {
  return overSsh(alias, "~/.local/bin/cr worker --install && loginctl enable-linger \"$USER\" && ~/.local/bin/cr worker --status");
}

/** After an update: restart the daemon so it runs the new binary. */
export function workerRestartCommand(alias: string): string {
  return overSsh(alias, "systemctl --user restart cr-worker && ~/.local/bin/cr worker --status");
}
