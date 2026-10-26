import { useEffect, useRef, useState, type ReactNode } from "react";
import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSettingsStore } from "../lib/settings";
import { autoSyncRefetch } from "../lib/sync";
import { Markdown } from "./Markdown";
import { toast } from "../lib/toast";
import { useDebouncedCallback } from "../lib/useDebouncedCallback";
import type { PrActor, PrCheck, PrLabel, PrMeta, PrReviewRequest } from "../lib/types";

/** Shared with the review header's status chips, which need the same PR
 *  metadata even while this panel is collapsed — same query key, so there's
 *  only one fetch. Shows the mirrored copy until the live fetch lands. `owner`/`name`/`number` may be null for a non-PR target.
 *  Hidden review tabs stay mounted, so only the active tab fetches: an
 *  inactive tab keeps its cached data and catches up when activated. */
export function usePrMeta(
  owner: string | null,
  name: string | null,
  number: number | null,
  active = true,
) {
  const known = owner != null && name != null && number != null;
  const autoSync = useSettingsStore((s) => s.autoSync);
  // The mirror's copy renders at once while the live fetch runs.
  const mirrored = useQuery({
    queryKey: ["pr-meta-mirror", owner, name, number],
    queryFn: () => api.mirrorPrMeta(owner as string, name as string, number as number),
    enabled: known,
    staleTime: Infinity,
  });
  return useQuery({
    queryKey: ["pr-meta", owner, name, number],
    queryFn: () => api.prMeta(owner as string, name as string, number as number),
    enabled: active && known,
    staleTime: 30_000,
    ...autoSyncRefetch(autoSync, 3 * 60_000),
    placeholderData: mirrored.data ?? undefined,
  });
}

export function PrMetaPanel({
  owner,
  name,
  number,
  active = true,
}: {
  owner: string;
  name: string;
  number: number;
  active?: boolean;
}) {
  const q = usePrMeta(owner, name, number, active);

  if (q.isPending) return <div className="pr-meta-panel muted">Loading PR details…</div>;
  if (q.isError || !q.data)
    return (
      <div className="pr-meta-panel muted pr-meta-error">
        Could not load PR details: {String(q.error)}
      </div>
    );

  const meta = q.data;
  const pr = { owner, name, number };

  return (
    <div className="pr-meta-panel">
      <div className="pr-meta-grid">
        <span className="pr-meta-key">Author</span>
        <div className="pr-meta-val">
          <Person login={meta.author?.login ?? "unknown"} avatarUrl={meta.author?.avatarUrl ?? null} />
        </div>
        <span className="pr-meta-key">Reviewers</span>
        <ReviewersField pr={pr} meta={meta} />
        <span className="pr-meta-key">Labels</span>
        <LabelsField pr={pr} meta={meta} />
      </div>

      <hr className="pr-meta-divider" />

      <div className="pr-meta-row">
        <Mergeability mergeable={meta.mergeable} />
        <span className="pr-meta-counts">
          <span className="add">+{meta.additions}</span>{" "}
          <span className="del">−{meta.deletions}</span> ·{" "}
          {meta.changedFiles} file{meta.changedFiles === 1 ? "" : "s"}
        </span>
      </div>


      {meta.checks.length > 0 && <ChecksList checks={meta.checks} />}

      {meta.body.trim() && <Description body={meta.body} />}
    </div>
  );
}

const MERGEABLE: Record<string, { label: string; kind: string }> = {
  MERGEABLE: { label: "Mergeable", kind: "ok" },
  CONFLICTING: { label: "Conflicts", kind: "warn" },
};

function Mergeability({ mergeable }: { mergeable: string | null }) {
  const m = (mergeable && MERGEABLE[mergeable]) || { label: "Checking…", kind: "neutral" };
  return <span className={`pr-meta-mergeable mergeable-${m.kind}`}>{m.label}</span>;
}

const REVIEW_KIND: Record<string, string> = {
  APPROVED: "ok",
  CHANGES_REQUESTED: "bad",
  DISMISSED: "neutral",
  COMMENTED: "neutral",
  PENDING: "pending",
};

function LabelChip({ label, onRemove }: { label: PrLabel; onRemove?: () => void }) {
  const bg = `#${label.color}`;
  const fg = readableText(label.color);
  return (
    <span className="pr-label-chip" style={{ background: bg, color: fg }}>
      {label.name}
      {onRemove && (
        <button className="pr-chip-x" style={{ color: fg }} title={`Remove ${label.name}`} onClick={onRemove}>
          ×
        </button>
      )}
    </span>
  );
}

