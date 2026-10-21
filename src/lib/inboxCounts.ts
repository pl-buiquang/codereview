import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import { parseBotLogins, useSettingsStore } from "./settings";
import { EMPTY_CONVERSATION_FILTERS } from "../store";

/** Inbox items that arrived since the New tab was last on screen, and unread conversations (bots hidden, ignore rules
 *  applied), for the tab and sidebar badges. Both are mirror-backed queries,
 *  so a sync refreshes them. */
export function useInboxCounts() {
  const botLogins = useSettingsStore((s) => s.botLogins);
  const ignoreRules = useSettingsStore((s) => s.conversationIgnoreRules);
  const inbox = useQuery({ queryKey: ["inbox"], queryFn: api.listInbox });
  const conversations = useQuery({
    queryKey: ["conversations", "badge", botLogins, ignoreRules],
    queryFn: () => api.listConversations(EMPTY_CONVERSATION_FILTERS, [...parseBotLogins(botLogins)], ignoreRules, 1),
  });
  const meta = useQuery({ queryKey: ["inbox-meta"], queryFn: api.inboxMeta });
  // New = first seen since the New tab was last on screen (until the meta loads, nothing).
  const seenAt = meta.data ? Date.parse(meta.data.newSeenAt) : Infinity;
  const newItems = (inbox.data ?? []).filter((i) => Date.parse(i.first_seen_at) > seenAt).length;
  const unreadConversations = conversations.data?.facets.unread ?? 0;
  return { newItems, unreadConversations, total: newItems + unreadConversations };
}
