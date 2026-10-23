import { describe, expect, it } from "vitest";
import type { BoardCard, BoardMeta } from "../lib/types";
import { cardIsUntagged } from "./BoardCard";

const meta = (kind: BoardMeta["kind"], tag: string | null = null) => ({ kind, tag }) as BoardMeta;
const card = (own: BoardMeta[], inherited: BoardMeta[] = []) =>
  ({ meta: own, inherited_meta: inherited }) as unknown as BoardCard;

describe("cardIsUntagged", () => {
  it("is true for a card without metadata", () => {
    expect(cardIsUntagged(card([]))).toBe(true);
  });

  it("ignores non-note metadata, even tagged", () => {
    expect(cardIsUntagged(card([meta("link", "wip"), meta("file"), meta("claude_session")]))).toBe(true);
  });

  it("is false when the card carries a note", () => {
    expect(cardIsUntagged(card([meta("note")]))).toBe(false);
  });

  it("is false when the linked issue carries a note", () => {
    expect(cardIsUntagged(card([], [meta("note", "plan")]))).toBe(false);
  });
});
