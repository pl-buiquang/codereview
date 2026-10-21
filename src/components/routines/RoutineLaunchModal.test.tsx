import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Routine, RoutineOption, RoutineRun } from "../../lib/types";

const routineListForContext = vi.fn();
const routineLaunch = vi.fn();
const routineRunStop = vi.fn();
vi.mock("../../lib/api", () => ({
  api: {
    routineListForContext: (...a: unknown[]) => routineListForContext(...a),
    routineLaunch: (...a: unknown[]) => routineLaunch(...a),
    routineRunStop: (...a: unknown[]) => routineRunStop(...a),
  },
}));

import { RoutineLaunchModal } from "./RoutineLaunchModal";
import { useUIStore } from "../../store";
import { useSettingsStore } from "../../lib/settings";

const routine = (over: Partial<Routine> = {}): Routine => ({
  id: 1,
  key: "babysit",
  name: "Babysit",
  description: "Keep CI green",
  context_type: "pr",
  prompt_template: "/goal x",
  system_prompt: "",
  system_prompt_mode: "append",
  needs_worktree: true,
  permission_mode: "bypassPermissions",
  allowed_tools: [],
  extra_args: [],
  model: "",
  timeout_min: 240,
  builtin: true,
  handler: null,
  enabled: true,
  ...over,
});

const run = (over: Partial<RoutineRun> = {}): RoutineRun => ({
  id: 9,
  routine_id: 1,
  routine_key: "babysit",
  routine_name: "Babysit",
  context_type: "pr",
  context_key: "pr:o/n#1",
  context_label: "o/n#1",
  context: { kind: "pr", id: "PR_1", base_paths: [] },
  status: "running",
  note: "",
  model: "",
  session_id: null,
  cwd: null,
  result_text: null,
  error: null,
  cost_usd: 0,
  num_turns: 0,
  queued_at: new Date().toISOString(),
  started_at: new Date().toISOString(),
  ended_at: null,
  ...over,
});

function renderModal(options: RoutineOption[]) {
  routineListForContext.mockResolvedValue(options);
  const onClose = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <RoutineLaunchModal kind="pr" id="PR_1" title="o/n#1" onClose={onClose} />
    </QueryClientProvider>,
  );
  return onClose;
}

describe("RoutineLaunchModal", () => {
  beforeEach(() => {
    useSettingsStore.setState({ aiEnabled: true });
    vi.clearAllMocks();
  });

  it("disables Run with an explanation while AI features are off", async () => {
    useSettingsStore.setState({ aiEnabled: false });
    renderModal([{ routine: routine(), active_run: null }]);
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: /Babysit/ }));
    const runButton = screen.getByRole("button", { name: "Run" });
    expect(runButton).toBeDisabled();
    expect(runButton).toHaveAttribute("title", expect.stringContaining("Settings → AI"));
  });

  it("launches the selected routine with the note", async () => {
    routineLaunch.mockResolvedValue(run({ status: "running" }));
    const onClose = renderModal([{ routine: routine(), active_run: null }]);
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: /Babysit/ }));
    await user.type(screen.getByPlaceholderText(/Extra instructions/), "be careful");
    await user.click(screen.getByRole("button", { name: "Run" }));

    await waitFor(() => expect(routineLaunch).toHaveBeenCalled());
    expect(routineLaunch.mock.calls[0][0]).toBe(1);
    expect(routineLaunch.mock.calls[0][1]).toMatchObject({ kind: "pr", id: "PR_1" });
    expect(routineLaunch.mock.calls[0][2]).toBe("be careful");
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("shows the active run instead of allowing a second launch", async () => {
    renderModal([{ routine: routine(), active_run: run() }]);
    const user = userEvent.setup();

    expect(await screen.findByText("Running")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Babysit/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Open log" }));
    expect(useUIStore.getState().routineRunFocus).toBe(9);
    expect(useUIStore.getState().sessionsTab).toBe("routines");
  });

  it("stops the active run", async () => {
    routineRunStop.mockResolvedValue(undefined);
    renderModal([{ routine: routine(), active_run: run() }]);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Stop" }));
    await waitFor(() => expect(routineRunStop).toHaveBeenCalled());
    expect(routineRunStop.mock.calls[0][0]).toBe(9);
  });

  it("explains when no routine fits", async () => {
    renderModal([]);
    expect(await screen.findByText(/No routine fits this context/)).toBeInTheDocument();
  });
});
