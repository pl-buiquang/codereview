import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// ReviewView imports the api barrel at module load; the Composer itself never
// calls it, but the import must resolve.
vi.mock("../lib/api", () => ({ api: {}, pickSavePath: vi.fn() }));

import { Composer } from "./ReviewView";

const noop = async () => {};

describe("Composer suggestion seed", () => {
  it("renders no Insert suggestion button without a seed", () => {
    render(<Composer onSubmit={noop} onCancel={() => {}} />);
    expect(screen.queryByText(/Insert suggestion/)).toBeNull();
  });

  it("seeds an empty textarea with just the fence", async () => {
    const fence = "```suggestion\nlet x = 1;\n```";
    render(<Composer onSubmit={noop} onCancel={() => {}} suggestionSeed={fence} />);

    await userEvent.click(screen.getByText(/Insert suggestion/));

    const textarea = screen.getByPlaceholderText("Leave a comment…") as HTMLTextAreaElement;
    expect(textarea.value).toBe(fence);
  });

  it("appends the fence after existing text with a blank-line separator", async () => {
    const fence = "```suggestion\nlet x = 1;\n```";
    render(<Composer onSubmit={noop} onCancel={() => {}} suggestionSeed={fence} />);

    const textarea = screen.getByPlaceholderText("Leave a comment…") as HTMLTextAreaElement;
    await userEvent.type(textarea, "please apply:");
    await userEvent.click(screen.getByText(/Insert suggestion/));

    expect(textarea.value).toBe(`please apply:\n\n${fence}`);
  });
});
