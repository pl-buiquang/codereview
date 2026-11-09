import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { AiProfile, HarnessInfo } from "../lib/types";

const aiProfileList = vi.fn();
const aiProfileSave = vi.fn();
const aiProfileDelete = vi.fn();
const aiHarnesses = vi.fn();
const aiHarnessModels = vi.fn();
const aiFeatureProfilesGet = vi.fn();
const aiFeatureProfilesSet = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    aiProfileList: (...a: unknown[]) => aiProfileList(...a),
    aiProfileSave: (...a: unknown[]) => aiProfileSave(...a),
    aiProfileDelete: (...a: unknown[]) => aiProfileDelete(...a),
    aiHarnesses: (...a: unknown[]) => aiHarnesses(...a),
    aiHarnessModels: (...a: unknown[]) => aiHarnessModels(...a),
    aiFeatureProfilesGet: (...a: unknown[]) => aiFeatureProfilesGet(...a),
    aiFeatureProfilesSet: (...a: unknown[]) => aiFeatureProfilesSet(...a),
  },
}));
vi.mock("../lib/confirm", () => ({ confirmDialog: () => Promise.resolve(true) }));

import { AiProfilesSection } from "./SettingsAiProfiles";

const profile = (over: Partial<AiProfile> = {}): AiProfile => ({
  id: 1,
  name: "Claude Code",
  harness: "claude",
  model: "",
  settings: {},
  host_id: null,
  is_default: true,
  ...over,
});

const claude: HarnessInfo = {
  id: "claude",
  name: "Claude Code",
  available: true,
  version: "2.1.0 (Claude Code)",
  caps: {
    sessions: true,
    resume: true,
    fork: true,
    tools: true,
    tool_allowlist: true,
    streaming: true,
    interactive_tui: true,
    reports_cost: true,
    replaces_system_prompt: true,
    access_levels: ["default", "read_only", "edit", "full"],
    pins_session_id: true,
  },
};

const opencode: HarnessInfo = {
  ...claude,
  id: "opencode",
  name: "opencode",
  available: false,
  version: null,
  caps: { ...claude.caps, fork: false, tool_allowlist: false, replaces_system_prompt: false },
};

function renderSection() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <AiProfilesSection />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  aiProfileList.mockReset().mockResolvedValue([profile(), profile({ id: 2, name: "Fast", model: "haiku", is_default: false })]);
  aiProfileSave.mockReset().mockImplementation((p) => Promise.resolve(profile(p)));
  aiProfileDelete.mockReset().mockResolvedValue(undefined);
  aiHarnesses.mockReset().mockResolvedValue([claude, opencode]);
  aiHarnessModels.mockReset().mockImplementation((harness: string) =>
    Promise.resolve(harness === "claude" ? [{ provider: "", models: ["sonnet", "opus"] }] : []),
  );
  aiFeatureProfilesGet.mockReset().mockResolvedValue({ chat: null, summaries: null, theme: null, sessions: null });
  aiFeatureProfilesSet.mockReset().mockResolvedValue(undefined);
});

