import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TooltipLayer } from "./TooltipLayer";

describe("TooltipLayer", () => {
  it("replaces the native title with a fast tooltip and restores it on leave", async () => {
    const user = userEvent.setup();
    render(
      <>
        <TooltipLayer />
        <button title={"Line one\nLine two"}>
          <span>Hover</span>
        </button>
      </>,
    );
    const btn = screen.getByRole("button");

    await user.hover(screen.getByText("Hover"));
    expect(btn).not.toHaveAttribute("title");
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Line one Line two");

    await user.unhover(btn);
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    expect(btn).toHaveAttribute("title", "Line one\nLine two");
  });

  it("skips clipped-only anchors whose text fits", async () => {
    const user = userEvent.setup();
    render(
      <>
        <TooltipLayer />
        <span title="a/b.ts" data-cr-title-when="clipped">a/b.ts</span>
      </>,
    );
    await user.hover(screen.getByText("a/b.ts"));
    await new Promise((r) => setTimeout(r, 200));
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });

  it("leaves data-native-title anchors alone", async () => {
    const user = userEvent.setup();
    render(
      <>
        <TooltipLayer />
        <span title="native" data-native-title>
          x
        </span>
      </>,
    );
    await user.hover(screen.getByText("x"));
    expect(screen.getByText("x")).toHaveAttribute("title", "native");
  });
});
