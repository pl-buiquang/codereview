import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { RoutineRun } from "../lib/types";

const routineRunList = vi.fn();
const boardCardForPr = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    routineRunList: (...a: unknown[]) => routineRunList(...a),
    boardCardForPr: (...a: unknown[]) => boardCardForPr(...a),
    boardMetaTags: () => Promise.resolve([]),
  },
}));

import { ContextPane } from "./ContextPane";
import { useUIStore } from "../store";

const run = (over: Partial<RoutineRun>): RoutineRun => ({
  id: 1,
  routine_id: 1,
  routine_key: "ai_review",
  routine_name: "AI review",
  context_type: "pr",
  context_key: "pr:acme/widgets#7",
  context_label: "acme/widgets#7",
  context: { kind: "pr", id: "acme/widgets#7", base_paths: [] } as RoutineRun["context"],
  status: "ok",
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
  ended_at: new Date().toISOString(),
  seen_at: null,
  ...over,
});

function renderPane() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <ContextPane owner="Acme" name="widgets" number={7} />
    </QueryClientProvider>,
  );
}

describe("ContextPane routine runs", () => {
  beforeEach(() => {
    routineRunList.mockReset();
    boardCardForPr.mockReset().mockResolvedValue(null);
  });

  it("lists the PR's runs with their full result and opens one in Sessions", async () => {
    routineRunList.mockResolvedValue([
      run({ id: 4, result_text: "## Verdict\n\nLooks **good**" }),
      run({ id: 3, routine_name: "Rebase", status: "error", error: "boom" }),
    ]);
    const showRoutineRun = vi.fn();
    useUIStore.setState({ showRoutineRun });
    renderPane();

    expect(await screen.findByText("Verdict")).toBeTruthy();
    expect(screen.getByText("good").tagName).toBe("STRONG");
    expect(screen.getByText("boom")).toBeTruthy();
    expect(screen.getByText("Failed")).toBeTruthy();
    expect(routineRunList).toHaveBeenCalledWith({ context_key: "pr:acme/widgets#7", limit: 10 });

    await userEvent.click(screen.getAllByRole("button", { name: "Open" })[0]);
    expect(showRoutineRun).toHaveBeenCalledWith(4);
  });

  it("shows an empty state", async () => {
    routineRunList.mockResolvedValue([]);
    renderPane();
    expect(await screen.findByText("No routine runs yet.")).toBeTruthy();
  });
});
