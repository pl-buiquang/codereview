import { describe, expect, it } from "vitest";
import { layoutGraph, PALETTE_SIZE, type GraphRow } from "./graphLayout";
import type { GraphCommit } from "./types";

function c(sha: string, ...parents: string[]): GraphCommit {
  return {
    sha,
    parents,
    subject: sha,
    body_preview: "",
    author_name: "a",
    author_email: "dev@example.com",
    author_time: 0,
    committer_time: 0,
  };
}

function row(rows: GraphRow[], sha: string): GraphRow {
  const r = rows.find((x) => x.sha === sha);
  if (!r) throw new Error(`no row ${sha}`);
  return r;
}

function expectRowsConnect(rows: GraphRow[]) {
  for (let i = 0; i + 1 < rows.length; i++) {
    const down = rows[i].edgesDown.map((e) => e.toLane).sort();
    const up = rows[i + 1].edgesUp.map((e) => e.fromLane).sort();
    expect(up).toEqual(down);
  }
}

describe("layoutGraph", () => {
  it("returns empty layout for no commits", () => {
    expect(layoutGraph([])).toEqual({ rows: [], maxLanes: 0 });
  });

  it("keeps a linear chain on lane 0 with one color", () => {
    const { rows, maxLanes } = layoutGraph([c("c", "b"), c("b", "a"), c("a")]);
    expect(maxLanes).toBe(1);
    expect(rows.map((r) => r.lane)).toEqual([0, 0, 0]);
    expect(new Set(rows.map((r) => r.color)).size).toBe(1);
    expect(rows[0].edgesUp).toEqual([]);
    expect(rows[0].edgesDown).toEqual([{ fromLane: 0, toLane: 0, color: rows[0].color }]);
    expect(rows[2].edgesDown).toEqual([]);
    expectRowsConnect(rows);
  });

  it("lays out a branch and merge", () => {
    // m merges main (b) and feature (f); both fork from a.
    const { rows, maxLanes } = layoutGraph([
      c("m", "b", "f"),
      c("f", "a"),
      c("b", "a"),
      c("a"),
    ]);
    expect(maxLanes).toBe(2);
    const m = row(rows, "m");
    expect(m.lane).toBe(0);
    expect(m.edgesDown.map((e) => [e.fromLane, e.toLane])).toEqual([
      [0, 0],
      [0, 1],
    ]);
    expect(row(rows, "f").lane).toBe(1);
    expect(row(rows, "b").lane).toBe(0);
    expect(row(rows, "b").color).toBe(m.color);
    expect(row(rows, "f").color).not.toBe(m.color);

    const a = row(rows, "a");
    expect(a.lane).toBe(0);
    expect(a.edgesUp.map((e) => [e.fromLane, e.toLane])).toEqual([
      [0, 0],
      [1, 0],
    ]);
    expect(a.edgesUp[1].color).toBe(row(rows, "f").color);
    expectRowsConnect(rows);
  });

  it("opens a lane per extra parent for an octopus merge", () => {
    const { rows, maxLanes } = layoutGraph([
      c("o", "p1", "p2", "p3"),
      c("p1", "base"),
      c("p2", "base"),
      c("p3", "base"),
      c("base"),
    ]);
    expect(maxLanes).toBe(3);
    const o = row(rows, "o");
    expect(o.edgesDown.map((e) => [e.fromLane, e.toLane])).toEqual([
      [0, 0],
      [0, 1],
      [0, 2],
    ]);
    expect(new Set(o.edgesDown.map((e) => e.color)).size).toBe(3);
    expect(row(rows, "p1").lane).toBe(0);
    expect(row(rows, "p2").lane).toBe(1);
    expect(row(rows, "p3").lane).toBe(2);
    const base = row(rows, "base");
    expect(base.lane).toBe(0);
    expect(base.edgesUp.map((e) => e.toLane)).toEqual([0, 0, 0]);
    expect(base.edgesDown).toEqual([]);
    expectRowsConnect(rows);
  });

  it("reuses a lane already expecting an extra parent", () => {
    const { rows, maxLanes } = layoutGraph([
      c("x", "a"),
      c("m", "b", "a"),
      c("b", "a"),
      c("a"),
    ]);
    expect(maxLanes).toBe(2);
    const m = row(rows, "m");
    expect(m.lane).toBe(1);
    expect(m.edgesDown.map((e) => [e.fromLane, e.toLane])).toEqual([
      [1, 0],
      [1, 1],
    ]);
    expectRowsConnect(rows);
  });

  it("keeps lanes open for parents not loaded yet", () => {
    const { rows, maxLanes } = layoutGraph([c("m", "b", "f"), c("b", "z")]);
    expect(maxLanes).toBe(2);
    const last = rows[rows.length - 1];
    expect(last.edgesDown.map((e) => [e.fromLane, e.toLane])).toEqual([
      [0, 0],
      [1, 1],
    ]);
    expectRowsConnect(rows);
  });

  it("does not change first-page rows when a page is appended", () => {
    const page1 = [c("m", "b", "f"), c("f", "e"), c("b", "a")];
    const page2 = [c("e", "a"), c("a", "root"), c("root")];
    const first = layoutGraph(page1);
    const both = layoutGraph([...page1, ...page2]);
    expect(both.rows.slice(0, page1.length)).toEqual(first.rows);
    expect(both.rows[page1.length - 1].edgesDown.map((e) => e.toLane)).toEqual(
      both.rows[page1.length].edgesUp.map((e) => e.fromLane),
    );
    expectRowsConnect(both.rows);
  });

  it("starts a new lane for an unexpected branch tip", () => {
    const { rows } = layoutGraph([c("t1", "a"), c("t2", "b"), c("a"), c("b")]);
    expect(rows.map((r) => r.lane)).toEqual([0, 1, 0, 1]);
    expect(rows[0].color).not.toBe(rows[1].color);
    expectRowsConnect(rows);
  });

  it("is deterministic and keeps colors within the palette", () => {
    const commits = [
      c("o", "p1", "p2", "p3"),
      c("t", "p2"),
      c("p1", "base"),
      c("p2", "base"),
      c("p3", "base"),
      c("base"),
    ];
    expect(layoutGraph(commits)).toEqual(layoutGraph(commits));
    for (const r of layoutGraph(commits).rows) {
      for (const color of [r.color, ...r.edgesUp.map((e) => e.color), ...r.edgesDown.map((e) => e.color)]) {
        expect(color).toBeGreaterThanOrEqual(0);
        expect(color).toBeLessThan(PALETTE_SIZE);
      }
    }
  });
});
