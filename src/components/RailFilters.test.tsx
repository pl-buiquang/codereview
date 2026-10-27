import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useUIStore } from "../store";
import { FilterList, FilterRail, MultiFilterList } from "./RailFilters";

const ENTRIES: [string, number][] = [
  ["open", 3],
  ["closed", 1],
];

beforeEach(() => useUIStore.setState({ collapsedRailSections: [] }));

describe("RailSection", () => {
  it("folds and unfolds its items, keyed by rail and title", async () => {
    const user = userEvent.setup();
    render(
      <FilterRail id="board">
        <FilterList title="Status" entries={ENTRIES} selected={null} onSelect={vi.fn()} />
      </FilterRail>,
    );
    expect(screen.getByText("open")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Status", expanded: true }));
    expect(screen.queryByText("open")).not.toBeInTheDocument();
    expect(useUIStore.getState().collapsedRailSections).toEqual(["board:Status"]);

    await user.click(screen.getByRole("button", { name: "Status", expanded: false }));
    expect(screen.getByText("open")).toBeInTheDocument();
  });

  it("keeps fold state separate between rails", () => {
    useUIStore.setState({ collapsedRailSections: ["board:Status"] });
    render(
      <FilterRail id="inbox">
        <FilterList title="Status" entries={ENTRIES} selected={null} onSelect={vi.fn()} />
      </FilterRail>,
    );
    expect(screen.getByText("open")).toBeInTheDocument();
  });

  it("shows the selected value when a single-select is folded", () => {
    useUIStore.setState({ collapsedRailSections: ["board:Status"] });
    render(
      <FilterRail id="board">
        <FilterList title="Status" entries={ENTRIES} selected="open" onSelect={vi.fn()} />
      </FilterRail>,
    );
    expect(screen.getByText("· open")).toBeInTheDocument();
  });

  it("shows a count when a multi-select is folded", () => {
    useUIStore.setState({ collapsedRailSections: ["board:Repos"] });
    render(
      <FilterRail id="board">
        <MultiFilterList title="Repos" entries={ENTRIES} selected={["open", "closed"]} onChange={vi.fn()} />
      </FilterRail>,
    );
    expect(screen.getByText("· 2")).toBeInTheDocument();
  });

  it("clears without toggling the fold", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    useUIStore.setState({ collapsedRailSections: ["board:Status"] });
    render(
      <FilterRail id="board">
        <FilterList title="Status" entries={ENTRIES} selected="open" onSelect={onSelect} />
      </FilterRail>,
    );
    await user.click(screen.getByRole("button", { name: "clear" }));
    expect(onSelect).toHaveBeenCalledWith(null);
    expect(useUIStore.getState().collapsedRailSections).toEqual(["board:Status"]);
  });
});
