import { describe, it, expect } from "vitest";
import {
  expandFromRawCode,
  getCollapsedLinesCountBetween,
  parseDiff,
} from "react-diff-view";
import {
  languageForPath,
  fileDisplayPath,
  indexFile,
  changeKeyOf,
  countChanges,
  hunkContextSnippet,
} from "./diff";

describe("languageForPath", () => {
  it("maps known extensions to refractor languages", () => {
    expect(languageForPath("src/main.ts")).toBe("typescript");
    expect(languageForPath("a/b/Component.tsx")).toBe("typescript");
    expect(languageForPath("lib.rs")).toBe("rust");
    expect(languageForPath("script.py")).toBe("python");
    expect(languageForPath("data.json")).toBe("json");
    expect(languageForPath("style.scss")).toBe("css");
  });

  it("lowercases the extension before lookup", () => {
    expect(languageForPath("README.MD")).toBe("markdown");
  });

  it("returns undefined for unknown or missing extensions", () => {
    expect(languageForPath("notes.unknownext")).toBeUndefined();
    expect(languageForPath("Makefile")).toBeUndefined();
  });

  it("uses only the basename, ignoring directory dots", () => {
    expect(languageForPath("my.dir/file.rs")).toBe("rust");
  });
});

describe("fileDisplayPath", () => {
  it("uses oldPath for deletes", () => {
    expect(
      fileDisplayPath({ type: "delete", oldPath: "gone.ts", newPath: "/dev/null" } as never),
    ).toBe("gone.ts");
  });

  it("uses newPath for renames and adds", () => {
    expect(
      fileDisplayPath({ type: "rename", oldPath: "old.ts", newPath: "new.ts" } as never),
    ).toBe("new.ts");
    expect(
      fileDisplayPath({ type: "add", oldPath: "/dev/null", newPath: "added.ts" } as never),
    ).toBe("added.ts");
  });
});

// A small unified diff: one context line, one deleted line, one inserted line.
const SAMPLE_DIFF = `diff --git a/file.txt b/file.txt
index 0000000..1111111 100644
--- a/file.txt
+++ b/file.txt
@@ -1,2 +1,2 @@
 unchanged
-old line
+new line
`;

describe("indexFile", () => {
  it("anchors normal, insert and delete changes to the right side/line", () => {
    const [file] = parseDiff(SAMPLE_DIFF);
    const { metaByKey, keyByAnchor } = indexFile(file);

    // The context (normal) line is anchored on both sides at line 1.
    expect(keyByAnchor.get("RIGHT:1")).toBeDefined();
    expect(keyByAnchor.get("LEFT:1")).toBeDefined();

    // Inserted line lands on the RIGHT at line 2.
    const insertKey = keyByAnchor.get("RIGHT:2");
    expect(insertKey).toBeDefined();
    expect(metaByKey.get(insertKey!)).toMatchObject({ side: "RIGHT", line: 2 });
    expect(metaByKey.get(insertKey!)!.lineText).toBe("+new line");

    // Deleted line lands on the LEFT at line 2.
    const deleteKey = keyByAnchor.get("LEFT:2");
    expect(deleteKey).toBeDefined();
    expect(metaByKey.get(deleteKey!)).toMatchObject({ side: "LEFT", line: 2 });
    expect(metaByKey.get(deleteKey!)!.lineText).toBe("-old line");
  });

  it("records the containing hunk header on each change", () => {
    const [file] = parseDiff(SAMPLE_DIFF);
    const { metaByKey } = indexFile(file);
    for (const meta of metaByKey.values()) {
      expect(meta.hunk).toContain("@@");
    }
  });

  it("produces keys consistent with changeKeyOf", () => {
    const [file] = parseDiff(SAMPLE_DIFF);
    const { metaByKey } = indexFile(file);
    const firstChange = file.hunks[0].changes[0];
    expect(metaByKey.has(changeKeyOf(firstChange))).toBe(true);
  });
});

describe("countChanges", () => {
  it("counts inserts and deletes, ignoring context lines", () => {
    const [file] = parseDiff(SAMPLE_DIFF);
    expect(countChanges(file)).toEqual({ add: 1, del: 1 });
  });

  it("returns zero for a binary file with no hunks", () => {
    const [file] = parseDiff(
      `diff --git a/logo.png b/logo.png
index 0000000..1111111 100644
Binary files a/logo.png and b/logo.png differ
`,
    );
    expect(countChanges(file)).toEqual({ add: 0, del: 0 });
  });
});

