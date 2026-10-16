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

  it("renders suggestion fences as a panel", () => {
    const { container } = render(
      <Markdown source={"```suggestion\nlet x = 1;\n```"} />,
    );

    const block = container.querySelector(".suggestion-block");
    expect(block).not.toBeNull();
    expect(screen.getByText("Suggested change")).toBeTruthy();
    const newCode = container.querySelector(".suggestion-block-new");
    expect(newCode?.textContent).toContain("let x = 1;");
    // The fence is upgraded, not left as a default highlighted code block.
    expect(container.querySelector("pre > code.language-suggestion")).toBeNull();
  });

  it("leaves other code fences alone", () => {
    const { container } = render(
      <Markdown source={"```ts\nconst a = 1;\n```"} />,
    );

    expect(container.querySelector(".suggestion-block")).toBeNull();
    const pre = container.querySelector("pre");
    expect(pre).not.toBeNull();
    expect(pre?.querySelector("code")?.textContent).toContain("const a = 1;");
  });

  it("renders empty suggestion as removal", () => {
    const { container } = render(<Markdown source={"```suggestion\n```"} />);

    const empty = container.querySelector(".suggestion-block-empty");
    expect(empty).not.toBeNull();
    expect(empty?.textContent).toBe("(removes the selected lines)");
  });

  it("fetches a proxied GitHub image once across mounts", async () => {
    const src = "https://github.com/user-attachments/assets/cache-test";
    const fetchImage = vi
      .spyOn(api, "fetchGithubImage")
      .mockResolvedValue("data:image/png;base64,AAAA");

    const first = render(<Markdown source={`![shot](${src})`} />);
    expect(await screen.findByRole("img", { name: "shot" })).toHaveAttribute(
      "src",
      "data:image/png;base64,AAAA",
    );
    first.unmount();

    render(<Markdown source={`![shot](${src})`} />);
    expect(screen.getByRole("img", { name: "shot" })).toHaveAttribute(
      "src",
      "data:image/png;base64,AAAA",
    );
    expect(fetchImage).toHaveBeenCalledTimes(1);
    fetchImage.mockRestore();
  });
});
