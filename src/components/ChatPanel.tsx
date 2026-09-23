import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import type { ChatMessage } from "../lib/types";
import { useSettingsStore } from "../lib/settings";
import { confirmDialog } from "../lib/confirm";
import { Icon } from "./icons";
import { Markdown } from "./Markdown";

interface ChatPanelProps {
  reviewId: number;
  paneCollapsed: boolean;
  onToggle: () => void;
  /** Message pre-filled from "Send to Chat" in the diff composer. */
  pendingMessage: string | null;
  onPendingConsumed: () => void;
}

export function ChatPanel({
  reviewId,
  paneCollapsed,
  onToggle,
  pendingMessage,
  onPendingConsumed,
}: ChatPanelProps) {
  const queryClient = useQueryClient();
  const chatModel = useSettingsStore((s) => s.chatModel);
  const [input, setInput] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const messagesQuery = useQuery({
    queryKey: ["chat-messages", reviewId],
    queryFn: () => api.chatMessages(reviewId),
    refetchOnWindowFocus: false,
  });

  const sendMutation = useMutation({
    mutationFn: (text: string) => api.chatSend(reviewId, text, chatModel || undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["chat-messages", reviewId] });
    },
  });

  const clearMutation = useMutation({
    mutationFn: () => api.chatClear(reviewId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["chat-messages", reviewId] });
      sendMutation.reset();
    },
  });

  // Auto-scroll to bottom when messages update.
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messagesQuery.data, sendMutation.isPending]);

  // Handle "Send to Chat" from the composer.
  useEffect(() => {
    if (pendingMessage === null) return;
    onPendingConsumed();
    if (sendMutation.isPending) return;
    sendMutation.mutate(pendingMessage);
    // Optimistic user message so the panel shows it immediately.
    queryClient.setQueryData(
      ["chat-messages", reviewId],
      (old: ChatMessage[] | undefined) => [
        ...(old ?? []),
        {
          id: -Date.now(),
          chat_id: 0,
          role: "user" as const,
          content: pendingMessage,
          input_tokens: null,
          output_tokens: null,
          cost_usd: null,
          created_at: new Date().toISOString(),
        },
      ],
    );
  }, [pendingMessage]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleSend = () => {
    const text = input.trim();
    if (!text || sendMutation.isPending) return;
    setInput("");
    sendMutation.mutate(text);
    // Optimistic user message.
    queryClient.setQueryData(
      ["chat-messages", reviewId],
      (old: ChatMessage[] | undefined) => [
        ...(old ?? []),
        {
          id: -Date.now(),
          chat_id: 0,
          role: "user" as const,
          content: text,
          input_tokens: null,
          output_tokens: null,
          cost_usd: null,
          created_at: new Date().toISOString(),
        },
      ],
    );
    setTimeout(() => textareaRef.current?.focus(), 0);
  };

  if (paneCollapsed) {
    return (
      <div className="chat-panel chat-panel--collapsed">
        <button
          className="btn btn-sm btn-ghost sidebar-toggle sidebar-expand-btn"
          title="Show chat (')"
          onClick={onToggle}
        >
          <Icon name="bot" size={14} />
        </button>
      </div>
    );
  }

  const messages = messagesQuery.data ?? [];
  const busy = sendMutation.isPending;

  return (
    <div className="chat-panel">
      <div className="chat-header">
        <span className="chat-title">
          <Icon name="bot" size={13} />
          Chat
        </span>
        <div className="chat-header-actions">
          {messages.length > 0 && (
            <button
              className="btn btn-sm btn-ghost"
              title="Clear conversation"
              disabled={busy || clearMutation.isPending}
              onClick={async () => {
                const ok = await confirmDialog({
                  title: "Clear chat?",
                  message: "This will delete the conversation history and start a fresh session.",
                  confirmLabel: "Clear",
                  danger: true,
                });
                if (ok) clearMutation.mutate();
              }}
            >
              Clear
            </button>
          )}
          <button
            className="btn btn-sm btn-ghost sidebar-toggle"
            title="Hide chat (')"
            onClick={onToggle}
          >
            <Icon name="x" size={12} />
          </button>
        </div>
      </div>

      <div className="chat-messages">
        {messages.length === 0 && !busy && (
          <p className="chat-empty muted">
            Ask anything about this review — the assistant can read files and
            run git commands at the reviewed revision.
          </p>
        )}
        {messages.map((msg) => (
          <div
            key={msg.id}
            className={`chat-message chat-message--${msg.role}`}
          >
            {msg.role === "assistant" ? (
              <Markdown source={msg.content} />
            ) : (
              <p className="chat-message-text">{msg.content}</p>
            )}
            {msg.role === "assistant" && msg.output_tokens != null && (
              <span className="chat-usage">{msg.output_tokens} tok</span>
            )}
          </div>
        ))}
        {busy && (
          <div className="chat-message chat-message--assistant">
            <div className="thinking-indicator">
              <span />
              <span />
              <span />
            </div>
          </div>
        )}
        {sendMutation.isError && (
          <div className="chat-error">
            {String(sendMutation.error)}
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      <div className="chat-input-area">
        <textarea
          ref={textareaRef}
          className="chat-input"
          placeholder="Ask about the changes… (⌘↵ to send)"
          value={input}
          disabled={busy}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.altKey)) {
              e.preventDefault();
              handleSend();
            }
          }}
          rows={3}
        />
        <button
          className="btn btn-primary btn-sm"
          disabled={!input.trim() || busy}
          onClick={handleSend}
        >
          Send
        </button>
      </div>
    </div>
  );
}