type PrRef = { owner: string; name: string; number: number };

function Person({ login, avatarUrl, children }: { login: string; avatarUrl: string | null; children?: ReactNode }) {
  return (
    <span className="pr-reviewer">
      {avatarUrl ? (
        <img className="pr-reviewer-avatar" src={avatarUrl} alt={login} />
      ) : (
        <span className="pr-reviewer-avatar pr-reviewer-avatar-empty" />
      )}
      <span className="pr-reviewer-login">{login}</span>
      {children}
    </span>
  );
}

/** A PR edit applied to the cached meta at once, rolled back if GitHub rejects it. */
function useMetaEdit<V>(
  pr: PrRef,
  run: (v: V) => Promise<void>,
  patch: (meta: PrMeta, v: V) => PrMeta,
  failure: string,
) {
  const queryClient = useQueryClient();
  const key = ["pr-meta", pr.owner, pr.name, pr.number];
  return useMutation({
    mutationFn: run,
    onMutate: async (v: V) => {
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<PrMeta>(key);
      if (previous) queryClient.setQueryData<PrMeta>(key, patch(previous, v));
      return { previous };
    },
    onError: (e, _v, context) => {
      if (context?.previous) queryClient.setQueryData<PrMeta>(key, context.previous);
      toast.error(`${failure}:\n${String(e)}`);
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: key }),
  });
}

const REVIEW_MARK: Record<string, string> = { ok: "✓", bad: "✕", neutral: "💬", pending: "⏳" };

function ReviewersField({ pr, meta }: { pr: PrRef; meta: PrMeta }) {
  const [picking, setPicking] = useState(false);
  const editable = meta.state === "OPEN";
  const author = meta.author?.login ?? null;
  const edit = useMetaEdit(
    pr,
    (v: { login: string; team: boolean; on: boolean; avatarUrl?: string | null }) =>
      api.setPrReviewer(pr.owner, pr.name, pr.number, v.login, v.team, v.on),
    (m, v) => ({
      ...m,
      reviewRequests: v.on
        ? [
            ...m.reviewRequests.filter((r) => r.login !== v.login),
            { login: v.login, avatarUrl: v.avatarUrl ?? avatarOf(m, v.login), isTeam: v.team },
          ]
        : m.reviewRequests.filter((r) => !(r.login === v.login && r.isTeam === v.team)),
    }),
    "Could not update reviewers",
  );
  const [typing, setTyping] = useState(false);
  const [debounced, setDebounced] = useState("");
  const settle = useDebouncedCallback((q: string) => {
    setDebounced(q.trim());
    setTyping(false);
  }, 250);
  const users = useQuery({
    queryKey: ["repo-assignable-users", pr.owner, pr.name, debounced],
    queryFn: () => api.repoAssignableUsers(pr.owner, pr.name, debounced),
    enabled: picking,
    staleTime: 10 * 60_000,
    placeholderData: keepPreviousData,
  });

  // A re-requested reviewer is listed once, as pending.
  const requested = new Set(meta.reviewRequests.filter((r) => !r.isTeam).map((r) => r.login));
  const submitted = meta.reviews.filter((r) => r.author?.login && !requested.has(r.author.login));

  return (
    <div className="pr-meta-val">
      {submitted.map((r, i) => {
        const login = r.author!.login!;
        const kind = REVIEW_KIND[r.state] ?? "neutral";
        return (
          <span key={`${login}-${i}`} className={`reviewer-${kind}`} title={`${login} · ${humanState(r.state)}`}>
            <Person login={login} avatarUrl={r.author?.avatarUrl ?? null}>
              <span className="pr-review-mark">{REVIEW_MARK[kind]}</span>
              {editable && login !== author && (
                <button
                  className="pr-chip-x pr-chip-hover"
                  title={`Re-request review from ${login}`}
                  onClick={() => edit.mutate({ login, team: false, on: true })}
                >
                  ↻
                </button>
              )}
            </Person>
          </span>
        );
      })}
      {meta.reviewRequests.map((r) => (
        <RequestChip
          key={`${r.isTeam ? "team" : "user"}-${r.login}`}
          request={r}
          onRemove={editable ? () => edit.mutate({ login: r.login, team: r.isTeam, on: false }) : undefined}
        />
      ))}
      {submitted.length === 0 && meta.reviewRequests.length === 0 && <span className="faint">None</span>}
      {editable && (
        <PickerButton
          title="Request reviewers"
          open={picking}
          setOpen={setPicking}
          placeholder="Search users…"
          onSearch={(q) => {
            setTyping(true);
            settle(q);
          }}
          loading={users.isPending || typing || users.isPlaceholderData}
          error={users.error}
          items={(users.data ?? [])
            .filter((u): u is PrActor & { login: string } => !!u.login && u.login !== author)
            .map((u) => ({
              key: u.login,
              label: u.login,
              icon: u.avatarUrl ? (
                <img className="pr-reviewer-avatar" src={u.avatarUrl} alt="" />
              ) : (
                <span className="pr-reviewer-avatar pr-reviewer-avatar-empty" />
              ),
            }))}
          selected={requested}
          onToggle={(login, on) =>
            edit.mutate({ login, team: false, on, avatarUrl: users.data?.find((u) => u.login === login)?.avatarUrl })
          }
        />
      )}
    </div>
  );
}

