import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api } from "./api";
import { useSettingsStore } from "./settings";
import type { AiProfile, FeatureProfiles, HarnessInfo } from "./types";

/** Model names offered per harness; free text is always allowed (opencode's list arrives with its harness). */
const MODEL_SUGGESTIONS: Record<string, string[]> = {
  claude: ["sonnet", "opus", "fable", "haiku"],
};

export const modelSuggestions = (harness: string): string[] => MODEL_SUGGESTIONS[harness] ?? [];

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
