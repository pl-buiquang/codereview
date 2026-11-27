import { useState, type ReactNode } from "react";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import type { ConversationSubject, DiscussionComment } from "../lib/types";
import { ThreadComment } from "./GithubThread";
import { Composer } from "./ReviewView";

export function DiscussionItem({
  comment,
  subjectId,
  onChanged,
  extra,
}: {
  comment: DiscussionComment;
  subjectId: string;
  onChanged: () => void;
  extra?: ReactNode;
}) {
  const body = <ThreadComment comment={comment} interactive discussionSubjectId={subjectId} onReacted={onChanged} />;
  if (comment.kind === "comment" && extra == null) return body;
  return (
    <div>
      {comment.kind === "review" && (
        <span className="chip">review · {(comment.reviewState ?? "commented").toLowerCase().replace(/_/g, " ")}</span>
      )}
      {extra}
      {body}
    </div>
  );
}

export function DiscussionReply({
  subject,
  onPosted,
}: {
  subject: Pick<ConversationSubject, "id" | "owner" | "name" | "number">;
  onPosted: () => void;
}) {
  const [replying, setReplying] = useState(false);
  return (
    <div className="github-thread-reply">
      {!replying ? (
        <button className="github-thread-action" onClick={() => setReplying(true)}>
          Comment…
        </button>
      ) : (
        <Composer
          submitLabel="Comment"
          onSubmit={async (text) => {
            try {
              await api.replyToDiscussion(subject.id, subject.owner, subject.name, subject.number, text);
              setReplying(false);
              onPosted();
            } catch (e) {
              toast.error(`Comment failed:\n${String(e)}`);
            }
          }}
          onCancel={() => setReplying(false)}
        />
      )}
    </div>
  );
}