function RequestChip({ request, onRemove }: { request: PrReviewRequest; onRemove?: () => void }) {
  const login = request.isTeam ? `@${request.login}` : request.login;
  return (
    <span className="reviewer-pending" title={`${login} · review requested`}>
      <Person login={login} avatarUrl={request.avatarUrl}>
        <span className="pr-review-mark">{REVIEW_MARK.pending}</span>
        {onRemove && (
          <button className="pr-chip-x" title={`Remove review request for ${login}`} onClick={onRemove}>
            ×
          </button>
        )}
      </Person>
    </span>
  );
}

function LabelsField({ pr, meta }: { pr: PrRef; meta: PrMeta }) {
  const [picking, setPicking] = useState(false);
  const repoLabels = useQuery({
    queryKey: ["repo-labels", pr.owner, pr.name],
    queryFn: () => api.repoLabels(pr.owner, pr.name),
    enabled: picking,
    staleTime: 10 * 60_000,
  });
  const edit = useMetaEdit(
    pr,
    (v: { label: PrLabel; on: boolean }) => api.setPrLabel(pr.owner, pr.name, pr.number, v.label.name, v.on),
    (m, v) => ({
      ...m,
      labels: v.on ? [...m.labels.filter((l) => l.name !== v.label.name), v.label] : m.labels.filter((l) => l.name !== v.label.name),
    }),
    "Could not update labels",
  );
  const byName = new Map((repoLabels.data ?? []).map((l) => [l.name, l]));

  return (
    <div className="pr-meta-val">
      {meta.labels.map((l) => (
        <LabelChip key={l.name} label={l} onRemove={() => edit.mutate({ label: l, on: false })} />
      ))}
      {meta.labels.length === 0 && <span className="faint">None</span>}
      <PickerButton
        title="Edit labels"
        open={picking}
        setOpen={setPicking}
        placeholder="Filter labels…"
        loading={repoLabels.isPending}
        error={repoLabels.error}
        items={(repoLabels.data ?? []).map((l) => ({
          key: l.name,
          label: l.name,
          icon: <span className="pr-label-dot" style={{ background: `#${l.color}` }} />,
        }))}
        selected={new Set(meta.labels.map((l) => l.name))}
        onToggle={(key, on) => {
          const label = byName.get(key);
          if (label) edit.mutate({ label, on });
        }}
      />
    </div>
  );
}

type PickerItem = { key: string; label: string; icon?: ReactNode };

/** `+` button opening a filterable checkbox list. Each toggle applies at once;
 *  the list stays open until an outside click or Esc. */
