import { describe, it, expect } from "vitest";
import { crAssetUrl, installCommand, isReleaseVersion, releaseTag } from "./crRemote";

describe("crRemote", () => {
  it("derives the release tag from a build version", () => {
    expect(releaseTag("0.8.1+d517356f")).toBe("0.8.1");
    expect(releaseTag("0.8.1-3+d517356f")).toBe("0.8.1");
    expect(isReleaseVersion("0.8.1+d517356f")).toBe(true);
    expect(isReleaseVersion("0.8.1-3+d517356f")).toBe(false);
  });

  it("builds the asset URL and an ssh install command", () => {
    expect(crAssetUrl("0.8.1", "x86_64-unknown-linux-gnu")).toBe(
      "https://github.com/pl-buiquang/codereview/releases/download/0.8.1/cr-x86_64-unknown-linux-gnu",
    );
    const cmd = installCommand("koza", "0.8.1", "x86_64-unknown-linux-gnu");
    expect(cmd.startsWith("ssh koza '")).toBe(true);
    expect(cmd).toContain("curl -fsSL -o ~/.local/bin/cr https://github.com/");
    expect(cmd.endsWith("~/.local/bin/cr info'")).toBe(true);
    expect(installCommand("", "0.8.1", "x86_64-unknown-linux-gnu").startsWith("ssh <host> ")).toBe(true);
  });
});
