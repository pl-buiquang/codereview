import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const quickPrDiff = vi.fn();
const mirrorPrMeta = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    quickPrDiff: (...a: unknown[]) => quickPrDiff(...a),
    mirrorPrMeta: (...a: unknown[]) => mirrorPrMeta(...a),
  },
}));

import { QuickViewPane } from "./QuickViewPane";
import type { InboxItem } from "../lib/types";

const DIFF = `diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old
+new
`;

const item = { id: "PR_1", type: "pr", number: 7, repo: "acme/widget", title: "Add widgets", author_login: "octocat" } as InboxItem;

function setup(over: { viewerLogin?: string | null; meta?: object } = {}) {
  mirrorPrMeta.mockResolvedValue({ state: "OPEN", isDraft: false, autoMerge: null, ...over.meta });
  const props = {
    item,
    viewerLogin: over.viewerLogin ?? "me",
    onClose: vi.fn(),
    onOpenReview: vi.fn(),
    onApprove: vi.fn(),
    onApproveAndMerge: vi.fn(),
  };
  render(
    <QueryClientProvider client={new QueryClient()}>
      <QuickViewPane {...props} />
    </QueryClientProvider>,
  );
  return props;
}

beforeEach(() => {
  quickPrDiff.mockReset().mockResolvedValue(DIFF);
  mirrorPrMeta.mockReset();
});

describe("QuickViewPane", () => {
  it("closes on Esc and on a backdrop click", async () => {
    const props = setup();
    await userEvent.keyboard("{Escape}");
    expect(props.onClose).toHaveBeenCalledTimes(1);
    await userEvent.click(document.querySelector(".quick-view-backdrop")!);
    expect(props.onClose).toHaveBeenCalledTimes(2);
  });

  it("fires the header actions", async () => {
    const props = setup();
    const merge = await screen.findByRole("button", { name: /Approve & merge/ });
    await waitFor(() => expect(merge).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Approve" }));
    await userEvent.click(merge);
    await userEvent.click(screen.getByRole("button", { name: "Open review" }));
    expect(props.onApprove).toHaveBeenCalled();
    expect(props.onApproveAndMerge).toHaveBeenCalledWith(false);
    expect(props.onOpenReview).toHaveBeenCalled();
  });

  it("hides the approve buttons on your own PR", async () => {
    setup({ viewerLogin: "octocat" });
    await screen.findByRole("button", { name: "Open review" });
    expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
  });

  it("disables Approve & merge on a draft", async () => {
    setup({ meta: { isDraft: true } });
    const merge = await screen.findByRole("button", { name: /Approve & merge/ });
    await waitFor(() => expect(merge).toHaveAttribute("title", "Draft PRs can't be merged"));
    expect(merge).toBeDisabled();
  });

  it("shows the error when the diff fails", async () => {
    quickPrDiff.mockRejectedValue("too large");
    setup();
    expect(await screen.findByText(/Could not load the diff/)).toBeInTheDocument();
  });

  it("collapses and expands a file card from its header chevron", async () => {
    setup();
    const header = await screen.findByRole("button", { name: /a\.txt/ });
    expect(header).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("new")).toBeInTheDocument();
    await userEvent.click(header);
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("new")).not.toBeInTheDocument();
    await userEvent.click(header);
    expect(screen.getByText("new")).toBeInTheDocument();
  });
});
