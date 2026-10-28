import { useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import type { ReactionContent, ReactionGroup } from "../lib/types";

export const REACTION_EMOJI: Record<ReactionContent, string> = {
  THUMBS_UP: "👍",
  THUMBS_DOWN: "👎",
  LAUGH: "😄",
  HOORAY: "🎉",
  CONFUSED: "😕",
  HEART: "❤️",
  ROCKET: "🚀",
  EYES: "👀",
};
const ORDER = Object.keys(REACTION_EMOJI) as ReactionContent[];

/** Emoji reactions of a GitHub comment. Pills toggle my reaction; the trailing
 *  picker adds one. Acts on GitHub directly; the mutation result replaces the
 *  local copy, which is re-seeded whenever the fetched `reactions` change.
 *  `discussionSubjectId` is the PR/issue node id for top-level discussion
 *  comments, so their mirrored facet is refreshed too. */
export function Reactions({
  subjectId,
  reactions,
  interactive,
  discussionSubjectId,
  onChanged,
}: {
  subjectId: string;
  reactions?: ReactionGroup[];
  interactive: boolean;
  discussionSubjectId?: string;
  onChanged?: () => void;
}) {
  const [groups, setGroups] = useState<ReactionGroup[]>(reactions ?? []);
  const [pending, setPending] = useState(false);
  const [picking, setPicking] = useState(false);
  const rootRef = useRef<HTMLSpanElement>(null);

  useEffect(() => setGroups(reactions ?? []), [reactions]);

  useEffect(() => {
    if (!picking) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setPicking(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setPicking(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [picking]);

  const toggle = async (content: ReactionContent) => {
    if (pending) return;
    const mine = groups.find((g) => g.content === content)?.viewerHasReacted ?? false;
    setPending(true);
    setPicking(false);
    try {
      const next = await api.setCommentReaction(subjectId, content, !mine, discussionSubjectId);
      setGroups(next);
      onChanged?.();
    } catch (e) {
      toast.error(`Reaction failed:\n${String(e)}`);
    } finally {
      setPending(false);
    }
  };

  const shown = ORDER.map((c) => groups.find((g) => g.content === c)).filter(
    (g): g is ReactionGroup => !!g && g.count > 0,
  );
  if (shown.length === 0 && !interactive) return null;

  return (
    <span className="reactions" ref={rootRef}>
      {shown.map((g) => (
        <button
          key={g.content}
          className={`reaction-pill${g.viewerHasReacted ? " on" : ""}`}
          disabled={!interactive || pending}
          title={g.viewerHasReacted ? "Remove reaction" : `React with ${REACTION_EMOJI[g.content]}`}
          aria-pressed={g.viewerHasReacted}
          onClick={() => toggle(g.content)}
        >
          {REACTION_EMOJI[g.content]} {g.count}
        </button>
      ))}
      {interactive && (
        <span className="reaction-add">
          <button
            className="reaction-pill reaction-add-btn"
            disabled={pending}
            title="Add reaction"
            aria-label="Add reaction"
            aria-expanded={picking}
            onClick={() => setPicking((v) => !v)}
          >
            <span aria-hidden>☺</span>
            <span aria-hidden>+</span>
          </button>
          {picking && (
            <span className="reaction-picker">
              {ORDER.map((c) => (
                <button
                  key={c}
                  className="reaction-pick"
                  title={c.toLowerCase().replace("_", " ")}
                  onClick={() => toggle(c)}
                >
                  {REACTION_EMOJI[c]}
                </button>
              ))}
            </span>
          )}
        </span>
      )}
    </span>
  );
}
