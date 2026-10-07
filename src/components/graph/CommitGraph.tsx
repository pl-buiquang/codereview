import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import type { GraphCommit, RefInfo, WorktreeInfo } from "../../lib/types";
import { PALETTE_SIZE, type Edge, type GraphRow } from "../../lib/graphLayout";
import { identicon } from "../../lib/avatar";
import "./CommitGraph.css";

export const WIP_SELECTION = "WIP";
export const ROW_HEIGHT = 34;
export const LANE_WIDTH = 26;
const FALLBACK_VIEWPORT_HEIGHT = 600;
const OVERSCAN_ROWS = 8;
const END_REACHED_THRESHOLD_ROWS = 20;
const AVATAR_RADIUS = 11;
const MERGE_RADIUS = 5;
const WIP_RADIUS = 9;
const MAX_VISIBLE_PILLS = 2;

export interface CommitGraphHandle {
  /** Scrolls the row for `sha` into view. Returns false when that commit isn't loaded. */
  scrollToSha: (sha: string) => boolean;
}

export interface CommitGraphProps {
  commits: GraphCommit[];
  /** Output of `layoutGraph(commits)`; `rows[i]` describes `commits[i]`. */
  rows: GraphRow[];
  maxLanes: number;
  /** Lowercased author email → avatar image URL; authors without one get a generated identicon. */
  avatars?: ReadonlyMap<string, string>;
  refs: RefInfo[];
  worktrees?: WorktreeInfo[];
  /** HEAD of the context checkout; gets a HEAD pill and anchors the WIP row. */
  headSha: string | null;
  /** When non-null, a dashed `// WIP +N` row renders directly above HEAD. */
  wip?: { count: number } | null;
  /** A commit sha, `WIP_SELECTION`, or null. */
  selectedSha: string | null;
  onSelect: (sha: string) => void;
  /** Fired once per `commits.length` when the viewport nears the end of the list. */
  onEndReached?: () => void;
  onRefContextMenu?: (ref: RefInfo, event: ReactMouseEvent) => void;
}

type Pill =
  | { kind: "head"; label: string }
  | { kind: "worktree"; label: string; title: string }
  | { kind: RefInfo["kind"]; label: string; ref: RefInfo };

type Item = { type: "wip" } | { type: "commit"; index: number };

function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

const colorClass = (color: number) => `cg-c${((color % PALETTE_SIZE) + PALETTE_SIZE) % PALETTE_SIZE}`;
const laneX = (lane: number) => lane * LANE_WIDTH + LANE_WIDTH / 2;

function edgePath(e: Edge, y0: number, y1: number): string {
  const x0 = laneX(e.fromLane);
  const x1 = laneX(e.toLane);
  if (x0 === x1) return `M${x0} ${y0} L${x1} ${y1}`;
  const ym = (y0 + y1) / 2;
  return `M${x0} ${y0} C${x0} ${ym} ${x1} ${ym} ${x1} ${y1}`;
}

