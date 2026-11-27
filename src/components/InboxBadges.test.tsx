import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("./CiChecksPopover", () => ({ CiChecksPopover: () => null }));

import { PrStatusChip } from "./InboxBadges";

describe("PrStatusChip", () => {
  it("offers a draft/open switch on an open PR", async () => {
    const onSetDraft = vi.fn();
    render(<PrStatusChip state="open" isDraft={false} onSetDraft={onSetDraft} />);
    await userEvent.click(screen.getByRole("button", { name: /open/ }));
    expect(screen.getByRole("menuitem", { name: /Open \(ready for review\)/ })).toBeDisabled();
    await userEvent.click(screen.getByRole("menuitem", { name: "Draft" }));
    expect(onSetDraft).toHaveBeenCalledWith(true);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("marks a draft PR ready for review", async () => {
    const onSetDraft = vi.fn();
    render(<PrStatusChip state="open" isDraft onSetDraft={onSetDraft} />);
    await userEvent.click(screen.getByRole("button", { name: /draft/ }));
    await userEvent.click(screen.getByRole("menuitem", { name: /Open/ }));
    expect(onSetDraft).toHaveBeenCalledWith(false);
  });

  it.each(["merged", "closed"])("has no dropdown on a %s PR", (state) => {
    render(<PrStatusChip state={state} isDraft={false} onSetDraft={() => {}} />);
    expect(screen.getByText(state)).toBeInTheDocument();
    expect(screen.queryByRole("button")).toBeNull();
  });
});
