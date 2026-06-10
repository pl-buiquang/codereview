import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { timeAgo } from "./timeAgo";

const NOW = new Date("2026-06-10T12:00:00Z");

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("timeAgo", () => {
  it("accepts epoch ms", () => {
    expect(timeAgo(Date.now() - 5 * 60_000)).toBe("5m ago");
  });

  it("still accepts ISO strings", () => {
    expect(timeAgo(new Date(NOW.getTime() - 2 * 60 * 60_000).toISOString())).toBe("2h ago");
  });

  it("returns empty string for garbage input", () => {
    expect(timeAgo("nope")).toBe("");
  });

  it("says 'just now' for sub-minute ages", () => {
    expect(timeAgo(Date.now() - 10_000)).toBe("just now");
  });
});
