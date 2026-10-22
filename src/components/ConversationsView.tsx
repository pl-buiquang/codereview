import { useEffect, useMemo, useState } from "react";
import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Diff, Hunk } from "react-diff-view";
import { api } from "../lib/api";
import { hunkSnippetFile, tokenizeFile } from "../lib/diff";
import { parseBotLogins, parseRepoBasePaths, useSettingsStore } from "../lib/settings";
import { timeAgo } from "../lib/timeAgo";
import { toast } from "../lib/toast";
import { useUIStore } from "../store";
import type { Conversation, DiscussionComment } from "../lib/types";
import { GithubThread, ThreadComment } from "./GithubThread";
import { Icon } from "./icons";
import { MultiFilterList } from "./RailFilters";
import { Composer } from "./ReviewView";
import { RepoName } from "./RepoName";
import { TagFilter } from "./TagFilter";

const PAGE = 100;
const EPOCH = "1970-01-01T00:00:00Z";

/** Conversations tab of the Inbox: review threads and top-level discussions of
 *  the PRs/issues I'm notified about or track on the Board. */
export function ConversationsView() {
  const queryClient = useQueryClient();
  const filters = useUIStore((s) => s.conversationFilters);
  const setFilters = useUIStore((s) => s.setConversationFilters);
  const openReview = useUIStore((s) => s.openReview);
  const requestThreadJump = useUIStore((s) => s.requestThreadJump);
  const botLogins = useSettingsStore((s) => s.botLogins);
  const repoBasePaths = useSettingsStore((s) => s.repoBasePaths);
  const [limit, setLimit] = useState(PAGE);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const filtersKey = JSON.stringify(filters);
  useEffect(() => setLimit(PAGE), [filtersKey]);

  const bots = useMemo(() => [...parseBotLogins(botLogins)], [botLogins]);
  const rules = useSettingsStore((s) => s.conversationIgnoreRules);
  const ruleNames = useMemo(() => new Map(rules.map((r) => [r.id, r.name])), [rules]);
  const query = useQuery({
    queryKey: ["conversations", filters, bots, rules, limit],
    queryFn: () => api.listConversations(filters, bots, rules, limit),
    placeholderData: keepPreviousData,
  });
  const items = query.data?.items ?? [];
  const facets = query.data?.facets;
  const total = query.data?.total ?? 0;

  const refresh = () => queryClient.invalidateQueries({ queryKey: ["conversations"] });
  const onErr = (e: unknown) => toast.error(String(e));

  const pin = useMutation({
    mutationFn: (c: Conversation) => api.setConversationPinned(c.id, c.subject.id, !c.pinned),
    onSuccess: refresh,
    onError: onErr,
  });
  const seen = useMutation({
    mutationFn: ({ c, read }: { c: Conversation; read: boolean }) => {
      const times = (c.thread?.comments ?? c.comments).map((x) => x.createdAt);
      return api.setConversationSeen(c.id, c.subject.id, read ? c.lastCommentAt : (times[times.length - 2] ?? EPOCH));
    },
    onSuccess: refresh,
    onError: onErr,
  });

  const toggle = (c: Conversation) => {
    const open = !expanded.has(c.id);
    setExpanded((prev) => {
      const next = new Set(prev);
      if (open) next.add(c.id);
      else next.delete(c.id);
      return next;
    });
    if (open && c.unreadCount > 0) seen.mutate({ c, read: true });
  };

  const openInReview = useMutation({
    mutationFn: async (c: Conversation) => {
      const { owner, name, number } = c.subject;
      if (c.kind === "thread") requestThreadJump({ owner, name, number, threadId: c.id });
      const review = await api.createReviewForPr(owner, name, number, parseRepoBasePaths(repoBasePaths));
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      return review.id;
    },
    onSuccess: (id) => openReview(id),
    onError: (e) => toast.error(`Could not open review:\n${String(e)}`),
  });

  const subjectEntries: [string, number][] = (facets?.subjects ?? [])
    .map((s) => [s.subject.id, s.count] as [string, number])
    .sort((a, b) => b[1] - a[1]);
  const subjectById = new Map((facets?.subjects ?? []).map((s) => [s.subject.id, s.subject]));
  const flag = (label: string, count: number | undefined, on: boolean, toggleFlag: () => void) => (
    <button className={`cr-rail-item${on ? " on" : ""}`} aria-pressed={on} onClick={toggleFlag}>
      <span className="lbl">{label}</span>
      <span className="count">{count ?? 0}</span>
    </button>
  );

  return (
    <div className="inbox-layout">
      <aside className="cr-rail">
        <div className="cr-rail-group">
          <div className="cr-rail-h">
            <span>Show</span>
          </div>
          {flag("Pinned", facets?.pinned, filters.pinned, () => setFilters({ pinned: !filters.pinned }))}
          {flag("Unread", facets?.unread, filters.unread, () => setFilters({ unread: !filters.unread }))}
          {flag("Unresolved threads", facets?.unresolved, filters.unresolved, () =>
            setFilters({ unresolved: !filters.unresolved }),
          )}
          {flag("Bot conversations", facets?.bots, filters.showBots, () => setFilters({ showBots: !filters.showBots }))}
          {flag("Ignored by rules", facets?.ignored, filters.showIgnored, () =>
            setFilters({ showIgnored: !filters.showIgnored }),
          )}
        </div>
        <TagFilter
          title="Pull requests / issues"
          placeholder="Search PR or issue…"
          tags={subjectEntries}
          selected={filters.subjectIds}
          onChange={(subjectIds) => setFilters({ subjectIds })}
          label={(id) => {
            const s = subjectById.get(id);
            return s ? `${s.name}#${s.number} ${s.title}` : id;
          }}
        />
        <MultiFilterList
          title="Repositories"
          entries={facets?.repos ?? []}
          selected={filters.repos}
          onChange={(repos) => setFilters({ repos })}
          renderLabel={(key) => <RepoName className="lbl mono" name={key} />}
        />
      </aside>

      <div className="cr-list">
        {query.isLoading && <p className="muted">Loading…</p>}
        {!query.isLoading && items.length === 0 && (
          <p className="muted">
            {total === 0 && !facets?.subjects.length
              ? "No conversations yet. Hit Sync to fetch your GitHub notifications and tracked PRs."
              : "Nothing matches these filters."}
          </p>
        )}
        {items.map((c) => (
          <ConversationRow
            key={c.id}
            c={c}
            open={expanded.has(c.id)}
            busy={openInReview.isPending}
            canOpenReview={c.subject.kind === "pr"}
            ruleNames={ruleNames}
            onToggle={() => toggle(c)}
            onPin={() => pin.mutate(c)}
            onSeen={(read) => seen.mutate({ c, read })}
            onOpenReview={() => openInReview.mutate(c)}
            onChanged={refresh}
          />
        ))}
        {items.length < total && (
          <button className="btn" onClick={() => setLimit((n) => n + PAGE)}>
            Load more ({total - items.length} more)
          </button>
        )}
      </div>
    </div>
  );
}

