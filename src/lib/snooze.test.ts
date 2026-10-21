import { describe, expect, it } from "vitest";
import { DEFAULT_SNOOZE_OPTIONS, isValidSnoozeOptions, parseSnoozeOptions } from "./snooze";

describe("snooze options", () => {
  it("accepts one line of comma-separated durations", () => {
    for (const ok of ["2h", "2h, 1d, 1w", " 30h,2d ,3w "]) expect(isValidSnoozeOptions(ok)).toBe(true);
    for (const bad of ["", "2", "h", "2x", "2h,", "2h 1d", "1.5h", "2h\n1d"]) expect(isValidSnoozeOptions(bad)).toBe(false);
  });

  it("parses durations to milliseconds", () => {
    expect(parseSnoozeOptions("2h, 1d, 1w")).toEqual([
      { label: "2h", ms: 7_200_000 },
      { label: "1d", ms: 86_400_000 },
      { label: "1w", ms: 604_800_000 },
    ]);
  });

  it("falls back to the defaults when invalid", () => {
    expect(parseSnoozeOptions("nope")).toEqual(parseSnoozeOptions(DEFAULT_SNOOZE_OPTIONS));
  });
});
