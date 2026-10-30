import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { RefInfo } from "../../lib/types";

vi.mock("../../lib/toast", () => ({ toast: { error: vi.fn(), info: vi.fn() } }));

import { CommitActionsMenu } from "./GitRefItems";

const SHA = "a".repeat(40);
const ref = (name: string, kind: RefInfo["kind"]): RefInfo => ({ name, kind, sha: SHA, is_head: false, updated_at: 0 });

const writeText = vi.fn((_t: string) => Promise.resolve());

function renderMenu(refs: RefInfo[]) {
  const onClose = vi.fn();
  render(
    <QueryClientProvider client={new QueryClient()}>
      <CommitActionsMenu sha={SHA} refs={refs} anchor={{ x: 0, y: 0 }} onClose={onClose} />
    </QueryClientProvider>,
  );
  return onClose;
}

describe("CommitActionsMenu copy items", () => {
  beforeEach(() => {
    writeText.mockClear();
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  });

  it("copies the sha", async () => {
    const onClose = renderMenu([]);
    await userEvent.click(screen.getByRole("menuitem", { name: "Copy SHA" }));
    expect(writeText).toHaveBeenCalledWith(SHA);
    expect(onClose).toHaveBeenCalled();
    expect(screen.queryByText(/Copy branch\/tag name/)).toBeNull();
  });

  it("copies the only ref name directly", async () => {
    renderMenu([ref("feature/x", "local")]);
    await userEvent.click(screen.getByRole("menuitem", { name: "Copy feature/x" }));
    expect(writeText).toHaveBeenCalledWith("feature/x");
  });

  it("opens a picker when several refs point at the commit", async () => {
    const onClose = renderMenu([ref("main", "local"), ref("origin/main", "remote"), ref("v1.0.0", "tag")]);
    await userEvent.click(screen.getByRole("menuitem", { name: "Copy branch/tag name…" }));
    const dialog = await screen.findByRole("dialog", { name: "Copy name" });
    expect(dialog).toBeTruthy();
    await userEvent.click(screen.getByRole("option", { name: /v1\.0\.0/ }));
    expect(writeText).toHaveBeenCalledWith("v1.0.0");
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});
