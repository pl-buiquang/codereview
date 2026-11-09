import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { HarnessInfo } from "../lib/types";

const aiHarnesses = vi.fn();
vi.mock("../lib/api", () => ({ api: { aiHarnesses: () => aiHarnesses() } }));

import { AgentSessionItems } from "./KebabMenu";

const harness = (id: string, name: string, over: Partial<HarnessInfo> = {}): HarnessInfo => ({
  id,
  name,
  available: true,
  version: null,
  caps: { interactive_tui: true } as HarnessInfo["caps"],
  ...over,
});

function renderItems(onOpenNew: (h?: string) => void) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <AgentSessionItems sessions={[]} busy={false} close={() => {}} onFocus={() => {}} onOpenNew={onOpenNew} />
    </QueryClientProvider>,
  );
}

describe("AgentSessionItems harness picker", () => {
  beforeEach(() => vi.clearAllMocks());

  it("offers only the default entry with a single harness", async () => {
    aiHarnesses.mockResolvedValue([harness("claude", "Claude Code")]);
    const onOpenNew = vi.fn();
    renderItems(onOpenNew);
    await userEvent.click(screen.getByRole("menuitem", { name: "New agent session" }));
    expect(onOpenNew).toHaveBeenCalledWith();
    await waitFor(() => expect(aiHarnesses).toHaveBeenCalled());
    expect(screen.queryByRole("menuitem", { name: /New Claude Code session/ })).not.toBeInTheDocument();
  });

  it("adds one entry per available interactive harness and passes its id", async () => {
    aiHarnesses.mockResolvedValue([
      harness("claude", "Claude Code"),
      harness("opencode", "opencode"),
      harness("codex", "Codex", { available: false }),
    ]);
    const onOpenNew = vi.fn();
    renderItems(onOpenNew);
    await userEvent.click(await screen.findByRole("menuitem", { name: "New opencode session" }));
    expect(onOpenNew).toHaveBeenCalledWith("opencode");
    expect(screen.queryByRole("menuitem", { name: /Codex/ })).not.toBeInTheDocument();
  });
});
