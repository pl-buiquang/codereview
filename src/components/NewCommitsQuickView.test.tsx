import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const quickInterdiff = vi.fn();
vi.mock("../lib/api", () => ({
  api: { quickInterdiff: (...a: unknown[]) => quickInterdiff(...a) },
}));

import { NewCommitsQuickView } from "./NewCommitsQuickView";

const DIFF = `diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old
+new
`;

function setup(busy = false) {
  const props = { reviewId: 1, headSha: "h1", busy, onClose: vi.fn(), onCreate: vi.fn() };
  render(
    <QueryClientProvider client={new QueryClient()}>
      <NewCommitsQuickView {...props} />
    </QueryClientProvider>,
  );
  return props;
}

beforeEach(() => {
  quickInterdiff.mockReset().mockResolvedValue({
    diff: DIFF,
    baseSha: "b2",
    headSha: "h2",
    rebased: false,
    approximate: false,
  });
});

describe("NewCommitsQuickView", () => {
  it("disables Create review until the diff loaded, then creates at the previewed commits", async () => {
    const props = setup();
    const button = screen.getByRole("button", { name: "Create review" });
    expect(button).toBeDisabled();
    expect(await screen.findByText("new")).toBeInTheDocument();
    expect(button).toBeEnabled();
    await userEvent.click(button);
    expect(props.onCreate).toHaveBeenCalledWith({ baseSha: "b2", headSha: "h2" });
  });

  it("shows the error and keeps Create review disabled", async () => {
    quickInterdiff.mockRejectedValue("no new commits");
    setup();
    expect(await screen.findByText(/Could not load the diff/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create review" })).toBeDisabled();
  });

  it("closes on Esc", async () => {
    const props = setup();
    await userEvent.keyboard("{Escape}");
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });
});
