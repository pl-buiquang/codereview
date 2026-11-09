import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { AiProfile, ChatMessage } from "../lib/types";

const chatSend = vi.fn();
const chatMessages = vi.fn();
const chatProfileId = vi.fn();
const aiProfileList = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    chatSend: (...a: unknown[]) => chatSend(...a),
    chatMessages: (...a: unknown[]) => chatMessages(...a),
    chatProfileId: (...a: unknown[]) => chatProfileId(...a),
    chatWorktree: () => Promise.resolve(null),
    chatClear: () => Promise.resolve(),
    aiProfileList: (...a: unknown[]) => aiProfileList(...a),
  },
}));

import { ChatPanel } from "./ChatPanel";
import { useSettingsStore } from "../lib/settings";

const profile = (id: number, name: string, over: Partial<AiProfile> = {}): AiProfile => ({
  id,
  name,
  harness: "claude",
  model: "",
  settings: {},
  host_id: null,
  is_default: false,
  ...over,
});

const message = (role: "user" | "assistant", content: string): ChatMessage => ({
  id: Math.floor(Math.random() * 1e6),
  chat_id: 1,
  role,
  content,
  input_tokens: null,
  output_tokens: null,
  cost_usd: null,
  created_at: "2026-01-01T00:00:00Z",
});

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ChatPanel reviewId={7} paneCollapsed={false} onToggle={() => {}} pendingMessage={null} onPendingConsumed={() => {}} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
  useSettingsStore.setState({ aiEnabled: true });
  chatSend.mockReset().mockResolvedValue({ text: "ok", input_tokens: 0, output_tokens: 0, cost_usd: 0 });
  chatMessages.mockReset().mockResolvedValue([]);
  chatProfileId.mockReset().mockResolvedValue(1);
  aiProfileList.mockReset().mockResolvedValue([profile(1, "Claude Code", { is_default: true }), profile(2, "Fast", { model: "haiku" })]);
});

describe("ChatPanel profile selector", () => {
  it("shows the profile the chat runs on and sends without a switch by default", async () => {
    const user = userEvent.setup();
    renderPanel();
    const select = (await screen.findByLabelText("Profile")) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("1"));
    await user.type(screen.getByRole("textbox"), "hello");
    await user.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(chatSend).toHaveBeenCalled());
    expect(chatSend.mock.calls[0].slice(0, 3)).toEqual([7, "hello", null]);
  });

  it("passes the chosen profile and warns that switching starts a new session", async () => {
    const user = userEvent.setup();
    chatMessages.mockResolvedValue([message("user", "hi"), message("assistant", "hello")]);
    renderPanel();
    const select = (await screen.findByLabelText("Profile")) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("1"));
    await screen.findByText("hello");
    expect(screen.queryByText(/starts a new session/)).toBeNull();

    await user.selectOptions(select, "2");
    expect(screen.getByText(/starts a new session/)).toBeTruthy();
    await user.type(screen.getByRole("textbox"), "again");
    await user.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(chatSend).toHaveBeenCalled());
    expect(chatSend.mock.calls[0].slice(0, 3)).toEqual([7, "again", 2]);
  });
});
