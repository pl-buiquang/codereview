import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Markdown } from "./Markdown";
import { api } from "../lib/api";

describe("Markdown", () => {
  it("renders bold, lists and links", () => {
    render(
      <Markdown source={"**bold**\n\n- one\n- two\n\n[link](https://example.com)"} />,
    );

    const strong = screen.getByText("bold");
    expect(strong.tagName).toBe("STRONG");

    const items = screen.getAllByRole("listitem");
    expect(items.map((li) => li.textContent)).toEqual(["one", "two"]);

    const link = screen.getByRole("link", { name: "link" });
    expect(link).toHaveAttribute("href", "https://example.com");
  });

  it("opens links via api.openUrl instead of navigating", async () => {
    const openUrl = vi.spyOn(api, "openUrl").mockResolvedValue(undefined);
    render(<Markdown source={"[link](https://example.com)"} />);

    await userEvent.click(screen.getByRole("link", { name: "link" }));

    expect(openUrl).toHaveBeenCalledWith("https://example.com");
    openUrl.mockRestore();
  });

  it("renders GFM tables, task lists and strikethrough", () => {
    const { container } = render(
      <Markdown
        source={
          "| a | b |\n| - | - |\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n\n~~gone~~"
        }
      />,
    );

    expect(container.querySelector("table")).not.toBeNull();
    const checkboxes = container.querySelectorAll('input[type="checkbox"]');
    expect(checkboxes.length).toBe(2);
    expect(screen.getByText("gone").tagName).toBe("DEL");
  });

  it("escapes raw HTML instead of executing it", () => {
    const { container } = render(
      <Markdown
        source={'<script>alert(1)</script> <img src=x onerror="alert(2)">'}
      />,
    );

    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img[onerror]")).toBeNull();
    expect(container.textContent).toContain("<script>alert(1)</script>");
  });
});
