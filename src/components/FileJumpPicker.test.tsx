import { describe, it, expect, vi, beforeAll } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FileJumpPicker } from "./FileJumpPicker";

const files = [
  { path: "src/a.ts", add: 1, del: 0 },
  { path: "src/b.ts", add: 2, del: 1 },
];

describe("FileJumpPicker", () => {
  beforeAll(() => {
    // jsdom has no layout, so no scrollIntoView.
    Element.prototype.scrollIntoView = vi.fn();
  });

  it("focuses the input, filters, and picks with Enter then dismisses", async () => {
    const onPick = vi.fn();
    const onDismiss = vi.fn();
    render(<FileJumpPicker autoFocus files={files} onPick={onPick} onDismiss={onDismiss} />);
    const input = screen.getByRole("combobox");
    expect(input).toHaveFocus();
    await userEvent.keyboard("b.ts{Enter}");
    expect(onPick).toHaveBeenCalledWith("src/b.ts");
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("Esc clears the query first, then dismisses", async () => {
    const onDismiss = vi.fn();
    render(<FileJumpPicker autoFocus files={files} onPick={vi.fn()} onDismiss={onDismiss} />);
    await userEvent.keyboard("a{Escape}");
    expect(onDismiss).not.toHaveBeenCalled();
    await userEvent.keyboard("{Escape}");
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
