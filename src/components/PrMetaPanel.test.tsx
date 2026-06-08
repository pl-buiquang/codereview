import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const prMeta = vi.fn();
const openUrl = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    prMeta: (owner: string, name: string, number: number) => prMeta(owner, name, number),
    openUrl: (url: string) => openUrl(url),
  },
}));

import { PrMetaPanel } from "./PrMetaPanel";
import type { PrMeta } from "../lib/types";

const meta = (over: Partial<PrMeta> = {}): PrMeta => ({
  number: 42,
  title: "Add widgets",
  url: "https://github.com/acme/widget/pull/42",
  body: "",
  state: "OPEN",
  isDraft: false,
  mergeable: "MERGEABLE",
  reviewDecision: null,
  additions: 120,
  deletions: 7,
  changedFiles: 5,
  author: { login: "octocat", avatarUrl: null },
  labels: [],
  reviews: [],
  ciState: null,
  checks: [],
  ...over,
});

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<PrMetaPanel owner="acme" name="widget" number={42} />, { wrapper });
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("PrMetaPanel", () => {
  it("renders counts, labels and the description as Markdown", async () => {
    prMeta.mockResolvedValue(
      meta({
        body: "**Summary** line.",
        labels: [{ name: "bug", color: "d73a4a" }],
      }),
    );
    renderPanel();

    expect(await screen.findByText(/5 files/)).toBeInTheDocument();
    expect(screen.getByText("+120")).toBeInTheDocument();
    expect(screen.getByText("−7")).toBeInTheDocument();
    expect(screen.getByText("bug")).toBeInTheDocument();
    expect(screen.getByText("Summary").tagName).toBe("STRONG");
  });

  it("shows a neutral Checking… state for unknown mergeability", async () => {
    prMeta.mockResolvedValue(meta({ mergeable: "UNKNOWN" }));
    renderPanel();

    const el = await screen.findByText("Checking…");
    expect(el).toHaveClass("mergeable-neutral");
  });

  it("renders a muted inline error without throwing", async () => {
    prMeta.mockRejectedValue("boom");
    renderPanel();

    expect(await screen.findByText(/Could not load PR details/)).toBeInTheDocument();
  });

  it("expands the check list and opens check urls via api.openUrl", async () => {
    const user = userEvent.setup();
    prMeta.mockResolvedValue(
      meta({ checks: [{ name: "build", state: "SUCCESS", url: "https://ci/build" }] }),
    );
    renderPanel();

    await user.click(await screen.findByText(/1 check/));
    await user.click(screen.getByRole("link", { name: "build" }));

    await waitFor(() => expect(openUrl).toHaveBeenCalledWith("https://ci/build"));
  });
});
