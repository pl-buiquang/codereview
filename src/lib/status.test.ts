import { describe, it, expect } from "vitest";

import { statusLabel } from "./status";

describe("statusLabel", () => {
  it("passes draft and published through unchanged", () => {
    expect(statusLabel("draft")).toBe("draft");
    expect(statusLabel("published")).toBe("published");
  });

  it("renders a friendly label for the pending status", () => {
    expect(statusLabel("published_pending")).toBe("pending on GitHub");
  });
});
