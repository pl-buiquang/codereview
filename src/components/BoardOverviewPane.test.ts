import { describe, expect, it } from "vitest";
import type { BoardCard } from "../lib/types";
import { linkCardRefs } from "./BoardOverviewPane";

const card = (id: string, repo: string, number: number) => ({ id, repo, number }) as BoardCard;
const cards = [card("A", "acme/back", 12), card("B", "acme/front", 7)];

describe("linkCardRefs", () => {
  it("links short and full repo refs to tracked cards", () => {
    expect(linkCardRefs("fix back#12 and acme/front#7", cards)).toBe(
      "fix [back#12](#card=A) and [acme/front#7](#card=B)",
    );
  });

  it("leaves unknown refs and URLs alone", () => {
    expect(linkCardRefs("back#99 https://x.io/acme/back#12", cards)).toBe(
      "back#99 https://x.io/acme/back#12",
    );
  });
});