function ConversationRow({
  c,
  open,
  busy,
  canOpenReview,
  ruleNames,
  onToggle,
  onPin,
  onSeen,
  onOpenReview,
  onChanged,
}: {
  c: Conversation;
  open: boolean;
  busy: boolean;
  canOpenReview: boolean;
  ruleNames: Map<string, string>;
  onToggle: () => void;
  onPin: () => void;
  onSeen: (read: boolean) => void;
  onOpenReview: () => void;
  onChanged: () => void;
}) {
  const { subject } = c;
  const unread = c.unreadCount > 0;
  return (
    <div className={`card conv-row${unread ? " unread" : ""}${c.pinned ? " pinned" : ""}`}>
      <div className="conv-head">
        <button className="conv-toggle" onClick={onToggle} aria-expanded={open}>
          <Icon name="chev" size={12} className={open ? "" : "conv-chev-closed"} />
          <div className="conv-title">
            <div className="conv-meta">
              <RepoName className="repo mono" name={`${subject.owner}/${subject.name}`} />
              <span className="mono">#{subject.number}</span>
              <span className="conv-subject">{subject.title}</span>
            </div>
            <div className="conv-meta">
              {c.kind === "thread" ? (
                <span className="chip mono">
                  {c.path}
                  {c.line != null ? `:${c.line}` : ""}
                </span>
              ) : (
                <span className="chip">Discussion</span>
              )}
              {c.isResolved && <span className="chip">resolved</span>}
              {c.isOutdated && <span className="chip">outdated</span>}
              {c.isBot && <span className="chip">bot</span>}
              {c.ignoredBy.length > 0 && (
                <span className="chip" title="Hidden by an ignore rule">
                  ignored · {c.ignoredBy.map((id) => ruleNames.get(id) ?? id).join(", ")}
                </span>
              )}
              {unread && <span className="chip accent">{c.unreadCount} new</span>}
              <span className="muted">
                {c.commentCount} comment{c.commentCount === 1 ? "" : "s"}
                {c.lastAuthor ? ` · ${c.lastAuthor}` : ""} · {timeAgo(c.lastCommentAt)}
              </span>
            </div>
          </div>
        </button>
        <div className="conv-actions">
          <button
            className={`btn-icon${c.pinned ? " on" : ""}`}
            title={c.pinned ? "Unpin" : "Pin"}
            aria-pressed={c.pinned}
            onClick={onPin}
          >
            <Icon name="pin" size={13} />
          </button>
          <button className="btn-icon" title={unread ? "Mark read" : "Mark unread"} onClick={() => onSeen(unread)}>
            <Icon name={unread ? "check" : "dot"} size={13} />
          </button>
          <button className="btn-icon" title="Open on GitHub" onClick={() => api.openUrl(c.url)}>
            <Icon name="ext" size={13} />
          </button>
          {canOpenReview && (
            <button className="btn" disabled={busy} onClick={onOpenReview}>
              {c.kind === "thread" ? "Open in review" : "Review"}
            </button>
          )}
        </div>
      </div>
      {open && (
        <div className="conv-body">
          {c.kind === "thread" && c.thread ? (
            <>
              {c.path && c.diffHunk && <HunkSnippet path={c.path} diffHunk={c.diffHunk} />}
              <GithubThread
                thread={c.thread}
                ctx={{ owner: subject.owner, name: subject.name, number: subject.number }}
                onChanged={() => {
                  // Re-read the thread from GitHub; that fills the mirror and emits mirror-updated.
                  api.prReviewThreads(subject.owner, subject.name, subject.number).catch(() => {});
                  onChanged();
                }}
              />
            </>
          ) : (
            <Discussion c={c} onChanged={onChanged} />
          )}
        </div>
      )}
    </div>
  );
}

