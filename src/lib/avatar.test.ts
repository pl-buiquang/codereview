import { describe, expect, it } from "vitest";
import { identicon } from "./avatar";

describe("identicon", () => {
  it("is deterministic and case/whitespace-insensitive", () => {
    expect(identicon("Ada Lovelace")).toBe(identicon(" ada lovelace "));
  });

  it("differs between seeds", () => {
    expect(identicon("alice")).not.toBe(identicon("bob"));
  });

  it("returns a mirrored svg data url", () => {
    const svg = decodeURIComponent(identicon("alice").split(",")[1]);
    expect(svg.startsWith("<svg")).toBe(true);
    const cells = [...svg.matchAll(/<rect x="(\d)" y="(\d)" width="1"/g)].map((m) => `${m[1]},${m[2]}`);
    for (const cell of cells) {
      const [x, y] = cell.split(",").map(Number);
      expect(cells).toContain(`${6 - x},${y}`);
    }
  });
});
