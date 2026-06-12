import { describe, it, expect } from "vitest";
import {
  expandFromRawCode,
  getCollapsedLinesCountBetween,
  parseDiff,
  type ChangeData,
  type HunkData,
  type TokenNode,
} from "react-diff-view";
import {
  anchorByLine,
  languageForPath,
  fileDisplayPath,
  indexFile,
  changeKeyOf,
  countChanges,
  hunkContextSnippet,
  leadingExpandRange,
  rightLinesText,
  sourceLineCount,
  suggestionFence,
  tokenizeFile,
  trailingExpandRange,
  trailingGap,
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

describe("anchorByLine", () => {
  // Maps the same anchors indexFile produces for SAMPLE_DIFF.
  const keyByAnchor = new Map<string, string>([
    ["RIGHT:1", "right-1"],
    ["LEFT:1", "left-1"],
    ["RIGHT:2", "right-2"],
    ["LEFT:2", "left-2"],
  ]);
  type Item = { id: string; side: string | null; line: number | null };
  const sideLine = (t: Item) => ({ side: t.side ?? "", line: t.line });

  it("anchors items by SIDE:line and groups multiple onto one key", () => {
    const items: Item[] = [
      { id: "a", side: "RIGHT", line: 2 },
      { id: "b", side: "RIGHT", line: 2 },
      { id: "c", side: "LEFT", line: 1 },
    ];
    const { byKey, orphans } = anchorByLine(items, sideLine, keyByAnchor);
    expect(orphans).toEqual([]);
    expect(byKey.get("right-2")?.map((i) => i.id)).toEqual(["a", "b"]);
    expect(byKey.get("left-1")?.map((i) => i.id)).toEqual(["c"]);
  });

  it("distinguishes LEFT and RIGHT for the same line number", () => {
    const items: Item[] = [
      { id: "r", side: "RIGHT", line: 1 },
      { id: "l", side: "LEFT", line: 1 },
    ];
    const { byKey } = anchorByLine(items, sideLine, keyByAnchor);
    expect(byKey.get("right-1")?.map((i) => i.id)).toEqual(["r"]);
    expect(byKey.get("left-1")?.map((i) => i.id)).toEqual(["l"]);
  });

  it("orphans items with a null line or a line not in the diff", () => {
    const items: Item[] = [
      { id: "nullline", side: "RIGHT", line: null },
      { id: "missing", side: "RIGHT", line: 99 },
      { id: "nullside", side: null, line: 1 },
    ];
    const { byKey, orphans } = anchorByLine(items, sideLine, keyByAnchor);
    expect(byKey.size).toBe(0);
    expect(orphans.map((i) => i.id)).toEqual(["nullline", "missing", "nullside"]);
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

// HunkData literal builder mirroring buildFullFileFile's change shape, so tests
// can place specific normal/insert/delete changes at chosen line numbers.
function normal(n: number, content: string): ChangeData {
  return { type: "normal", isNormal: true, content, oldLineNumber: n, newLineNumber: n };
}
function insert(n: number, content: string): ChangeData {
  return { type: "insert", isInsert: true, content, lineNumber: n };
}
function del(n: number, content: string): ChangeData {
  return { type: "delete", isDelete: true, content, lineNumber: n };
}
function hunkOf(changes: ChangeData[]): HunkData {
  return {
    content: "@@ -1,1 +1,1 @@",
    oldStart: 1,
    newStart: 1,
    oldLines: changes.length,
    newLines: changes.length,
    changes,
  };
}

describe("rightLinesText", () => {
  it("returns a single normal line's content without a diff sign", () => {
    const hunks = [hunkOf([normal(1, "a"), normal(2, "  const x = 1;"), normal(3, "c")])];
    expect(rightLinesText(hunks, 2, 2)).toEqual(["  const x = 1;"]);
  });

  it("resolves insert lines via their lineNumber", () => {
    const hunks = [hunkOf([normal(1, "a"), insert(2, "new line")])];
    expect(rightLinesText(hunks, 2, 2)).toEqual(["new line"]);
  });

  it("returns a range spanning insert + normal in lo→hi order", () => {
    const hunks = [hunkOf([insert(1, "first"), normal(2, "second"), insert(3, "third")])];
    expect(rightLinesText(hunks, 1, 3)).toEqual(["first", "second", "third"]);
  });

  it("returns null for a line in a collapsed gap", () => {
    const hunks = [hunkOf([normal(1, "a"), normal(2, "b")])];
    expect(rightLinesText(hunks, 5, 5)).toBeNull();
  });

  it("returns null when a range is only partially present", () => {
    const hunks = [hunkOf([normal(1, "a"), normal(2, "b")])];
    expect(rightLinesText(hunks, 2, 3)).toBeNull();
  });

  it("ignores delete changes (no RIGHT presence)", () => {
    const hunks = [hunkOf([normal(1, "a"), del(2, "gone"), normal(2, "b")])];
    // Delete at "line 2" must not satisfy a RIGHT request for line 2…
    expect(rightLinesText(hunks, 2, 2)).toEqual(["b"]);
    // …and a RIGHT line that only a delete occupies is unresolvable.
    const onlyDelete = [hunkOf([del(2, "gone")])];
    expect(rightLinesText(onlyDelete, 2, 2)).toBeNull();
  });
});

describe("suggestionFence", () => {
  it("wraps lines in a basic 3-backtick suggestion fence", () => {
    expect(suggestionFence(["let x = 1;"])).toBe("```suggestion\nlet x = 1;\n```");
    expect(suggestionFence(["a", "b"])).toBe("```suggestion\na\nb\n```");
  });

  it("grows the fence past a triple-backtick run inside the content", () => {
    expect(suggestionFence(["before ``` after"])).toBe(
      "````suggestion\nbefore ``` after\n````",
    );
  });
});

// Collect every token node type reachable from a side's per-line token trees.
function tokenTypes(linesOfTrees: TokenNode[][]): Set<string> {
  const types = new Set<string>();
  const walk = (node: TokenNode) => {
    types.add(node.type);
    for (const child of node.children ?? []) walk(child);
  };
  for (const line of linesOfTrees) for (const node of line) walk(node);
  return types;
}

// A modified line pair in a TypeScript file (languageForPath resolves "f.ts").
const MODIFIED_TS_DIFF = `diff --git a/f.ts b/f.ts
index 0000000..1111111 100644
--- a/f.ts
+++ b/f.ts
@@ -1,1 +1,1 @@
-const a = 1;
+const a = 2;
`;

// Same modified pair, but in a file with no registered language.
const MODIFIED_UNKNOWN_DIFF = `diff --git a/notes.unknownext b/notes.unknownext
index 0000000..1111111 100644
--- a/notes.unknownext
+++ b/notes.unknownext
@@ -1,1 +1,1 @@
-const a = 1;
+const a = 2;
`;

// A pure insertion (no paired delete) into a TypeScript file.
const INSERT_ONLY_TS_DIFF = `diff --git a/f.ts b/f.ts
index 0000000..1111111 100644
--- a/f.ts
+++ b/f.ts
@@ -1,1 +1,2 @@
 const a = 1;
+const b = 2;
`;

describe("tokenizeFile", () => {
  it("marks intra-line edits on a modified line pair", () => {
    const [file] = parseDiff(MODIFIED_TS_DIFF);
    const tokens = tokenizeFile(file);
    expect(tokens).toBeDefined();
    expect(tokenTypes(tokens!.new)).toContain("edit");
    expect(tokenTypes(tokens!.old)).toContain("edit");
  });

  it("keeps syntax highlight tokens alongside edit marks", () => {
    const [file] = parseDiff(MODIFIED_TS_DIFF);
    const tokens = tokenizeFile(file);
    expect(tokens).toBeDefined();
    const types = tokenTypes(tokens!.new);
    const refractorTypes = [...types].filter((t) => t !== "text" && t !== "edit");
    expect(refractorTypes.length).toBeGreaterThan(0);
  });

  it("skips markEdits above the changed-line threshold", () => {
    const [file] = parseDiff(MODIFIED_TS_DIFF); // add + del = 2
    const tokens = tokenizeFile(file, { markEditsMaxChanges: 1 });
    expect(tokens).toBeDefined();
    expect(tokenTypes(tokens!.new)).not.toContain("edit");
    expect(tokenTypes(tokens!.old)).not.toContain("edit");
  });

  it("marks edits in files without a registered language", () => {
    const [file] = parseDiff(MODIFIED_UNKNOWN_DIFF);
    const tokens = tokenizeFile(file);
    expect(tokens).toBeDefined();
    expect(tokenTypes(tokens!.new)).toContain("edit");
  });

  it("returns undefined when neither language nor edits apply", () => {
    const [file] = parseDiff(MODIFIED_UNKNOWN_DIFF);
    expect(tokenizeFile(file, { markEditsMaxChanges: 0 })).toBeUndefined();
  });

  it("adds no edit marks for pure insert/delete blocks", () => {
    const [file] = parseDiff(INSERT_ONLY_TS_DIFF);
    const tokens = tokenizeFile(file);
    expect(tokens).toBeDefined();
    expect(tokenTypes(tokens!.new)).not.toContain("edit");
    expect(tokenTypes(tokens!.old)).not.toContain("edit");
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

  // One hunk in the middle of the 20-line BASE_SOURCE: hidden lines both above
  // (old 1–5) and below (old 9–20), exercising the edge expanders.
  const EDGE_DIFF = `diff --git a/f.txt b/f.txt
index e1fc989..368d3c3 100644
--- a/f.txt
+++ b/f.txt
@@ -6,3 +6,3 @@ b05
 b06
-b07
+n07
 b08
`;

  describe("sourceLineCount", () => {
    it("ignores the trailing-newline empty element", () => {
      expect(sourceLineCount("a\nb\n")).toBe(2);
      expect(sourceLineCount("a\nb")).toBe(2);
      expect(sourceLineCount("")).toBe(0);
      expect(sourceLineCount("\n")).toBe(1);
    });
  });

  describe("trailingGap", () => {
    const lastHunk = { oldStart: 10, oldLines: 5 } as never;

    it("counts hidden old-side lines below the last hunk", () => {
      expect(trailingGap(lastHunk, 30)).toBe(16); // 30 - 14
      expect(trailingGap(lastHunk, 14)).toBe(0); // hunk reaches EOF
    });

    it("clamps to zero, never negative", () => {
      expect(trailingGap(lastHunk, 10)).toBe(0);
      expect(trailingGap(lastHunk, 0)).toBe(0);
    });
  });

  describe("leadingExpandRange", () => {
    it("reveals the n lines adjacent to the first hunk", () => {
      expect(leadingExpandRange({ oldStart: 100 } as never, 20)).toEqual([80, 100]);
    });

    it("clamps to the top of the file", () => {
      expect(leadingExpandRange({ oldStart: 10 } as never, 20)).toEqual([1, 10]);
      expect(
        leadingExpandRange({ oldStart: 10 } as never, Number.POSITIVE_INFINITY),
      ).toEqual([1, 10]);
    });

    it("yields an empty range when there is no leading gap", () => {
      expect(leadingExpandRange({ oldStart: 1 } as never, 20)).toEqual([1, 1]);
    });
  });

  describe("trailingExpandRange", () => {
    const lastHunk = { oldStart: 10, oldLines: 5 } as never;

    it("reveals the n lines just below the last hunk", () => {
      expect(trailingExpandRange(lastHunk, 100, 20)).toEqual([15, 35]);
    });

    it("Infinity clamps to EOF (end exclusive reaches the last line)", () => {
      expect(trailingExpandRange(lastHunk, 100, Number.POSITIVE_INFINITY)).toEqual([
        15, 101,
      ]);
    });

    it("yields an empty range when the hunk already reaches EOF", () => {
      expect(trailingExpandRange(lastHunk, 14, 20)).toEqual([15, 15]);
    });
  });

  it("leading expansion reveals line 1 and keeps revealed lines commentable", () => {
    const [file] = parseDiff(EDGE_DIFF);
    const first = file.hunks[0];
    expect(first.oldStart).toBe(6); // fixture's first hunk starts past line 1

    const [start, end] = leadingExpandRange(first, Number.POSITIVE_INFINITY);
    const expanded = expandFromRawCode(file.hunks, BASE_SOURCE, start, end);

    // Old line 1 is now a normal change with matching old/new numbers (the
    // hunk's insert/delete pair is balanced, so numbering stays level).
    const lineOne = expanded[0].changes[0];
    expect(lineOne).toMatchObject({
      type: "normal",
      oldLineNumber: 1,
      newLineNumber: 1,
      content: "b01",
    });

    // Revealed lines stay commentable: indexFile yields a RIGHT anchor for them.
    const { metaByKey, keyByAnchor } = indexFile({ ...file, hunks: expanded });
    const key = keyByAnchor.get("RIGHT:1");
    expect(key).toBeDefined();
    expect(metaByKey.get(key!)).toMatchObject({ side: "RIGHT", line: 1 });
  });

  it("trailing expansion reaches the final source line with no phantom EOF line", () => {
    const [file] = parseDiff(EDGE_DIFF);
    const last = file.hunks[file.hunks.length - 1];

    const [start, end] = trailingExpandRange(
      last,
      sourceLineCount(BASE_SOURCE),
      Number.POSITIVE_INFINITY,
    );
    expect([start, end]).toEqual([9, 21]);
    const expanded = expandFromRawCode(file.hunks, BASE_SOURCE, start, end);

    const lastHunk = expanded[expanded.length - 1];
    const finalChange = lastHunk.changes[lastHunk.changes.length - 1];
    expect(finalChange).toMatchObject({
      type: "normal",
      oldLineNumber: 20,
      content: "b20",
    });

    // No phantom empty line appended past EOF.
    const { keyByAnchor } = indexFile({ ...file, hunks: expanded });
    expect(keyByAnchor.get("LEFT:20")).toBeDefined();
    expect(keyByAnchor.get("LEFT:21")).toBeUndefined();
  });
});
