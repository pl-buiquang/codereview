import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UpdateReviewButton } from "./UpdateReviewButton";

function setup(over: Partial<React.ComponentProps<typeof UpdateReviewButton>> = {}) {
  const props = {
    busy: false,
    canUpdate: true,
    canReviewNew: true,
    onUpdate: vi.fn(),
    onReviewNew: vi.fn(),
    ...over,
  };
  render(<UpdateReviewButton {...props} />);
  return props;
}

describe("UpdateReviewButton", () => {
  it("defaults to Update review", async () => {
    const user = userEvent.setup();
    const props = setup();
    await user.click(screen.getByRole("button", { name: "Update review" }));
    expect(props.onUpdate).toHaveBeenCalledTimes(1);
    expect(props.onReviewNew).not.toHaveBeenCalled();
  });

  it("offers a new review of the new commits in the menu", async () => {
    const user = userEvent.setup();
    const props = setup();
    await user.click(screen.getByRole("button", { name: "More update options" }));
    await user.click(screen.getByRole("menuitem", { name: "Review new commits as a new review" }));
    expect(props.onReviewNew).toHaveBeenCalledTimes(1);
    expect(props.onUpdate).not.toHaveBeenCalled();
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("on a published review only the menu action is enabled", async () => {
    const user = userEvent.setup();
    const props = setup({ canUpdate: false });
    expect(screen.getByRole("button", { name: "Update review" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "More update options" }));
    await user.click(screen.getByRole("menuitem"));
    expect(props.onReviewNew).toHaveBeenCalledTimes(1);
  });

  it("local drafts get no menu, local published reviews get nothing", () => {
    const { unmount } = render(
      <UpdateReviewButton busy={false} canUpdate canReviewNew={false} onUpdate={vi.fn()} onReviewNew={vi.fn()} />,
    );
    expect(screen.queryByRole("button", { name: "More update options" })).not.toBeInTheDocument();
    unmount();
    const { container } = render(
      <UpdateReviewButton busy={false} canUpdate={false} canReviewNew={false} onUpdate={vi.fn()} onReviewNew={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});
