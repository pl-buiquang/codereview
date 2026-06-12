import { describe, it, expect } from "vitest";
import { isEditableTarget, pickThread, moveCursorKey } from "./keyboard";

describe("isEditableTarget", () => {
  it("is true for editable fields and elements inside [contenteditable]", () => {
    const textarea = document.createElement("textarea");
    const input = document.createElement("input");
    const select = document.createElement("select");
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    const inner = document.createElement("span");
    editable.appendChild(inner);

    expect(isEditableTarget(textarea)).toBe(true);
    expect(isEditableTarget(input)).toBe(true);
    expect(isEditableTarget(select)).toBe(true);
    expect(isEditableTarget(inner)).toBe(true);
  });

  it("is false for non-editable elements and null", () => {
    const td = document.createElement("td");
    const button = document.createElement("button");

    expect(isEditableTarget(td)).toBe(false);
    expect(isEditableTarget(button)).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
  });
});

describe("pickThread", () => {
  it("picks the next one below, skipping the one we're on", () => {
    expect(pickThread([100, 400, 900], 100, 1)).toBe(1);
  });

  it("picks the previous one above", () => {
    expect(pickThread([100, 400, 900], 900, -1)).toBe(1);
  });

  it("returns null at the ends and for empty tops", () => {
    expect(pickThread([100, 400, 900], 900, 1)).toBeNull();
    expect(pickThread([100, 400, 900], 100, -1)).toBeNull();
    expect(pickThread([], 0, 1)).toBeNull();
  });

  it("treats a thread within eps of the current offset as the current one", () => {
    // current 396, eps 8: 400 is within eps so it is "current"; next is 900.
    expect(pickThread([100, 400, 900], 396, 1, 8)).toBe(2);
  });
});

describe("moveCursorKey", () => {
  const keys = ["a", "b", "c"];

  it("starts at keys[0] when current is null", () => {
    expect(moveCursorKey(keys, null, 1)).toBe("a");
    expect(moveCursorKey(keys, null, -1)).toBe("a");
  });

  it("clamps at both ends without wrapping", () => {
    expect(moveCursorKey(keys, "c", 1)).toBe("c");
    expect(moveCursorKey(keys, "a", -1)).toBe("a");
  });

  it("restarts at keys[0] for an unknown current, and returns null for empty keys", () => {
    expect(moveCursorKey(keys, "z", 1)).toBe("a");
    expect(moveCursorKey([], null, 1)).toBeNull();
    expect(moveCursorKey([], "a", -1)).toBeNull();
  });
});