const CONTEXT_DIFF = `diff --git a/f.ts b/f.ts
index 0000000..1111111 100644
--- a/f.ts
+++ b/f.ts
@@ -1,7 +1,7 @@
 line1
 line2
 line3
-old4
+new4
 line5
 line6
 line7
`;

describe("hunkContextSnippet", () => {
  it("pads the selection with surrounding context lines", () => {
    const [file] = parseDiff(CONTEXT_DIFF);
    // Comment on the inserted line (RIGHT, new line 4), one line of context.
    const lines = hunkContextSnippet(file.hunks[0], "RIGHT", 4, 4, 1).split("\n");
    expect(lines[0]).toContain("@@ -1,7 +1,7 @@");
    expect(lines.slice(1)).toEqual(["-old4", "+new4", " line5"]);
  });

  it("does not reach beyond the requested context window", () => {
    const [file] = parseDiff(CONTEXT_DIFF);
    const snippet = hunkContextSnippet(file.hunks[0], "RIGHT", 4, 4, 1);
    expect(snippet).not.toContain("line2");
    expect(snippet).not.toContain("line7");
  });

  it("clamps the window to the hunk bounds", () => {
    const [file] = parseDiff(CONTEXT_DIFF);
    const snippet = hunkContextSnippet(file.hunks[0], "RIGHT", 4, 4, 99);
    expect(snippet).toContain(" line1");
    expect(snippet).toContain(" line7");
  });

  it("falls back to the bare header when the selection isn't in the hunk", () => {
    const [file] = parseDiff(CONTEXT_DIFF);
    const snippet = hunkContextSnippet(file.hunks[0], "RIGHT", 999, 999, 3);
    expect(snippet).toContain("@@ -1,7 +1,7 @@");
    expect(snippet.split("\n")).toHaveLength(1);
  });
});

// Two hunks (changes at old lines 2 and 18) with a 9-line gap (old lines 6–14)
// collapsed between them. The base (LEFT) source is the full 20-line file, which
// is what context expansion slices.
const TWO_HUNK_DIFF = `diff --git a/f.txt b/f.txt
index e1fc989..368d3c3 100644
--- a/f.txt
+++ b/f.txt
@@ -1,5 +1,5 @@
 b01
-b02
+n02
 b03
 b04
 b05
@@ -15,6 +15,6 @@ b14
 b15
 b16
 b17
-b18
+n18
 b19
 b20
`;

const BASE_SOURCE = Array.from({ length: 20 }, (_, i) =>
  `b${String(i + 1).padStart(2, "0")}`,
).join("\n");

describe("context expansion anchoring", () => {
  it("makes revealed gap lines clickable and consistently anchored", () => {
    const [file] = parseDiff(TWO_HUNK_DIFF);
    const [h1, h2] = file.hunks;

    // Gap lives in OLD/LEFT coordinates between the two hunks.
    const collapsed = getCollapsedLinesCountBetween(h1, h2);
    expect(collapsed).toBe(9);
    const gapStart = h1.oldStart + h1.oldLines; // 6
    const gapEnd = gapStart + collapsed; // 15

    const expanded = expandFromRawCode(file.hunks, BASE_SOURCE, gapStart, gapEnd);
    const { metaByKey, keyByAnchor } = indexFile({ ...file, hunks: expanded });

    // The first revealed line is old line 6 = "b06"; here new line number is also
    // 6 (the upstream change is a balanced insert/delete that keeps counts level).
    const rightKey = keyByAnchor.get("RIGHT:6");
    const leftKey = keyByAnchor.get("LEFT:6");
    expect(rightKey).toBeDefined();
    expect(leftKey).toBeDefined();
    expect(rightKey).toBe(leftKey);

    const meta = metaByKey.get(rightKey!);
    expect(meta).toMatchObject({ side: "RIGHT", line: 6 });
    expect(meta!.lineText).toBe(" b06");

    // The whole gap is now addressable, not just its first line.
    expect(keyByAnchor.get("RIGHT:14")).toBeDefined();
  });

  it("partial expansion only reveals N lines from the top of the gap", () => {
    const [file] = parseDiff(TWO_HUNK_DIFF);
    const [h1] = file.hunks;
    const gapStart = h1.oldStart + h1.oldLines; // 6
    const expanded = expandFromRawCode(file.hunks, BASE_SOURCE, gapStart, gapStart + 3);
    const { keyByAnchor } = indexFile({ ...file, hunks: expanded });

    // First 3 gap lines revealed (6,7,8), the rest still collapsed.
    expect(keyByAnchor.get("RIGHT:6")).toBeDefined();
    expect(keyByAnchor.get("RIGHT:8")).toBeDefined();
    expect(keyByAnchor.get("RIGHT:9")).toBeUndefined();
  });
});
