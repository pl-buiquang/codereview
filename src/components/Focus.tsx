import { useState } from "react";
import { useMutation, useQueryClient, type UseMutationResult } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import {
  FOCUS_LEVELS,
  digestGroups,
  digestMarkdown,
  hiddenCount,
  type FileFocus,
  type HunkFocus,
} from "../lib/focus";
import type { FocusLevel, FocusOverride, FocusView } from "../lib/types";
import { useSettingsStore } from "../lib/settings";
import { Markdown } from "./Markdown";
import { MODEL_OPTIONS } from "./ChatPanel";

export type GenerateFocus = UseMutationResult<void, Error, { model?: string }>;

/** Starts a background run; completion arrives as a `focus-updated` event. */
export function useGenerateFocus(reviewId: number): GenerateFocus {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ model }) => api.generateFocus(reviewId, model),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["focus", reviewId] }),
    onError: (e) => toast.error(`Could not start focus generation:\n${String(e)}`),
  });
}

export function FocusGenerate({
  view,
  claudeAvailable,
  generate,
}: {
  view: FocusView | null;
  claudeAvailable: boolean;
  generate: GenerateFocus;
}) {
  const chatModel = useSettingsStore((s) => s.chatModel);
  const [open, setOpen] = useState(false);
  const [model, setModel] = useState(chatModel);
  if (view?.focus.status === "generating" || generate.isPending) {
    return (
      <span className="focus-status">
        <span className="spinner" /> Generating focus…
      </span>
    );
  }
  const label = view?.focus.planned_at ? "Regenerate focus" : "Generate focus";
  return (
    <span className="focus-generate">
      <button
        className="btn"
        disabled={!claudeAvailable}
        title={
          claudeAvailable
            ? "Tier every hunk with AI and hide the noise"
            : "Install the `claude` CLI to generate a focus here (`/local-review --focus` also works from the terminal)"
        }
        onClick={() => setOpen((o) => !o)}
      >
        <span className="ai-pill">AI</span> {label}
      </button>
      {open && (
        <div className="focus-generate-pop" role="dialog" aria-label={label}>
          <label>
            Model
            <select
              className="focus-level-select"
              value={model}
              onChange={(e) => setModel(e.target.value)}
            >
              {MODEL_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
          <span className="focus-generate-actions">
            <button className="btn btn-sm btn-ghost" onClick={() => setOpen(false)}>
              Cancel
            </button>
            <button
              className="btn btn-sm btn-primary"
              onClick={() => {
                setOpen(false);
                generate.mutate({ model: model || undefined });
              }}
            >
              Generate
            </button>
          </span>
        </div>
      )}
    </span>
  );
}

export interface FocusActions {
  setLevel: (level: FocusLevel) => void;
  setHunkOverride: (hunkIds: string[], value: FocusOverride | null) => void;
  setFileOverride: (path: string, value: FocusOverride | null) => void;
  clear: () => void;
}

/** Focus mutations, applied optimistically: refetching the focus re-fetches the diff. */
export function useFocusActions(reviewId: number): FocusActions {
  const queryClient = useQueryClient();
  const key = ["focus", reviewId];
  const patch = (fn: (v: FocusView) => FocusView) =>
    queryClient.setQueriesData<FocusView | null>({ queryKey: key }, (old) =>
      old ? fn(old) : old,
    );
  const run = (p: Promise<unknown>) =>
    p.catch((e) => {
      toast.error(String(e));
      queryClient.invalidateQueries({ queryKey: key });
    });

  return {
    setLevel: (level) => {
      patch((v) => ({ ...v, focus: { ...v.focus, level } }));
      run(api.setFocusLevel(reviewId, level));
    },
    setHunkOverride: (hunkIds, value) => {
      const ids = new Set(hunkIds);
      patch((v) => ({
        ...v,
        hunks: v.hunks.map((h) => (ids.has(h.hunk_id) ? { ...h, override: value } : h)),
      }));
      run(Promise.all(hunkIds.map((id) => api.setHunkOverride(reviewId, id, value))));
    },
    setFileOverride: (path, value) => {
      patch((v) => ({
        ...v,
        file_overrides: [
          ...v.file_overrides.filter((o) => o.file_path !== path),
          ...(value ? [{ file_path: path, override: value }] : []),
        ],
      }));
      run(api.setFileOverride(reviewId, path, value));
    },
    clear: async () => {
      if (
        !(await confirmDialog({
          title: "Clear focus",
          message:
            "Remove the focus (tiers, summary and overrides)? AI comments it added that you haven't edited are removed too; comments you wrote or edited are kept.",
          confirmLabel: "Clear focus",
          danger: true,
        }))
      )
        return;
      queryClient.setQueriesData<FocusView | null>({ queryKey: key }, null);
      run(
        api.clearFocus(reviewId).then(() => {
          queryClient.invalidateQueries({ queryKey: ["review", reviewId] });
          queryClient.invalidateQueries({ queryKey: ["reviews"] });
        }),
      );
    },
  };
}

export function FocusLevelSelect({
  level,
  onChange,
}: {
  level: FocusLevel;
  onChange: (l: FocusLevel) => void;
}) {
  return (
    <label className="focus-level" title="Focus level: which hunk tiers are shown">
      <span className="ai-pill">AI</span>
      <select
        className="focus-level-select"
        value={level}
        onChange={(e) => onChange(e.target.value as FocusLevel)}
      >
        {FOCUS_LEVELS.map((l) => (
          <option key={l.value} value={l.value} title={l.title}>
            {l.label}
          </option>
        ))}
      </select>
    </label>
  );
}

const shortSha = (s: string | null) => (s ? s.slice(0, 7) : "?");

export function FocusDigest({
  view,
  model,
  headSha,
  readOnly,
  actions,
  onReveal,
  onJump,
  onCopyToSummary,
  onRegenerate,
}: {
  view: FocusView;
  model: FileFocus[] | null;
  headSha: string | null;
  readOnly: boolean;
  actions: FocusActions;
  onReveal: (hunkIds: string[]) => void;
  onJump: (fileIndex: number) => void;
  onCopyToSummary: (markdown: string) => void;
  onRegenerate?: () => void;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const { focus } = view;
  const planned = focus.planned_at != null;
  const groups = model ? digestGroups(model) : [];
  const hidden = model ? hiddenCount(model) : 0;
  const allHidden = model != null && model.length > 0 && model.every((f) => f.fullyHidden);

  return (
    <div className="focus-digest">
      <div className="focus-digest-head">
        <button
          className="focus-digest-toggle"
          onClick={() => setCollapsed((c) => !c)}
          title={collapsed ? "Show the focus digest" : "Hide the focus digest"}
        >
          <span className="ai-pill">AI</span>
          <span className="focus-digest-title">Focus</span>
          {planned && model && (
            <span className="muted">
              · {hidden} hidden {hidden === 1 ? "item" : "items"}
            </span>
          )}
          {planned && !model && <span className="muted">· off</span>}
        </button>
        {focus.status === "generating" && (
          <span className="focus-status">
            <span className="spinner" /> Generating focus…
          </span>
        )}
        {focus.status === "failed" && (
          <span className="focus-status focus-failed" title={focus.error ?? undefined}>
            Focus generation failed{focus.error ? `: ${focus.error}` : ""}
          </span>
        )}
        {view.stale && (
          <span className="focus-stale">
            Focus generated at <code>{shortSha(focus.head_sha)}</code>; the review is now on{" "}
            <code>{shortSha(headSha)}</code>
            {onRegenerate && (
              <button className="expand-btn" onClick={onRegenerate}>
                Regenerate
              </button>
            )}
          </span>
        )}
        <span className="focus-digest-actions">
          {planned && model && (
            <button
              className="btn btn-sm btn-ghost"
              disabled={readOnly}
              title={
                readOnly
                  ? "Published reviews cannot be edited"
                  : "Insert the digest as markdown into the review summary"
              }
              onClick={() => onCopyToSummary(digestMarkdown(view, model))}
            >
              Copy into summary
            </button>
          )}
          <button className="btn btn-sm btn-ghost" onClick={actions.clear}>
            Clear focus
          </button>
        </span>
      </div>
      {!collapsed && planned && (
        <div className="focus-digest-body">
          {focus.summary && <Markdown source={focus.summary} />}
          {allHidden && (
            <p className="focus-empty">
              Everything is hidden at this level.{" "}
              <button className="expand-btn" onClick={() => actions.setLevel("off")}>
                Show all
              </button>
            </p>
          )}
          {groups.length > 0 && (
            <ul className="focus-groups">
              {groups.map((g) => (
                <li key={g.reason}>
                  <span className="focus-reason">{g.reason}</span>{" "}
                  <span className="muted">({g.items.length})</span>{" "}
                  {g.items.map((it) => (
                    <button
                      key={it.hunk.entry!.hunk_id}
                      className="focus-item mono"
                      title={it.hunk.entry!.header || "whole file"}
                      onClick={() => {
                        onReveal([it.hunk.entry!.hunk_id]);
                        onJump(it.fileIndex);
                      }}
                    >
                      {it.path}
                    </button>
                  ))}
                  <button
                    className="expand-btn"
                    onClick={() => onReveal(g.items.map((it) => it.hunk.entry!.hunk_id))}
                  >
                    show
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

/** Collapsed run of hidden hunks inside a file. */
export function FocusHiddenBar({
  hunks,
  onReveal,
  onAlwaysShow,
}: {
  hunks: HunkFocus[];
  onReveal: () => void;
  onAlwaysShow: () => void;
}) {
  const reasons = [...new Set(hunks.map((h) => h.reason).filter(Boolean))];
  const n = hunks.length;
  return (
    <div className="focus-bar">
      <span className="focus-bar-label">
        ▸ {n} hidden {n === 1 ? "hunk" : "hunks"}
        {reasons.length > 0 ? ` — ${reasons.join(", ")}` : ""}
      </span>
      <button className="expand-btn" onClick={onReveal} title="Show for this session">
        show
      </button>
      <button className="expand-btn" onClick={onAlwaysShow} title="Show at every level (saved)">
        keep shown
      </button>
    </div>
  );
}

/** Tier pill, AI note and hide action above a visible hunk. */
export function FocusHunkMeta({
  hunk,
  onHide,
}: {
  hunk: HunkFocus;
  onHide: () => void;
}) {
  const e = hunk.entry!;
  return (
    <div className="focus-meta">
      <div className="focus-meta-row">
        <span className={`focus-tier focus-tier-${e.tier}`} title={`Tier (${e.source ?? "default"})`}>
          {e.tier}
        </span>
        {e.is_new && (
          <span className="focus-new" title="This hunk changed after the focus was generated">
            new since focus
          </span>
        )}
        {e.reason && <span className="muted focus-meta-reason">{e.reason}</span>}
        {hunk.pinned && <span className="muted focus-meta-reason">· has comments</span>}
        <button className="expand-btn focus-meta-hide" onClick={onHide} title="Hide this hunk">
          hide
        </button>
      </div>
    </div>
  );
}
