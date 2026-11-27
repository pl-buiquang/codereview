import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReviewDiscussionComment } from "../lib/types";

vi.mock("./Discussion", () => ({
  DiscussionItem: ({ comment, extra }: { comment: ReviewDiscussionComment; extra?: React.ReactNode }) => (
    <div data-testid="item">
      {comment.body}
      {extra}
    </div>
  ),
  DiscussionReply: () => null,
}));

import { PrConversationRow } from "./PrConversationRow";

const c = (id: string, body: string, hiddenBy: string[] = []): ReviewDiscussionComment => ({
  id,
  databaseId: null,
  kind: "comment",
  reviewState: null,
  author: { login: id } as ReviewDiscussionComment["author"],
  body,
  createdAt: "2026-01-01T00:00:00Z",
  url: "u",
  hiddenBy,
});

const subject = { id: "PR_1", owner: "acme", name: "widgets", number: 1 };

describe("PrConversationRow", () => {
  it("hides filtered comments until Show all is on", async () => {
    render(
      <PrConversationRow
        subject={subject}
        comments={[c("bot", "validator passed", ["bot"]), c("bob", "please fix"), c("sonar", "gate", ["r1"])]}
        rules={[{ id: "r1", name: "Quality gates", enabled: true }]}
        onChanged={() => {}}
      />,
    );
    expect(screen.getByText(/1 comment · 2 hidden/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /PR conversation/ }));
    expect(screen.getAllByTestId("item").map((e) => e.textContent)).toEqual(["please fix"]);
    await userEvent.click(screen.getByLabelText("Show all"));
    expect(screen.getAllByTestId("item")).toHaveLength(3);
    expect(screen.getByText("hidden: Quality gates")).toBeInTheDocument();
    expect(screen.getByText("hidden: bot")).toBeInTheDocument();
  });

  it("colors approval and change-request reviews", async () => {
    const review = (id: string, reviewState: string) => ({ ...c(id, id), kind: "review" as const, reviewState });
    const { container } = render(
      <PrConversationRow
        subject={subject}
        comments={[review("ok", "APPROVED"), review("ko", "CHANGES_REQUESTED"), c("plain", "plain")]}
        rules={[]}
        onChanged={() => {}}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /PR conversation/ }));
    expect([...container.querySelectorAll(".pr-conversation-item")].map((e) => e.className)).toEqual([
      "pr-conversation-item is-approved",
      "pr-conversation-item is-changes-requested",
      "pr-conversation-item",
    ]);
  });

  it("has no Show all switch when nothing is hidden", () => {
    render(<PrConversationRow subject={subject} comments={[c("bob", "hi")]} rules={[]} onChanged={() => {}} />);
    expect(screen.queryByLabelText("Show all")).toBeNull();
  });
});
