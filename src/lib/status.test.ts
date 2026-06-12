import { describe, it, expect } from "vitest";

import { statusLabel, statusBadgeClass } from "./status";

describe("statusLabel", () => {
  it("passes draft and published through unchanged", () => {
    expect(statusLabel("draft")).toBe("draft");
    expect(statusLabel("published")).toBe("published");
  });

  it("renders a friendly label for the pending status", () => {
    expect(statusLabel("published_pending")).toBe("pending on GitHub");
  });
});

describe("statusBadgeClass", () => {
  it("maps each status to its badge modifier", () => {
    expect(statusBadgeClass("draft")).toBe("badge-draft");
    expect(statusBadgeClass("published_pending")).toBe("badge-pending");
    expect(statusBadgeClass("published")).toBe("badge-pr");
  });
});