export const CommitGraph = forwardRef<CommitGraphHandle, CommitGraphProps>(function CommitGraph(
  {
    commits,
    rows,
    maxLanes,
    avatars,
    refs,
    worktrees = [],
    headSha,
    wip = null,
    selectedSha,
    onSelect,
    onEndReached,
    onRefContextMenu,
  },
  ref,
) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(FALLBACK_VIEWPORT_HEIGHT);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () => setViewportHeight(el.clientHeight > 0 ? el.clientHeight : FALLBACK_VIEWPORT_HEIGHT);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const headIndex = headSha ? commits.findIndex((c) => c.sha === headSha) : -1;
  const showWip = !!wip && wip.count > 0;

  const items = useMemo<Item[]>(() => {
    const out: Item[] = commits.map((_, index) => ({ type: "commit", index }));
    if (showWip) out.splice(Math.max(headIndex, 0), 0, { type: "wip" });
    return out;
  }, [commits, showWip, headIndex]);

  const pillsBySha = useMemo(() => {
    const map = new Map<string, Pill[]>();
    const push = (sha: string, pill: Pill) => {
      const list = map.get(sha);
      if (list) list.push(pill);
      else map.set(sha, [pill]);
    };
    if (headSha) push(headSha, { kind: "head", label: "HEAD" });
    const order: Record<RefInfo["kind"], number> = { local: 0, remote: 1, tag: 2 };
    for (const r of [...refs].sort((a, b) => order[a.kind] - order[b.kind] || Number(b.is_head) - Number(a.is_head))) {
      push(r.sha, { kind: r.kind, label: r.name, ref: r });
    }
    for (const wt of worktrees) {
      if (wt.is_main || !wt.head_sha) continue;
      push(wt.head_sha, { kind: "worktree", label: basename(wt.display_path || wt.path), title: wt.display_path || wt.path });
    }
    return map;
  }, [refs, worktrees, headSha]);

  const total = items.length;
  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN_ROWS);
  const last = Math.min(total, Math.ceil((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN_ROWS);

  const firedForLength = useRef<number | null>(null);
  useEffect(() => {
    if (!onEndReached || commits.length === 0) return;
    if (last < total - END_REACHED_THRESHOLD_ROWS) return;
    if (firedForLength.current === commits.length) return;
    firedForLength.current = commits.length;
    onEndReached();
  }, [last, total, commits.length, onEndReached]);

  const scrollToIndex = useCallback(
    (index: number) => {
      const el = scrollRef.current;
      if (!el) return;
      const rowTop = index * ROW_HEIGHT;
      let next = el.scrollTop;
      if (rowTop < next || rowTop + ROW_HEIGHT > next + viewportHeight) {
        next = Math.max(0, rowTop - Math.floor((viewportHeight - ROW_HEIGHT) / 2));
      }
      el.scrollTop = next;
      setScrollTop(next);
    },
    [viewportHeight],
  );

  useImperativeHandle(
    ref,
    () => ({
      scrollToSha: (sha: string) => {
        const idx = items.findIndex((it) =>
          sha === WIP_SELECTION ? it.type === "wip" : it.type === "commit" && commits[it.index].sha === sha,
        );
        if (idx === -1) return false;
        scrollToIndex(idx);
        return true;
      },
    }),
    [items, commits, scrollToIndex],
  );

  const svgWidth = Math.max(maxLanes, 1) * LANE_WIDTH;
  const headRow = headIndex >= 0 ? rows[headIndex] : undefined;
  const wipLane = headRow?.lane ?? 0;

  const renderPills = (pills: Pill[] | undefined) => {
    if (!pills || pills.length === 0) return null;
    const shown = pills.slice(0, MAX_VISIBLE_PILLS);
    const hidden = pills.slice(MAX_VISIBLE_PILLS);
    return (
      <>
        {shown.map((p, i) => (
          <span
            key={`${p.kind}:${p.label}:${i}`}
            className={`cg-pill cg-pill-${p.kind}${"ref" in p && p.ref.is_head ? " cg-pill-current" : ""}`}
            title={"title" in p ? p.title : p.label}
            data-ref-kind={p.kind}
            onContextMenu={
              "ref" in p && onRefContextMenu
                ? (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    onRefContextMenu(p.ref, e);
                  }
                : undefined
            }
          >
            {p.label}
          </span>
        ))}
        {hidden.length > 0 && (
          <span className="cg-pill cg-pill-more" title={hidden.map((p) => p.label).join(", ")}>
            +{hidden.length}
          </span>
        )}
      </>
    );
  };

  const mid = ROW_HEIGHT / 2;

  const renderWipRow = (top: number) => {
    const passThrough = headRow?.edgesUp ?? [];
    const selected = selectedSha === WIP_SELECTION;
    return (
      <div
        key="__wip"
        role="row"
        aria-selected={selected}
        data-testid="cg-wip-row"
        className={`cg-row cg-row-wip${selected ? " is-selected" : ""}`}
        style={{ top, height: ROW_HEIGHT }}
        onClick={() => onSelect(WIP_SELECTION)}
      >
        <div className="cg-refs" />
        <svg className="cg-lanes" width={svgWidth} height={ROW_HEIGHT} aria-hidden>
          {passThrough.map((e, i) => (
            <path key={`p${i}`} className={`cg-edge ${colorClass(e.color)}`} d={edgePath({ ...e, toLane: e.fromLane }, 0, ROW_HEIGHT)} />
          ))}
          <path className="cg-edge cg-edge-wip" d={`M${laneX(wipLane)} ${mid} L${laneX(wipLane)} ${ROW_HEIGHT}`} />
          <circle className="cg-dot cg-dot-wip" cx={laneX(wipLane)} cy={mid} r={WIP_RADIUS} />
        </svg>
        <div className="cg-message">
          <span className="cg-subject cg-subject-wip">// WIP +{wip?.count ?? 0}</span>
        </div>
      </div>
    );
  };

  const renderCommitRow = (index: number, top: number) => {
    const commit = commits[index];
    const row = rows[index];
    const selected = selectedSha === commit.sha;
    const isHead = commit.sha === headSha;
    const isMerge = commit.parents.length > 1;
    const lane = row?.lane ?? 0;
    return (
      <div
        key={commit.sha}
        role="row"
        aria-selected={selected}
        data-sha={commit.sha}
        className={`cg-row${selected ? " is-selected" : ""}${isHead ? " is-head" : ""}`}
        style={{ top, height: ROW_HEIGHT }}
        onClick={() => onSelect(commit.sha)}
      >
        <div className="cg-refs">{renderPills(pillsBySha.get(commit.sha))}</div>
        <svg className="cg-lanes" width={svgWidth} height={ROW_HEIGHT} aria-hidden>
          {row?.edgesUp.map((e, i) => (
            <path key={`u${i}`} className={`cg-edge ${colorClass(e.color)}`} d={edgePath(e, 0, mid)} />
          ))}
          {row?.edgesDown.map((e, i) => (
            <path key={`d${i}`} className={`cg-edge ${colorClass(e.color)}`} d={edgePath(e, mid, ROW_HEIGHT)} />
          ))}
          {isHead && showWip && (
            <path className="cg-edge cg-edge-wip" d={`M${laneX(lane)} 0 L${laneX(lane)} ${mid}`} />
          )}
          {isMerge ? (
            <circle
              className={`cg-dot ${colorClass(row?.color ?? 0)} cg-dot-merge${isHead ? " cg-dot-head" : ""}`}
              cx={laneX(lane)}
              cy={mid}
              r={MERGE_RADIUS}
            />
          ) : (
            <>
              <clipPath id={`cg-av-${commit.sha}`}>
                <circle cx={laneX(lane)} cy={mid} r={AVATAR_RADIUS} />
              </clipPath>
              <circle
                className={`cg-dot ${colorClass(row?.color ?? 0)}${isHead ? " cg-dot-head" : ""}`}
                cx={laneX(lane)}
                cy={mid}
                r={AVATAR_RADIUS + 2}
              />
              <image
                className="cg-avatar"
                href={avatars?.get(commit.author_email.toLowerCase()) ?? identicon(commit.author_name || commit.author_email)}
                x={laneX(lane) - AVATAR_RADIUS}
                y={mid - AVATAR_RADIUS}
                width={AVATAR_RADIUS * 2}
                height={AVATAR_RADIUS * 2}
                preserveAspectRatio="xMidYMid slice"
                clipPath={`url(#cg-av-${commit.sha})`}
              >
                <title>{`${commit.author_name} <${commit.author_email}>`}</title>
              </image>
            </>
          )}
        </svg>
        <div className="cg-message">
          <span className="cg-subject">{commit.subject}</span>
          {commit.body_preview && <span className="cg-body">{commit.body_preview}</span>}
        </div>
      </div>
    );
  };

  const visible = [];
  for (let i = first; i < last; i++) {
    const it = items[i];
    const top = i * ROW_HEIGHT;
    visible.push(it.type === "wip" ? renderWipRow(top) : renderCommitRow(it.index, top));
  }

  return (
    <div
      ref={scrollRef}
      className="cg-scroll"
      role="grid"
      aria-rowcount={total}
      data-testid="commit-graph"
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
    >
      <div className="cg-canvas" style={{ height: total * ROW_HEIGHT }}>
        {visible}
      </div>
    </div>
  );
});
