import { describe, expect, it } from "vitest";
import { formatUsd, targetLabel } from "./StatusBar";
import type { Target } from "../lib/types";

const target = (over: Partial<Target>): Target => ({
  id: 1,
  repo_id: 1,
  kind: "local",
  github_pr_number: null,
  gh_pr_id: null,
  title: "t",
  base_ref: "main",
  head_ref: "feat",
  three_dot: true,
  created_at: "",
  ...over,
});

describe("StatusBar helpers", () => {
  it("labels PRs by number and local targets by ref range", () => {
    expect(targetLabel(target({ kind: "github_pr", github_pr_number: 42 }))).toBe("#42");
    expect(targetLabel(target({}))).toBe("main...feat");
    expect(targetLabel(target({ three_dot: false }))).toBe("main..feat");
  });
  it("formats dollars with two decimals", () => {
    expect(formatUsd(1.234)).toBe("$1.23");
    expect(formatUsd(0)).toBe("$0.00");
  });
});
