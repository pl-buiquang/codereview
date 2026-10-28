import { describe, expect, it } from "vitest";
import { identifierAt } from "./definition";

describe("identifierAt", () => {
  const line = "  let x = compute_total(items) + $el.empty? + 42;";

  it("picks the word under the caret, including both edges", () => {
    expect(identifierAt(line, line.indexOf("compute_total") + 3)).toBe("compute_total");
    expect(identifierAt(line, line.indexOf("compute_total"))).toBe("compute_total");
    expect(identifierAt(line, line.indexOf("(items"))).toBe("compute_total");
  });

  it("keeps a trailing ? or ! (ruby-style names) and a leading $", () => {
    expect(identifierAt(line, line.indexOf("empty") + 1)).toBe("empty?");
    expect(identifierAt(line, line.indexOf("$el") + 1)).toBe("$el");
  });

  it("returns null on whitespace, punctuation and numbers", () => {
    expect(identifierAt(line, 1)).toBeNull();
    expect(identifierAt(line, line.indexOf("42") + 1)).toBeNull();
    expect(identifierAt("a + b", 1)).toBe("a"); // caret just after the word
    expect(identifierAt("a + b", 2)).toBeNull();
    expect(identifierAt("", 0)).toBeNull();
  });
});
