import { describe, it, expect } from "vitest";
import { parseDiff } from "react-diff-view";
import {
  languageForPath,
  fileDisplayPath,
  indexFile,
  changeKeyOf,
  countChanges,
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