function HunkSnippet({ path, diffHunk }: { path: string; diffHunk: string }) {
  const file = useMemo(() => hunkSnippetFile(path, diffHunk), [path, diffHunk]);
  const tokens = useMemo(() => (file ? tokenizeFile(file) : undefined), [file]);
  if (!file) return null;
  return (
    <div className="conv-snippet">
      <Diff viewType="unified" diffType={file.type} hunks={file.hunks} tokens={tokens}>
        {(hunks) => hunks.map((hunk) => <Hunk key={hunk.content} hunk={hunk} />)}
      </Diff>
    </div>
  );
}

function Discussion({ c, onChanged }: { c: Conversation; onChanged: () => void }) {
  const [replying, setReplying] = useState(false);
  const { subject } = c;
  return (
    <div className="github-thread">
      {c.comments.map((d) => (
        <DiscussionItem key={d.id} comment={d} />
      ))}
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
                onChanged();
              } catch (e) {
                toast.error(`Comment failed:\n${String(e)}`);
              }
            }}
            onCancel={() => setReplying(false)}
          />
        )}
      </div>
    </div>
  );
}

function DiscussionItem({ comment }: { comment: DiscussionComment }) {
  if (comment.kind === "comment") return <ThreadComment comment={comment} />;
  return (
    <div>
      <span className="chip">review · {(comment.reviewState ?? "commented").toLowerCase().replace(/_/g, " ")}</span>
      <ThreadComment comment={comment} />
    </div>
  );
}
