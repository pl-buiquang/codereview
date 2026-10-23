import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TagFilter } from "./TagFilter";

const TAGS: [string, number][] = [
  ["urgent", 3],
  ["projA", 2],
  ["projB", 1],
];

describe("TagFilter", () => {
  it("filters options by query and selects on Enter", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<TagFilter tags={TAGS} selected={[]} onChange={onChange} />);

    const input = screen.getByPlaceholderText("Filter by tag…");
    await user.click(input);
    await user.type(input, "proj");

    expect(screen.getByRole("option", { name: /projA/ })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /urgent/ })).not.toBeInTheDocument();

    await user.keyboard("{Enter}");
    expect(onChange).toHaveBeenCalledWith(["projA"]);
  });

  it("removes a chip via its remove button", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<TagFilter tags={TAGS} selected={["urgent"]} onChange={onChange} />);

    await user.click(screen.getByTitle("Remove urgent filter"));
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("removes the last chip on Backspace in an empty input", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<TagFilter tags={TAGS} selected={["urgent", "projA"]} onChange={onChange} />);

    const input = screen.getByPlaceholderText("Filter by tag…");
    await user.click(input);
    await user.keyboard("{Backspace}");

    expect(onChange).toHaveBeenCalledWith(["urgent"]);
  });

  it("excludes already-selected tags from the dropdown", async () => {
    const user = userEvent.setup();
    render(<TagFilter tags={TAGS} selected={["urgent"]} onChange={vi.fn()} />);

    await user.click(screen.getByPlaceholderText("Filter by tag…"));
    expect(screen.queryByRole("option", { name: /urgent/ })).not.toBeInTheDocument();
    expect(screen.getByRole("option", { name: /projA/ })).toBeInTheDocument();
  });

  it("hides tags marked unavailable from the dropdown", async () => {
    const user = userEvent.setup();
    render(<TagFilter tags={TAGS} selected={[]} onChange={vi.fn()} unavailable={["projA"]} />);

    await user.click(screen.getByPlaceholderText("Filter by tag…"));
    expect(screen.queryByRole("option", { name: /projA/ })).not.toBeInTheDocument();
    expect(screen.getByRole("option", { name: /urgent/ })).toBeInTheDocument();
  });

  it("renders a custom title and placeholder", () => {
    render(
      <TagFilter tags={TAGS} selected={[]} onChange={vi.fn()} title="Exclude tags" placeholder="Exclude tag…" />,
    );

    expect(screen.getByText("Exclude tags")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Exclude tag…")).toBeInTheDocument();
  });

  it("toggles the Untagged row", async () => {
    const user = userEvent.setup();
    const onUntagged = vi.fn();
    render(
      <TagFilter tags={TAGS} selected={[]} onChange={vi.fn()} untagged={{ on: false, count: 4, onChange: onUntagged }} />,
    );

    const row = screen.getByRole("button", { name: /Untagged/ });
    expect(row).toHaveTextContent("4");
    await user.click(row);
    expect(onUntagged).toHaveBeenCalledWith(true);
  });

  it("shows the Untagged row even without tags, and clear resets it", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    const onUntagged = vi.fn();
    render(<TagFilter tags={[]} selected={[]} onChange={onChange} untagged={{ on: true, count: 2, onChange: onUntagged }} />);

    expect(screen.getByRole("button", { name: /Untagged/ })).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByText("clear"));
    expect(onChange).toHaveBeenCalledWith([]);
    expect(onUntagged).toHaveBeenCalledWith(false);
  });
});
