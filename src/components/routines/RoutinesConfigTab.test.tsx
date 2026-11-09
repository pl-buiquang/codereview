import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { AiProfile, Routine } from "../../lib/types";

const routineList = vi.fn();
const routineSave = vi.fn();
const aiProfileList = vi.fn();
vi.mock("../../lib/api", () => ({
  api: {
    routineList: (...a: unknown[]) => routineList(...a),
    routineSave: (...a: unknown[]) => routineSave(...a),
    aiProfileList: (...a: unknown[]) => aiProfileList(...a),
    routineMaxConcurrency: () => Promise.resolve(4),
    routineSettings: () => Promise.resolve({ plans_dir: "", notes_root: "" }),
    routineSetSettings: () => Promise.resolve(),
    routineSetMaxConcurrency: () => Promise.resolve(),
    routineDelete: () => Promise.resolve(),
    routineReset: () => Promise.resolve(),
  },
}));

import { RoutinesConfigTab } from "./RoutinesConfigTab";

const routine = (over: Partial<Routine> = {}): Routine => ({
  id: 1,
  key: "babysit",
  name: "Babysit",
  description: "Keep CI green",
  context_type: "pr",
  prompt_template: "/goal x",
  system_prompt: "be careful",
  kind: "plain",
  needs_worktree: true,
  access: "full",
  harness_overrides: { claude: { permission_mode: "bypassPermissions", allowed_tools: [], extra_args: [], system_prompt_mode: "append" } },
  profile_id: null,
  model: "",
  timeout_min: 240,
  builtin: true,
  handler: null,
  enabled: true,
  ...over,
});

const system = routine({
  id: 2,
  key: "focus",
  name: "Focus",
  description: "Triage a review's hunks into focus tiers.",
  kind: "system",
  handler: "focus",
  access: "read_only",
  timeout_min: 30,
});

const profile = (over: Partial<AiProfile> = {}): AiProfile => ({
  id: 1,
  name: "Claude Code",
  harness: "claude",
  model: "",
  settings: {},
  host_id: null,
  is_default: true,
  ...over,
});

function renderTab() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <RoutinesConfigTab />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  routineList.mockReset().mockResolvedValue([routine(), routine({ id: 3, key: "mine", name: "Mine", builtin: false }), system]);
  routineSave.mockReset().mockImplementation((r) => Promise.resolve({ ...routine(), ...r }));
  aiProfileList.mockReset().mockResolvedValue([profile(), profile({ id: 2, name: "Fast", model: "haiku", is_default: false })]);
});

describe("RoutinesConfigTab", () => {
  it("lists plain routines apart from read-only system routines", async () => {
    renderTab();
    await screen.findByRole("heading", { name: "System routines" });
    const card = await screen.findByTestId("system-routine-focus");
    expect(within(card).getByText("Triage a review's hunks into focus tiers.")).toBeTruthy();
    // System routines are not in the editable list, and the plain ones are.
    expect(screen.getAllByText("Babysit").length).toBeGreaterThan(0);
    expect(screen.getByText("Mine")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Focus/ })).toBeNull();
  });

  it("edits a system routine through its four fields only", async () => {
    const user = userEvent.setup();
    renderTab();
    const card = await screen.findByTestId("system-routine-focus");
    const save = within(card).getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    // Four controls and nothing for prompts, tools or access.
    expect(within(card).getAllByRole("combobox")).toHaveLength(1);
    expect(within(card).getAllByRole("textbox")).toHaveLength(1);
    expect(within(card).getAllByRole("spinbutton")).toHaveLength(1);
    expect(within(card).getAllByRole("checkbox")).toHaveLength(1);

    await user.selectOptions(within(card).getByRole("combobox"), "Fast");
    await user.type(within(card).getByRole("textbox"), "sonnet");
    await user.click(within(card).getByRole("checkbox"));
    expect(save.disabled).toBe(false);
    await user.click(save);

    await waitFor(() => expect(routineSave).toHaveBeenCalled());
    expect(routineSave.mock.calls[0][0]).toMatchObject({ id: 2, profile_id: 2, model: "sonnet", enabled: false, timeout_min: 30 });
  });

  it("shows the Claude Code options only when the resolved harness is Claude", async () => {
    const user = userEvent.setup();
    aiProfileList.mockResolvedValue([profile(), profile({ id: 2, name: "Codex", harness: "codex", is_default: false })]);
    renderTab();
    await screen.findByText("Claude Code options");
    expect(screen.getByLabelText(/Permission mode/)).toBeTruthy();
    // The neutral access selector is always there.
    const form = within(document.querySelector("form")!);
    expect(form.getByLabelText("Access")).toBeTruthy();

    await user.selectOptions(form.getByLabelText("Profile"), "Codex");
    expect(screen.queryByText("Claude Code options")).toBeNull();
    expect(form.getByLabelText("Access")).toBeTruthy();
  });

  it("saves the Claude options into harness_overrides", async () => {
    const user = userEvent.setup();
    renderTab();
    await screen.findByText("Claude Code options");
    const tools = screen.getByLabelText(/Allowed tools/);
    await user.type(tools, "Read");
    await user.click(within(document.querySelector("form")!).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(routineSave).toHaveBeenCalled());
    const sent = routineSave.mock.calls[0][0];
    expect(sent.access).toBe("full");
    expect(sent.harness_overrides.claude).toMatchObject({ permission_mode: "bypassPermissions", allowed_tools: ["Read"] });
    expect(sent).not.toHaveProperty("permission_mode");
  });
});