function PickerButton({
  title,
  open,
  setOpen,
  placeholder,
  loading,
  error,
  items,
  selected,
  onToggle,
  onSearch,
}: {
  title: string;
  open: boolean;
  setOpen: (open: boolean) => void;
  placeholder: string;
  loading: boolean;
  error: unknown;
  items: PickerItem[];
  selected: Set<string>;
  onToggle: (key: string, on: boolean) => void;
  /** Search server-side: the text goes to the caller, and `items` are already its matches. */
  onSearch?: (query: string) => void;
}) {
  const rootRef = useRef<HTMLSpanElement>(null);
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open, setOpen]);

  const q = query.trim().toLowerCase();
  // Selected first, so what's on the PR stays visible at the top.
  const options = items
    .filter((i) => onSearch || !q || i.label.toLowerCase().includes(q))
    .sort((a, b) => Number(selected.has(b.key)) - Number(selected.has(a.key)));
  const active = Math.min(highlight, Math.max(options.length - 1, 0));

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setHighlight(Math.min(active + 1, Math.max(options.length - 1, 0)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setHighlight(Math.max(active - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const opt = options[active];
      if (opt) onToggle(opt.key, !selected.has(opt.key));
    } else if (e.key === "Escape") {
      e.stopPropagation();
      setOpen(false);
    }
  };

  return (
    <span className="pr-picker" ref={rootRef}>
      <button className="pr-picker-add" title={title} aria-expanded={open} onClick={() => setOpen(!open)}>
        +
      </button>
      {open && (
        <div className="pr-picker-pop card" role="listbox" aria-multiselectable>
          <input
            className="input pr-picker-input"
            autoFocus
            placeholder={placeholder}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setHighlight(0);
              onSearch?.(e.target.value);
            }}
            onKeyDown={onKeyDown}
          />
          <div className="pr-picker-list scroll">
            {loading && options.length === 0 ? (
              <div className="pr-picker-empty faint">Loading…</div>
            ) : error ? (
              <div className="pr-picker-empty pr-meta-error">{String(error)}</div>
            ) : options.length === 0 ? (
              <div className="pr-picker-empty faint">No matches</div>
            ) : (
              options.map((o, i) => {
                const on = selected.has(o.key);
                return (
                  <button
                    key={o.key}
                    role="option"
                    aria-selected={on}
                    className={`tag-filter-option${i === active ? " on" : ""}`}
                    onMouseEnter={() => setHighlight(i)}
                    onClick={() => onToggle(o.key, !on)}
                  >
                    <span className="pr-picker-check">{on ? "✓" : ""}</span>
                    {o.icon}
                    <span className="lbl">{o.label}</span>
                  </button>
                );
              })
            )}
          </div>
        </div>
      )}
    </span>
  );
}

function avatarOf(meta: PrMeta, login: string): string | null {
  return meta.reviews.find((r) => r.author?.login === login)?.author?.avatarUrl ?? null;
}

function humanState(state: string): string {
  return state.toLowerCase().replace(/_/g, " ");
}

function ChecksList({ checks }: { checks: PrCheck[] }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="pr-meta-checks">
      <button className="pr-meta-toggle" onClick={() => setOpen((v) => !v)}>
        {open ? "▾" : "▸"} {checks.length} check{checks.length === 1 ? "" : "s"}
      </button>
      {open && (
        <ul className="pr-check-list">
          {checks.map((c, i) => (
            <li key={`${c.name}-${i}`} className={`pr-check check-${checkKind(c.state)}`}>
              <span className="pr-check-mark">{checkMark(c.state)}</span>
              {c.url ? (
                <a
                  href={c.url}
                  onClick={(e) => {
                    e.preventDefault();
                    if (c.url) api.openUrl(c.url);
                  }}
                >
                  {c.name}
                </a>
              ) : (
                <span>{c.name}</span>
              )}
              {c.state && <span className="pr-check-state">{c.state.toLowerCase()}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Description({ body }: { body: string }) {
  return (
    <div className="pr-meta-description">
      <div className="pr-meta-desc-body scroll">
        <Markdown source={body} />
      </div>
    </div>
  );
}

const PASSED = new Set(["success", "completed", "neutral", "skipped"]);
const FAILED = new Set(["failure", "error", "cancelled", "timed_out", "action_required"]);

function checkKind(state: string | null): string {
  if (!state) return "pending";
  const s = state.toLowerCase();
  if (PASSED.has(s)) return "ok";
  if (FAILED.has(s)) return "bad";
  return "pending";
}

function checkMark(state: string | null): string {
  const k = checkKind(state);
  return k === "ok" ? "✓" : k === "bad" ? "✕" : "•";
}

/** Pick black/white text for a hex background via relative luminance. */
function readableText(hex: string): string {
  const n = parseInt(hex, 16);
  if (Number.isNaN(n) || hex.length !== 6) return "#000";
  const r = (n >> 16) & 0xff;
  const g = (n >> 8) & 0xff;
  const b = n & 0xff;
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return luminance > 0.6 ? "#000" : "#fff";
}
