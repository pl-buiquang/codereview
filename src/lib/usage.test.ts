import { describe, expect, it } from "vitest";
import { featureLabel, formatCost, formatDuration, formatTokens, runSubject, shortModel } from "./usage";

describe("usage formatting", () => {
  it("formats cost", () => {
    expect(formatCost(0)).toBe("$0");
    expect(formatCost(0.004)).toBe("<$0.01");
    expect(formatCost(1.644545)).toBe("$1.64");
  });

  it("formats token counts", () => {
    expect(formatTokens(83)).toBe("83");
    expect(formatTokens(1500)).toBe("1.5k");
    expect(formatTokens(164246)).toBe("164k");
    expect(formatTokens(2_500_000)).toBe("2.50M");
  });

  it("formats durations", () => {
    expect(formatDuration(5362)).toBe("5s");
    expect(formatDuration(125_000)).toBe("2m 05s");
    expect(formatDuration(3_900_000)).toBe("1h 05m");
  });

  it("shortens inference-profile ARNs only", () => {
    expect(shortModel("arn:aws:bedrock:eu-west-1:1:application-inference-profile/abc[1m]")).toBe("abc[1m]");
    expect(shortModel("claude-opus-5-5")).toBe("claude-opus-5-5");
  });

  it("labels features with their variant", () => {
    expect(featureLabel("ai_review", "critique")).toBe("AI review · critique");
    expect(featureLabel("focus")).toBe("Focus");
    expect(featureLabel("routine", "babysit")).toBe("Routine · babysit");
    expect(featureLabel("future_thing")).toBe("future_thing");
  });

  it("describes the subject of a run", () => {
    const base = { repo: null, pr_number: null, base_ref: null, head_ref: null, label: "" };
    expect(runSubject({ ...base, repo: "acme/w", pr_number: 7 })).toBe("acme/w#7");
    expect(runSubject({ ...base, repo: "acme/w", base_ref: "main", head_ref: "feat" })).toBe("acme/w main…feat");
    expect(runSubject({ ...base, label: "Board overview" })).toBe("Board overview");
    expect(runSubject(base)).toBe("—");
  });
});
