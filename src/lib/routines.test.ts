import { describe, expect, it } from "vitest";
import { buildTranscript, cardContextKey, claudeOptions, formatDuration, prContextKey, resolvedHarness, withClaudeOptions } from "./routines";

describe("context keys", () => {
  it("lowercases the repo like the backend does", () => {
    expect(prContextKey("Cardiologs/Back", 12)).toBe("pr:cardiologs/back#12");
    expect(cardContextKey({ type: "issue", repo: "O/N", number: 3 })).toBe("issue:o/n#3");
    expect(cardContextKey({ type: "pr", repo: "O/N", number: 3 })).toBe("pr:o/n#3");
  });
});

describe("formatDuration", () => {
  it("formats seconds, minutes and hours", () => {
    const at = (s: number) => new Date(Date.UTC(2026, 0, 1, 0, 0, s)).toISOString();
    expect(formatDuration(at(0), at(42))).toBe("42s");
    expect(formatDuration(at(0), at(125))).toBe("2m 5s");
    expect(formatDuration(at(0), at(3 * 3600 + 120))).toBe("3h 2m");
    expect(formatDuration(null, null)).toBe("");
  });
});

describe("buildTranscript", () => {
  const line = (o: unknown) => JSON.stringify(o);

  it("pairs tool calls with their results and ends with the result", () => {
    const items = buildTranscript([
      line({ type: "system", subtype: "init" }),
      line({
        type: "assistant",
        message: {
          content: [
            { type: "text", text: "Checking CI" },
            { type: "tool_use", id: "t1", name: "Bash", input: { command: "gh pr checks 1" } },
          ],
        },
      }),
      line({
        type: "user",
        message: { content: [{ type: "tool_result", tool_use_id: "t1", content: "all green", is_error: false }] },
      }),
      line({ type: "result", is_error: false, result: "Done", total_cost_usd: 0.12 }),
    ]);
    expect(items).toHaveLength(3);
    expect(items[0]).toEqual({ kind: "text", text: "Checking CI" });
    expect(items[1]).toMatchObject({ kind: "tool", name: "Bash", result: "all green", isError: false });
    expect(items[2]).toEqual({ kind: "result", text: "Done", isError: false, cost: 0.12 });
  });

  it("skips unparseable lines and empty text blocks", () => {
    const items = buildTranscript([
      "not json",
      line({ type: "assistant", message: { content: [{ type: "text", text: "  " }] } }),
    ]);
    expect(items).toEqual([]);
  });
});

describe("routine harness options", () => {
  it("fills in defaults and keeps other harnesses' entries when patching", () => {
    expect(claudeOptions({})).toEqual({ permission_mode: "", allowed_tools: [], extra_args: [], system_prompt_mode: "append" });
    const patched = withClaudeOptions({ codex: { sandbox: "x" }, claude: { allowed_tools: ["Read"] } }, { extra_args: ["--effort", "low"] });
    expect(patched.codex).toEqual({ sandbox: "x" });
    expect(patched.claude).toEqual({ permission_mode: "", allowed_tools: ["Read"], extra_args: ["--effort", "low"], system_prompt_mode: "append" });
  });

  it("resolves the harness of a profile choice, falling back to the default profile", () => {
    const profiles = [
      { id: 1, harness: "claude", is_default: true },
      { id: 2, harness: "codex", is_default: false },
    ];
    expect(resolvedHarness(2, profiles)).toBe("codex");
    expect(resolvedHarness(null, profiles)).toBe("claude");
    expect(resolvedHarness(99, profiles)).toBe("claude");
    expect(resolvedHarness(null, [])).toBe("claude");
  });
});
