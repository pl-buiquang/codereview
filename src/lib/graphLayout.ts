import type { GraphCommit } from "./types";

// edgesUp: top half of the row, fromLane at the row's top edge -> toLane at the row's center.
// edgesDown: bottom half of the row, fromLane at the row's center -> toLane at the row's bottom edge.
// Pass-through lanes appear in both halves as l -> l, so each row can be drawn independently and
// row i's edgesDown toLanes always equal row i+1's edgesUp fromLanes.
export interface GraphRow {
  sha: string;
  lane: number;
  color: number;
  edgesUp: Edge[];
  edgesDown: Edge[];
}

export interface Edge {
  fromLane: number;
  toLane: number;
  color: number;
}

export const PALETTE_SIZE = 8;

export function layoutGraph(commits: GraphCommit[]): { rows: GraphRow[]; maxLanes: number } {
  const lanes: (string | null)[] = [];
  const colors: number[] = [];
  let nextColor = 0;
  let maxLanes = 0;
  const rows: GraphRow[] = [];

  const takeColor = () => {
    const c = nextColor % PALETTE_SIZE;
    nextColor += 1;
    return c;
  };
  const freeLane = () => {
    const idx = lanes.indexOf(null);
    if (idx !== -1) return idx;
    lanes.push(null);
    colors.push(0);
    return lanes.length - 1;
  };

  for (const commit of commits) {
    const { sha } = commit;

    let lane = lanes.indexOf(sha);
    let color: number;
    if (lane === -1) {
      lane = freeLane();
      color = takeColor();
      colors[lane] = color;
    } else {
      color = colors[lane];
    }

    const edgesUp: Edge[] = [];
    for (let l = 0; l < lanes.length; l++) {
      const expected = lanes[l];
      if (expected === null) continue;
      if (expected === sha) {
        edgesUp.push({ fromLane: l, toLane: lane, color: colors[l] });
        lanes[l] = null;
      } else {
        edgesUp.push({ fromLane: l, toLane: l, color: colors[l] });
      }
    }

    const [firstParent, ...extraParents] = commit.parents;
    const branchEdges: Edge[] = [];
    if (firstParent !== undefined) {
      lanes[lane] = firstParent;
      colors[lane] = color;
      branchEdges.push({ fromLane: lane, toLane: lane, color });
    }
    for (const parent of extraParents) {
      let target = lanes.indexOf(parent);
      if (target === -1) {
        target = freeLane();
        lanes[target] = parent;
        colors[target] = takeColor();
      }
      if (!branchEdges.some((e) => e.toLane === target)) {
        branchEdges.push({ fromLane: lane, toLane: target, color: colors[target] });
      }
    }

    const edgesDown: Edge[] = [];
    for (let l = 0; l < lanes.length; l++) {
      if (lanes[l] === null) continue;
      const fromCommit = branchEdges.find((e) => e.toLane === l);
      edgesDown.push(fromCommit ?? { fromLane: l, toLane: l, color: colors[l] });
    }

    while (lanes.length > 0 && lanes[lanes.length - 1] === null) {
      lanes.pop();
      colors.pop();
    }

    for (const e of [...edgesUp, ...edgesDown]) {
      maxLanes = Math.max(maxLanes, e.fromLane + 1, e.toLane + 1);
    }
    maxLanes = Math.max(maxLanes, lane + 1);

    rows.push({ sha, lane, color, edgesUp, edgesDown });
  }

  return { rows, maxLanes };
}
