import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const prChecks = vi.fn();
const rerunCiJob = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    prChecks: (...a: unknown[]) => prChecks(...a),
    rerunCiJob: (...a: unknown[]) => rerunCiJob(...a),
    rerunCiRuns: vi.fn(),
    openUrl: vi.fn(),
  },
}));

import { CiBadge } from "./InboxBadges";
import type { CiCheck, PrChecks } from "../lib/types";

const check = (over: Partial<CiCheck>): CiCheck => ({
  name: "e2e",
  jobId: 99,
  runId: 1,
  workflow: "CI",
  state: "FAILURE",
  bucket: "failed",
  url: "https://x/job/99",
  required: false,
  durationSecs: 10,
  failedSteps: ["Run e2e"],
  annotations: [],
  ...over,
});
const checks = (cs: CiCheck[]): PrChecks => ({ state: "FAILURE", prUrl: "https://github.com/o/r/pull/7", checks: cs });

describe("CiChecksPopover", () => {
  it("keeps the list open after rerunning a single job", async () => {
    prChecks.mockResolvedValue(checks([check({}), check({ name: "lint", jobId: 5, bucket: "passed", state: "SUCCESS" })]));
    rerunCiJob.mockResolvedValue(undefined);
    render(
      <QueryClientProvider client={new QueryClient()}>
        <CiBadge state="failure" pr={{ owner: "o", name: "r", number: 7 }} />
      </QueryClientProvider>,
    );
    await userEvent.click(screen.getByTitle(/show checks/i));
    expect(await screen.findByText("e2e")).toBeInTheDocument();
    await userEvent.click(screen.getByTitle("Rerun this job"));
    await waitFor(() => expect(rerunCiJob).toHaveBeenCalledWith("o", "r", 99));
    expect(screen.getByText("e2e")).toBeInTheDocument();
    expect(await screen.findByText("Rerun requested")).toBeInTheDocument();
  });

  it("keeps rows once the rerun job shows as queued", async () => {
    prChecks.mockResolvedValueOnce(checks([check({}), check({ name: "lint", jobId: 5, bucket: "passed", state: "SUCCESS" })]));
    prChecks.mockResolvedValue(
      checks([
        check({ bucket: "pending", state: "QUEUED", failedSteps: [], durationSecs: null }),
        check({ name: "lint", jobId: 5, bucket: "passed", state: "SUCCESS" }),
      ]),
    );
    rerunCiJob.mockResolvedValue(undefined);
    render(
      <QueryClientProvider client={new QueryClient()}>
        <CiBadge state="failure" pr={{ owner: "o", name: "r", number: 7 }} />
      </QueryClientProvider>,
    );
    await userEvent.click(screen.getByTitle(/show checks/i));
    await userEvent.click(await screen.findByTitle("Rerun this job"));
    await waitFor(() => expect(prChecks).toHaveBeenCalledTimes(2), { timeout: 4000 });
    expect(await screen.findByText("e2e")).toBeInTheDocument();
    expect(screen.getByText(/1 pending/)).toBeInTheDocument();
  });
});
