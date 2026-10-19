import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StackBadge } from "./InboxBadges";

describe("StackBadge", () => {
  it("renders nothing for a PR outside a stack", () => {
    const { container } = render(<StackBadge number={null} position={null} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the stack number and position", () => {
    render(<StackBadge number={40} position={2} />);
    expect(screen.getByText(/stack #40 · 2/)).toBeInTheDocument();
  });

  it("omits the position when unknown", () => {
    render(<StackBadge number={40} position={null} />);
    expect(screen.getByText(/stack #40$/)).toBeInTheDocument();
  });

  it("calls onClick when clickable", async () => {
    const onClick = vi.fn();
    render(<StackBadge number={40} position={1} onClick={onClick} />);
    await userEvent.setup().click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledOnce();
  });
});
