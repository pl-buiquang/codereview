import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import { parseBotLogins, useSettingsStore } from "./settings";
import { EMPTY_CONVERSATION_FILTERS } from "../store";

/** New inbox items not yet viewed or triaged, and unread conversations (bots hidden, ignore rules
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
  const newItems = (inbox.data ?? []).filter((i) => !i.viewed_at).length;
  const unreadConversations = conversations.data?.facets.unread ?? 0;
  return { newItems, unreadConversations, total: newItems + unreadConversations };
}
