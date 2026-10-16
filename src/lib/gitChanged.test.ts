import { describe, expect, it } from "vitest";
import { gitChangedQueryKeys } from "./gitChanged";

describe("gitChangedQueryKeys", () => {
  it("staging only reloads the working-tree status", () => {
    expect(gitChangedQueryKeys({ repo_id: 4, kinds: ["index"] })).toEqual([["wip", 4]]);
  });

  it("merges kinds without duplicates, scoped to the repo", () => {
    const keys = gitChangedQueryKeys({ repo_id: 4, kinds: ["refs", "fetch"] });
    expect(keys).toEqual([
      ["refs", 4],
      ["graph", 4],
      ["branches", 4],
      ["review-live", 4],
      ["repo-sync", 4],
    ]);
  });

  it("a checkout also refreshes worktrees", () => {
    expect(gitChangedQueryKeys({ repo_id: 1, kinds: ["head"] })).toContainEqual(["worktrees", 1]);
  });
});
