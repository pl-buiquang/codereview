import { describe, expect, it } from "vitest";
import { parseDiff } from "react-diff-view";
import { findDiffMatches, wrapIndex } from "./diffSearch";

const DIFF = `diff --git a/foo.ts b/foo.ts
index 1111111..2222222 100644
--- a/foo.ts
+++ b/foo.ts
@@ -1,3 +1,3 @@
 const x = 1;
-const y = 2;
+const y = 42;
 const z = 3;
`;

const SECOND = `diff --git a/bar.ts b/bar.ts
index 3333333..4444444 100644
--- a/bar.ts
+++ b/bar.ts
@@ -1,1 +1,1 @@
-let q = 1;
+let q = 42;
`;

describe("findDiffMatches", () => {
  it("returns nothing for a blank query", () => {
    const files = parseDiff(DIFF);
    expect(findDiffMatches(files, "")).toEqual([]);
    expect(findDiffMatches(files, "   ")).toEqual([]);
  });

  it("matches every change type case-insensitively", () => {
    const files = parseDiff(DIFF);
    // 2 context + 1 delete + 1 insert lines all contain "const"
    expect(findDiffMatches(files, "CONST").length).toBe(4);
  });

  it("matches inserted text only on the insert line", () => {
    const files = parseDiff(DIFF);
    expect(findDiffMatches(files, "= 42").length).toBe(1);
  });

  it("matches deleted text only on the delete line", () => {
    const files = parseDiff(DIFF);
    expect(findDiffMatches(files, "y = 2").length).toBe(1);
  });

  it("spans all files in file-then-document order", () => {
    const files = parseDiff(DIFF + SECOND);
    const m = findDiffMatches(files, "42");
    expect(m.map((x) => x.fileIndex)).toEqual([0, 1]);
  });
});

describe("wrapIndex", () => {
  it("wraps forwards and backwards", () => {
    expect(wrapIndex(0, 3)).toBe(0);
    expect(wrapIndex(3, 3)).toBe(0);
    expect(wrapIndex(-1, 3)).toBe(2);
    expect(wrapIndex(4, 3)).toBe(1);
  });

  it("is 0 for an empty set", () => {
    expect(wrapIndex(5, 0)).toBe(0);
  });
});
