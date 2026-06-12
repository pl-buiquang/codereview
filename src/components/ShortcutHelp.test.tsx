import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ShortcutHelp } from "./ShortcutHelp";
import { BINDINGS } from "../lib/keyboard";

describe("ShortcutHelp", () => {
  it("renders every binding", () => {
    render(<ShortcutHelp onClose={() => {}} />);
    for (const b of BINDINGS) {
      expect(screen.getByText(b.description)).toBeInTheDocument();
    }
  });

  it("closes on backdrop click and ✕, but not on panel-body click", async () => {
    const onClose = vi.fn();
    const { container } = render(<ShortcutHelp onClose={onClose} />);

    await userEvent.click(screen.getByText("Keyboard shortcuts"));
    expect(onClose).not.toHaveBeenCalled();

    await userEvent.click(screen.getByTitle("Close"));
    expect(onClose).toHaveBeenCalledTimes(1);

    const backdrop = container.querySelector(".modal-backdrop")!;
    await userEvent.click(backdrop);
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
