import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Markdown } from "./Markdown";
import { api } from "../lib/api";

// Real mermaid needs browser layout; the wiring is what's under test.
vi.mock("mermaid", () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn().mockResolvedValue({ svg: '<svg data-testid="diagram"></svg>' }),
  },
}));

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

  it("strips scripts and event handlers from raw HTML", () => {
    const { container } = render(
      <Markdown
        source={'<script>alert(1)</script> <img src=x onerror="alert(2)">'}
      />,
    );

    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img[onerror]")).toBeNull();
  });

  it("renders <details> blocks and hides HTML comments", () => {
    const { container } = render(
      <Markdown
        source={"<!-- hidden -->\n\nvisible\n\n<details>\n<summary>Exact commands</summary>\n\nbody **bold**\n\n</details>"}
      />,
    );

    expect(container.textContent).not.toContain("hidden");
    expect(container.querySelector("details > summary")?.textContent).toBe("Exact commands");
    expect(container.querySelector("details strong")?.textContent).toBe("bold");
  });

  it("colours diff fences per line", () => {
    const { container } = render(
      <Markdown source={"```diff\n@@ -1 +1 @@\n-old\n+new\n ctx\n```"} />,
    );

    expect(container.querySelectorAll(".md-diff-add")).toHaveLength(1);
    expect(container.querySelectorAll(".md-diff-del")).toHaveLength(1);
    expect(container.querySelectorAll(".md-diff-hunk")).toHaveLength(1);
  });

  it("highlights code fences once the grammar loads", async () => {
    const { container } = render(<Markdown source={"```sh\necho \"hi\"\n```"} />);

    await waitFor(() => expect(container.querySelector("code .token")).not.toBeNull());
    expect(container.querySelector("code")?.textContent).toBe('echo "hi"\n');
  });

  it("renders mermaid fences as a diagram", async () => {
    const { container } = render(<Markdown source={"```mermaid\ngraph TD; A-->B\n```"} />);

    expect(container.querySelector(".md-mermaid")).not.toBeNull();
    await waitFor(() => expect(container.querySelector(".md-mermaid svg")).not.toBeNull());
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

  it("fetches the signed URL for an SSO attachment but caches under the original src", async () => {
    const uuid = "40c62b64-7e5b-48e4-b488-c3417a123a9c";
    const src = `https://github.com/user-attachments/assets/${uuid}`;
    const signed = `https://private-user-images.githubusercontent.com/1/2-${uuid}.png?jwt=x`;
    const fetchImage = vi
      .spyOn(api, "fetchGithubImage")
      .mockResolvedValue("data:image/png;base64,BBBB");
    render(<Markdown source={`![shot](${src})`} imageUrls={{ [uuid]: signed }} />);
    expect(await screen.findByRole("img", { name: "shot" })).toHaveAttribute(
      "src",
      "data:image/png;base64,BBBB",
    );
    expect(fetchImage).toHaveBeenCalledWith(signed);
    fetchImage.mockRestore();
  });
});
