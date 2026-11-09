import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createElement, type ReactNode } from "react";
import type { HarnessInfo } from "./types";

const aiImportLegacyModels = vi.fn();
vi.mock("./api", () => ({ api: { aiImportLegacyModels: (...a: unknown[]) => aiImportLegacyModels(...a) } }));

import { modelSuggestions, profileEditorFields, profileLabel, useImportLegacyAiModels } from "./aiProfiles";
import { useSettingsStore } from "./settings";

const info = (id: string): HarnessInfo => ({
  id,
  name: id,
  available: true,
  version: null,
  caps: {
    sessions: true,
    resume: true,
    fork: false,
    tools: true,
    tool_allowlist: true,
    streaming: true,
    interactive_tui: true,
    reports_cost: true,
    replaces_system_prompt: true,
    access_levels: ["read_only"],
    pins_session_id: true,
  },
});

describe("profile editor", () => {
  it("shows the model for every harness and settings only where a harness has any", () => {
    expect(profileEditorFields(info("claude"))).toEqual({ model: true, settings: false });
    expect(profileEditorFields(info("codex"))).toEqual({ model: true, settings: true });
    expect(profileEditorFields(undefined)).toEqual({ model: true, settings: false });
  });

  it("suggests Claude models and nothing for unknown harnesses", () => {
    expect(modelSuggestions("claude")).toContain("opus");
    expect(modelSuggestions("zed")).toEqual([]);
  });

  it("labels a profile with its model when it has one", () => {
    expect(profileLabel({ name: "Fast", model: "haiku" })).toBe("Fast · haiku");
    expect(profileLabel({ name: "Claude Code", model: "" })).toBe("Claude Code");
  });
});

describe("useImportLegacyAiModels", () => {
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: new QueryClient() }, children);

  beforeEach(() => {
    aiImportLegacyModels.mockReset();
    useSettingsStore.setState({ legacyAiModels: null });
  });

  it("does nothing without pending models", () => {
    renderHook(() => useImportLegacyAiModels(), { wrapper });
    expect(aiImportLegacyModels).not.toHaveBeenCalled();
  });

  it("imports the pending models once and clears them", async () => {
    aiImportLegacyModels.mockResolvedValue(true);
    useSettingsStore.setState({ legacyAiModels: { chat: "opus", overview: "sonnet" } });
    renderHook(() => useImportLegacyAiModels(), { wrapper });
    await waitFor(() => expect(useSettingsStore.getState().legacyAiModels).toBeNull());
    expect(aiImportLegacyModels).toHaveBeenCalledWith("opus", "sonnet");
  });

  it("keeps them for the next start when the import fails", async () => {
    aiImportLegacyModels.mockRejectedValue(new Error("db busy"));
    vi.spyOn(console, "warn").mockImplementation(() => {});
    useSettingsStore.setState({ legacyAiModels: { chat: "opus", overview: "" } });
    renderHook(() => useImportLegacyAiModels(), { wrapper });
    await waitFor(() => expect(aiImportLegacyModels).toHaveBeenCalled());
    expect(useSettingsStore.getState().legacyAiModels).toEqual({ chat: "opus", overview: "" });
  });
});
