import type {
  FocusEntry,
  FocusLevel,
  FocusOverride,
  FocusTier,
  FocusView,
} from "./types";

export const FOCUS_LEVELS: { value: FocusLevel; label: string; title: string }[] = [
  { value: "off", label: "Off", title: "Show the full diff" },
  { value: "thorough", label: "Thorough", title: "Everything except trivial hunks" },
  { value: "focus", label: "Focus", title: "Critical and review hunks" },
  { value: "critical", label: "Critical only", title: "Only critical hunks" },
];

const TIER_RANK: Record<FocusTier, number> = { trivial: 0, skim: 1, review: 2, critical: 3 };
const LEVEL_MIN: Record<FocusLevel, number> = { off: 0, thorough: 1, focus: 2, critical: 3 };

export const MANUAL_REASON = "hidden manually";

export function tierVisible(tier: FocusTier, level: FocusLevel): boolean {
  return TIER_RANK[tier] >= LEVEL_MIN[level];
}

/** A focus overlays the diff only once a plan has been written and the level isn't Off. */
export function focusActive(view: FocusView | null | undefined): view is FocusView {
  return !!view && view.focus.planned_at != null && view.focus.level !== "off";
}

export interface HunkSpan {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
}

export interface Anchor {
  path: string;
  side: string;
  line: number;
}

export interface HunkFocus {
  entry: FocusEntry | null;
  hidden: boolean;
  /** Has a comment or GitHub thread, so it is never hidden. */
  pinned: boolean;
  reason: string | null;
}

export interface FileFocus {
  index: number;
  path: string;
  /** Aligned with the file's parsed (unexpanded) hunks. */
  hunks: HunkFocus[];
  /** Stands for the whole file when it has no textual hunks. */
  whole: HunkFocus | null;
  fullyHidden: boolean;
  override: FocusOverride | null;
}

export function hunkHidden(
  entry: FocusEntry,
  fileOverride: FocusOverride | null,
  level: FocusLevel,
  pinned: boolean,
  revealed: boolean,
): boolean {
  if (level === "off" || pinned || revealed) return false;
  const o = entry.override ?? fileOverride;
  if (o) return o === "hide";
  return !tierVisible(entry.tier, level);
}

function hiddenReason(entry: FocusEntry, fileOverride: FocusOverride | null): string {
  const o = entry.override ?? fileOverride;
  if (o === "hide") return MANUAL_REASON;
  return entry.reason ?? `${entry.tier} tier`;
}

function anchorInSpan(span: HunkSpan, a: Anchor): boolean {
  if (a.side === "LEFT") return a.line >= span.oldStart && a.line < span.oldStart + span.oldLines;
  return a.line >= span.newStart && a.line < span.newStart + span.newLines;
}

const entryKey = (path: string, oldStart: number, newStart: number) =>
  `${path}\u0000${oldStart}\u0000${newStart}`;

/**
 * Resolve every parsed hunk to its focus entry and visibility. Entries are
 * matched by (file, oldStart, newStart); a hunk without one stays visible.
 * Returns null when no focus is active.
 */
export function buildFocusModel(
  files: { path: string; hunks: HunkSpan[] }[],
  view: FocusView | null | undefined,
  anchors: Anchor[],
  revealed: ReadonlySet<string>,
): FileFocus[] | null {
  if (!focusActive(view)) return null;
  const level = view.focus.level;
  const byKey = new Map<string, FocusEntry>();
  for (const e of view.hunks) byKey.set(entryKey(e.file_path, e.old_start, e.new_start), e);
  const fileOverrides = new Map(view.file_overrides.map((o) => [o.file_path, o.override]));
  const anchorsByPath = new Map<string, Anchor[]>();
  for (const a of anchors) {
    const arr = anchorsByPath.get(a.path) ?? [];
    arr.push(a);
    anchorsByPath.set(a.path, arr);
  }

  return files.map((f, index) => {
    const fo = fileOverrides.get(f.path) ?? null;
    const fileAnchors = anchorsByPath.get(f.path) ?? [];
    const resolve = (entry: FocusEntry | null, pinned: boolean): HunkFocus => {
      if (!entry) return { entry, pinned, hidden: false, reason: null };
      const hidden = hunkHidden(entry, fo, level, pinned, revealed.has(entry.hunk_id));
      return { entry, pinned, hidden, reason: hidden ? hiddenReason(entry, fo) : null };
    };
    if (f.hunks.length === 0) {
      const whole = resolve(byKey.get(entryKey(f.path, 0, 0)) ?? null, fileAnchors.length > 0);
      return { index, path: f.path, hunks: [], whole, fullyHidden: whole.hidden, override: fo };
    }
    const hunks = f.hunks.map((h) =>
      resolve(
        byKey.get(entryKey(f.path, h.oldStart, h.newStart)) ?? null,
        fileAnchors.some((a) => anchorInSpan(h, a)),
      ),
    );
    return {
      index,
      path: f.path,
      hunks,
      whole: null,
      fullyHidden: hunks.every((h) => h.hidden),
      override: fo,
    };
  });
}

/**
 * Indices of the original hunks that a rendered hunk covers. Context expansion
 * merges neighbouring hunks, so one rendered hunk may stand for several.
 */
export function originalsIn(rendered: HunkSpan, originals: HunkSpan[]): number[] {
  const out: number[] = [];
  originals.forEach((o, i) => {
    if (
      o.oldStart >= rendered.oldStart &&
      o.oldStart + o.oldLines <= rendered.oldStart + rendered.oldLines &&
      o.newStart >= rendered.newStart &&
      o.newStart + o.newLines <= rendered.newStart + rendered.newLines
    )
      out.push(i);
  });
  return out;
}

export interface DigestItem {
  fileIndex: number;
  path: string;
  hunk: HunkFocus;
}

export interface DigestGroup {
  reason: string;
  items: DigestItem[];
}

/** Hidden hunks/files grouped by reason, largest group first. */
export function digestGroups(model: FileFocus[]): DigestGroup[] {
  const groups = new Map<string, DigestItem[]>();
  for (const f of model) {
    const all = f.whole ? [f.whole] : f.hunks;
    for (const h of all) {
      if (!h.hidden || !h.reason) continue;
      const arr = groups.get(h.reason) ?? [];
      arr.push({ fileIndex: f.index, path: f.path, hunk: h });
      groups.set(h.reason, arr);
    }
  }
  return [...groups.entries()]
    .map(([reason, items]) => ({ reason, items }))
    .sort((a, b) => b.items.length - a.items.length || a.reason.localeCompare(b.reason));
}

export function hiddenCount(model: FileFocus[]): number {
  return digestGroups(model).reduce((n, g) => n + g.items.length, 0);
}

function fileList(items: DigestItem[]): string {
  const counts = new Map<string, number>();
  for (const it of items) counts.set(it.path, (counts.get(it.path) ?? 0) + 1);
  return [...counts.entries()]
    .map(([p, n]) => (n > 1 ? `\`${p}\` (${n})` : `\`${p}\``))
    .join(", ");
}

export function digestMarkdown(view: FocusView, model: FileFocus[]): string {
  const lines = ["### Focus digest", ""];
  if (view.focus.summary) lines.push(view.focus.summary.trim(), "");
  const groups = digestGroups(model);
  if (groups.length === 0) {
    lines.push(`Nothing hidden at level **${view.focus.level}**.`);
  } else {
    lines.push(`Hidden at level **${view.focus.level}**:`);
    for (const g of groups) lines.push(`- **${g.reason}** (${g.items.length}): ${fileList(g.items)}`);
  }
  return lines.join("\n");
}
