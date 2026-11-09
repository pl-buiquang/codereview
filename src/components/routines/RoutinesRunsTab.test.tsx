import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { RoutineRun } from "../../lib/types";

const routineRunList = vi.fn();
const hostStates = vi.fn();
vi.mock("../../lib/api", () => ({
  api: {
    routineRunList: (...a: unknown[]) => routineRunList(...a),
    routineList: () => Promise.resolve([]),
    aiHarnesses: () => Promise.resolve([{ id: "claude", name: "Claude Code", available: true, caps: {} }]),
    routineRunEvents: () => Promise.resolve([]),
    hostList: () => Promise.resolve([{ id: 5, name: "koza", type: "ssh", ssh_target: "koza", remote_endpoint: "7", enabled: true, last_seen: null, last_protocol: null, last_version: null }]),
    hostStates: (...a: unknown[]) => hostStates(...a),
    routineRunStop: () => Promise.resolve(),
    routineRunRelaunch: () => Promise.resolve(),
    resumeClaudeSession: () => Promise.resolve(),
  },
}));

import { RoutinesRunsTab } from "./RoutinesRunsTab";

const run = (over: Partial<RoutineRun> = {}): RoutineRun => ({
  id: 1,
  routine_id: 1,
  routine_key: "babysit",
  routine_name: "Babysit",
  context_type: "pr",
  context_key: "pr:o/n#1",
  context_label: "o/n#1",
  context: { kind: "pr", id: "c1", base_paths: [] },
  status: "running",
  note: "",
  model: "",
  session_id: null,
  cwd: null,
  result_text: null,
  error: null,
  cost_usd: 0,
  num_turns: 0,
  harness: "claude",
  host_id: null,
  queued_at: new Date().toISOString(),
  started_at: new Date().toISOString(),
  ended_at: null,
  ...over,
});

function renderTab() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <RoutinesRunsTab />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  hostStates.mockReset().mockResolvedValue([{ host_id: 5, state: { state: "connected", protocol: 1, version: "0.6.0" } }]);
});

describe("RoutinesRunsTab remote runs", () => {
  it("badges a run with its host and none for a local one", async () => {
    routineRunList.mockResolvedValue([run({ id: 1, host_id: 5 }), run({ id: 2, context_key: "pr:o/n#2", context_label: "o/n#2" })]);
    renderTab();
    const badges = await screen.findAllByTestId("run-host-badge");
    expect(badges).toHaveLength(1);
    expect(badges[0].textContent).toBe("koza");
  });

  it("flags a running remote run whose host connection is down", async () => {
    hostStates.mockResolvedValue([{ host_id: 5, state: { state: "unreachable", error: "Connection timed out" } }]);
    routineRunList.mockResolvedValue([run({ host_id: 5 })]);
    renderTab();
    expect(await screen.findByText("disconnected")).toBeTruthy();
  });

  it("shows a remote failure readably in the run header", async () => {
    const user = userEvent.setup();
    routineRunList.mockResolvedValue([run({ host_id: 5, status: "error", error: "host_unreachable: ssh: Connection refused", ended_at: new Date().toISOString() })]);
    renderTab();
    await user.click(await screen.findByRole("button", { name: /Babysit/ }));
    const warn = await screen.findByText(/The host could not be reached/);
    expect(warn.textContent).toBe("The host could not be reached: ssh: Connection refused");
    expect(within(document.body).queryByRole("button", { name: /Resume/ })).toBeNull();
  });
});
