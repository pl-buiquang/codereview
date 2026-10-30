import { describe, expect, it } from "vitest";
import { monitorSummary } from "./sync";
import type { MonitorAction } from "./types";

const action = (kind: MonitorAction["kind"], number: number, detail = ""): MonitorAction => ({
  card_id: `c${number}`,
  repo: "o/r",
  number,
  kind,
  detail,
});

describe("monitorSummary", () => {
  it("is null when nothing happened", () => {
    expect(monitorSummary({ actions: [], error: null })).toBeNull();
  });

  it("summarizes updates, reruns and gave-up PRs", () => {
    const s = monitorSummary({
      actions: [action("updated", 1), action("updated", 2), action("reran", 3), action("gave_up", 123)],
      error: null,
    });
    expect(s).toEqual({ text: "Monitor: updated 2 branches · reran 1 CI run · gave up on o/r#123", isError: false, warn: true });
  });

  it("flags per-PR and round errors", () => {
    expect(monitorSummary({ actions: [action("error", 4, "boom")], error: null })).toEqual({
      text: "Monitor: failed: o/r#4 (boom)",
      isError: true,
      warn: false,
    });
    expect(monitorSummary({ actions: [], error: "offline" })?.isError).toBe(true);
  });
});
