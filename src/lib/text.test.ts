import { describe, it, expect } from "vitest";
import { summaryLine } from "./text";

describe("summaryLine", () => {
  it("picks the first non-empty line, trimmed", () => {
    expect(summaryLine("\n\n  hello world  \nsecond")).toBe("hello world");
  });

  it("returns empty string for a blank body", () => {
    expect(summaryLine("")).toBe("");
    expect(summaryLine("   \n\t\n")).toBe("");
  });

  it("truncates lines longer than the cap with an ellipsis", () => {
    const long = "a".repeat(100);
    const out = summaryLine(long);
    expect(out).toHaveLength(80);
    expect(out.endsWith("…")).toBe(true);
    expect(out.slice(0, 79)).toBe("a".repeat(79));
  });

  it("leaves lines at or under the cap untouched", () => {
    const exact = "b".repeat(80);
    expect(summaryLine(exact)).toBe(exact);
  });

  it("respects a custom max", () => {
    expect(summaryLine("abcdef", 4)).toBe("abc…");
  });
});
