export const AI_SUBTAGS = ["ai:explain", "ai:review", "ai:codex"] as const;
export type AiSubtag = (typeof AI_SUBTAGS)[number];
export type CommentTag = "ai" | AiSubtag | "local";

export function isAiTag(tag: CommentTag | null | undefined): boolean {
  return tag === "ai" || (tag != null && tag.startsWith("ai:"));
}

export function tagLabel(tag: CommentTag): string {
  switch (tag) {
    case "ai":
      return "AI";
    case "ai:explain":
      return "Explain";
    case "ai:review":
      return "Review";
    case "ai:codex":
      return "Codex";
    case "local":
      return "Local";
  }
}

export function tagTitle(tag: CommentTag): string {
  return tag === "local" ? "Local only; never published" : "Written by AI; not published";
}