describe("AiProfilesSection", () => {
  it("shows harness availability with its version", async () => {
    renderSection();
    const panel = await screen.findByTestId("ai-harnesses");
    await within(panel).findByText("found");
    expect(within(panel).getByText("2.1.0 (Claude Code)")).toBeTruthy();
  });

  it("lists the profiles and marks the default", async () => {
    renderSection();
    const panel = await screen.findByTestId("ai-profiles");
    await within(panel).findByText("Fast");
    expect(within(panel).getByText("default")).toBeTruthy();
    expect(within(panel).getByText(/haiku/)).toBeTruthy();
  });

  it("adds a profile with a model and offers no settings field for Claude Code", async () => {
    const user = userEvent.setup();
    renderSection();
    await within(await screen.findByTestId("ai-profiles")).findByText("Fast");
    await user.click(screen.getByRole("button", { name: "Add profile" }));
    await user.type(screen.getByLabelText("Name"), "Opus");
    await user.type(screen.getByLabelText("Model"), "opus");
    expect(screen.queryByLabelText(/^Settings \(JSON/)).toBeNull();
    await user.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(aiProfileSave).toHaveBeenCalled());
    expect(aiProfileSave.mock.calls[0][0]).toMatchObject({ id: null, name: "Opus", harness: "claude", model: "opus", settings: {} });
  });

  it("makes another profile the default and deletes a non-default one", async () => {
    const user = userEvent.setup();
    renderSection();
    await within(await screen.findByTestId("ai-profiles")).findByText("Fast");
    await user.click(screen.getByRole("button", { name: "Make default" }));
    await waitFor(() => expect(aiProfileSave).toHaveBeenCalled());
    expect(aiProfileSave.mock.calls[0][0]).toMatchObject({ id: 2, is_default: true });
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(aiProfileDelete).toHaveBeenCalled());
    expect(aiProfileDelete.mock.calls[0][0]).toBe(2);
  });

  it("maps a feature to a profile, keeping the other mappings", async () => {
    const user = userEvent.setup();
    aiFeatureProfilesGet.mockResolvedValue({ chat: null, summaries: null, theme: 2, sessions: null });
    renderSection();
    await within(await screen.findByTestId("ai-profiles")).findByText("Fast");
    const table = screen.getByTestId("ai-feature-profiles");
    await user.selectOptions(within(table).getByLabelText("Review chat"), "Fast");
    await waitFor(() => expect(aiFeatureProfilesSet).toHaveBeenCalled());
    expect(aiFeatureProfilesSet.mock.calls[0][0]).toEqual({ chat: 2, summaries: null, theme: 2, sessions: null });
    await user.selectOptions(within(table).getByLabelText("Session summaries"), "Default profile");
    await waitFor(() => expect(aiFeatureProfilesSet).toHaveBeenCalledTimes(2));
  });

  it("picks an opencode model from a searchable, provider-grouped list", async () => {
    const user = userEvent.setup();
    aiHarnessModels.mockImplementation((harness: string) =>
      Promise.resolve(
        harness === "opencode"
          ? [
              { provider: "openrouter", models: ["openrouter/anthropic/claude-haiku-4.5", "openrouter/google/gemma"] },
              { provider: "opencode", models: ["opencode/beam-free"] },
            ]
          : [],
      ),
    );
    renderSection();
    await within(await screen.findByTestId("ai-profiles")).findByText("Fast");
    await user.click(screen.getByRole("button", { name: "Add profile" }));
    await user.selectOptions(screen.getByLabelText("Harness"), "opencode");
    await user.type(screen.getByLabelText("Name"), "Router");
    await user.click(screen.getByLabelText("Model"));
    const list = await screen.findByRole("listbox", { name: "Models" });
    expect(within(list).getByText("openrouter")).toBeTruthy();
    expect(within(list).getByText("opencode")).toBeTruthy();
    await user.type(screen.getByLabelText("Model"), "haiku");
    expect(within(list).queryByText("opencode/beam-free")).toBeNull();
    await user.click(within(list).getByText("openrouter/anthropic/claude-haiku-4.5"));
    expect((screen.getByLabelText("Model") as HTMLInputElement).value).toBe("openrouter/anthropic/claude-haiku-4.5");
    await user.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() => expect(aiProfileSave).toHaveBeenCalled());
    expect(aiProfileSave.mock.calls[0][0]).toMatchObject({ harness: "opencode", model: "openrouter/anthropic/claude-haiku-4.5" });
  });

  it("hints at opencode auth login when opencode lists no model, and refreshes on demand", async () => {
    const user = userEvent.setup();
    renderSection();
    await within(await screen.findByTestId("ai-profiles")).findByText("Fast");
    await user.click(screen.getByRole("button", { name: "Add profile" }));
    await user.selectOptions(screen.getByLabelText("Harness"), "opencode");
    expect(await screen.findByText("opencode auth login")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Copy command" })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Refresh models" }));
    await waitFor(() => expect(aiHarnessModels).toHaveBeenCalledWith("opencode", true));
  });
});
