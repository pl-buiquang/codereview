import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api } from "./api";
import { useSettingsStore } from "./settings";
import type { AiProfile, FeatureProfiles, HarnessInfo, ModelGroup } from "./types";

/** The command that adds a provider to opencode (shown, with a copy button, when it lists no model). */
export const OPENCODE_LOGIN_COMMAND = "opencode auth login";

/** The groups with only the models containing `query` (case-insensitive; a provider name matches all of its models). Empty groups are dropped. */
export function filterModelGroups(groups: ModelGroup[], query: string): ModelGroup[] {
  const q = query.trim().toLowerCase();
  if (!q) return groups;
  return groups
    .map((g) => ({ provider: g.provider, models: g.provider.toLowerCase().includes(q) ? g.models : g.models.filter((m) => m.toLowerCase().includes(q)) }))
    .filter((g) => g.models.length > 0);
}

/** Which profile fields the editor shows for a harness. The model applies to every harness; the
 *  harness-specific settings JSON only to those that have profile-level options (Claude Code has none). */
export function profileEditorFields(info: HarnessInfo | undefined): { model: boolean; settings: boolean } {
  return { model: true, settings: info != null && info.id !== "claude" };
}

/** The features that pick a profile, in the order of the Settings table. */
export const FEATURE_ROWS: { key: keyof FeatureProfiles; label: string; hint: string }[] = [
  { key: "chat", label: "Review chat", hint: "The review chat. Chats pin the profile they started with." },
  { key: "summaries", label: "Session summaries", hint: "Live session summaries. Without a profile model, a fast built-in model is used." },
  { key: "theme", label: "Theme suggestions", hint: "AI suggestions in Settings → Theme. Without a profile model, a built-in model is used." },
  { key: "sessions", label: "Interactive sessions", hint: "The agent opened in a terminal from a card or review." },
];

/** A profile's one-line description for lists and pickers. */
export const profileLabel = (p: Pick<AiProfile, "name" | "model">): string => (p.model ? `${p.name} · ${p.model}` : p.name);

/** Hand the chat / overview model fields of the old settings to the backend once, then forget them. */
export function useImportLegacyAiModels() {
  const legacy = useSettingsStore((s) => s.legacyAiModels);
  const clear = useSettingsStore((s) => s.clearLegacyAiModels);
  const queryClient = useQueryClient();
  useEffect(() => {
    if (!legacy) return;
    let cancelled = false;
    api
      .aiImportLegacyModels(legacy.chat, legacy.overview)
      .then(() => {
        if (cancelled) return;
        clear();
        queryClient.invalidateQueries({ queryKey: ["ai-profiles"] });
        queryClient.invalidateQueries({ queryKey: ["routines"] });
      })
      .catch((e) => console.warn("Could not import the AI model settings", e));
    return () => {
      cancelled = true;
    };
  }, [legacy, clear, queryClient]);
}
