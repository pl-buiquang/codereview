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
  const legacy = (o: unknown) => ({ json: line(o), norm: null });
  const norm = (...events: unknown[]) => ({ json: "{}", norm: line({ v: 1, events }) });

  it("pairs tool calls with their results and ends with the result (legacy rows)", () => {
    const items = buildTranscript([
      legacy({ type: "system", subtype: "init" }),
      legacy({
        type: "assistant",
        message: {
          content: [
            { type: "text", text: "Checking CI" },
            { type: "tool_use", id: "t1", name: "Bash", input: { command: "gh pr checks 1" } },
          ],
        },
      }),
      legacy({
        type: "user",
        message: { content: [{ type: "tool_result", tool_use_id: "t1", content: "all green", is_error: false }] },
      }),
      legacy({ type: "result", is_error: false, result: "Done", total_cost_usd: 0.12 }),
    ]);
    expect(items).toHaveLength(3);
    expect(items[0]).toEqual({ kind: "text", text: "Checking CI" });
    expect(items[1]).toMatchObject({ kind: "tool", name: "Bash", result: "all green", isError: false });
    expect(items[2]).toEqual({ kind: "result", text: "Done", isError: false, cost: 0.12 });
  });

  it("skips unparseable lines and empty text blocks", () => {
    const items = buildTranscript([
      { json: "not json", norm: null },
      legacy({ type: "assistant", message: { content: [{ type: "text", text: "  " }] } }),
    ]);
    expect(items).toEqual([]);
  });

  it("builds the same transcript from normalized events as from the legacy lines", () => {
    const fromNorm = buildTranscript([
      norm({ type: "other", kind: "system", raw: {} }),
      norm(
        { type: "text", text: "Checking CI" },
        { type: "tool_call", id: "t1", name: "Bash", input: { command: "gh pr checks 1" } },
      ),
      norm({ type: "tool_result", id: "t1", ok: true, text: "all green" }),
      norm({ type: "final", ok: true, text: "Done", usage: { num_turns: 1, input_tokens: 1, output_tokens: 1, cost_usd: 0.12 } }),
    ]);
    const fromLegacy = buildTranscript([
      legacy({ type: "system" }),
      legacy({
        type: "assistant",
        message: {
          content: [
            { type: "text", text: "Checking CI" },
            { type: "tool_use", id: "t1", name: "Bash", input: { command: "gh pr checks 1" } },
          ],
        },
      }),
      legacy({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "t1", content: "all green" }] } }),
      legacy({ type: "result", is_error: false, result: "Done", total_cost_usd: 0.12 }),
    ]);
    expect(fromNorm).toEqual(fromLegacy);
  });

  it("shows an opencode run as its text and tool items, without an empty result item", () => {
    const usage = { num_turns: 1, input_tokens: 1, output_tokens: 1, cost_usd: 0.001 };
    const items = buildTranscript([
      norm({ type: "other", kind: "step_start", raw: {} }),
      norm({ type: "text", text: "Reading it" }),
      norm({ type: "tool_call", id: "t1", name: "read", input: { path: "note.txt" } }, { type: "tool_result", id: "t1", ok: true, text: "hello" }),
      norm({ type: "text", text: "It says hello." }),
      norm({ type: "final", ok: true, text: "", usage }),
    ]);
    expect(items.map((i) => i.kind)).toEqual(["text", "tool", "text"]);
    expect(items[1]).toMatchObject({ name: "read", result: "hello" });
    // a failed run still shows its error
    const failed = buildTranscript([norm({ type: "final", ok: false, text: "Model unavailable", usage })]);
    expect(failed).toEqual([{ kind: "result", text: "Model unavailable", isError: true, cost: 0.001 }]);
  });

  it("renders a failing tool result and an error final from normalized events", () => {
    const items = buildTranscript([
      norm({ type: "tool_call", id: "t1", name: "Read", input: {} }),
      norm({ type: "tool_result", id: "t1", ok: false, text: "nope" }),
      norm({ type: "final", ok: false, text: "boom", usage: { num_turns: 0, input_tokens: 0, output_tokens: 0, cost_usd: 0 } }),
    ]);
    expect(items[0]).toMatchObject({ kind: "tool", result: "nope", isError: true });
    expect(items[1]).toEqual({ kind: "result", text: "boom", isError: true, cost: null });
  });

  it("mixes legacy and normalized rows in one run and ignores a norm of an unknown version", () => {
    const items = buildTranscript([
      legacy({ type: "assistant", message: { content: [{ type: "text", text: "old" }] } }),
      norm({ type: "text", text: "new" }),
      { json: line({ type: "assistant", message: { content: [{ type: "text", text: "fallback" }] } }), norm: line({ v: 2, events: [] }) },
      { json: "{}", norm: "{broken" },
    ]);
    expect(items.map((i) => (i.kind === "text" ? i.text : i.kind))).toEqual(["old", "new", "fallback"]);
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
