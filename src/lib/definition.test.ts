import { describe, expect, it } from "vitest";
import { identifierAt } from "./definition";

describe("identifierAt", () => {
  const line = "  let x = compute_total(items) + $el.empty? + 42;";
  const word = (offset: number) => identifierAt(line, offset)?.word ?? null;

  it("picks the word under the caret, including both edges", () => {
    expect(word(line.indexOf("compute_total") + 3)).toBe("compute_total");
    expect(word(line.indexOf("compute_total"))).toBe("compute_total");
    expect(word(line.indexOf("(items"))).toBe("compute_total");
  });

  it("keeps a trailing ? or ! (ruby-style names) and a leading $", () => {
    expect(word(line.indexOf("empty") + 1)).toBe("empty?");
    expect(word(line.indexOf("$el") + 1)).toBe("$el");
  });

  it("returns null on whitespace, punctuation and numbers", () => {
    expect(word(1)).toBeNull();
    expect(word(line.indexOf("42") + 1)).toBeNull();
    expect(identifierAt("a + b", 1)?.word).toBe("a"); // caret just after the word
    expect(identifierAt("a + b", 2)).toBeNull();
    expect(identifierAt("", 0)).toBeNull();
  });

  it("reports the [start, end) range of the word, trailing ? included", () => {
    const i = line.indexOf("empty?");
    expect(identifierAt(line, i + 2)).toEqual({ word: "empty?", start: i, end: i + 6 });
    const j = line.indexOf("compute_total");
    expect(identifierAt(line, j)).toEqual({ word: "compute_total", start: j, end: j + 13 });
  });
});
