/**
 * The fixed set of design directions. Each direction is a `cr-{id}` class in
 * src/styles/tokens.css that defines its fonts, radii, and (with `.dark`/`.light`)
 * the full token palette. Adding a direction: see THEMING.md.
 */
export type Direction = "a" | "b" | "c";

export const DIRECTIONS: { id: Direction; label: string; blurb: string }[] = [
  { id: "a", label: "Continuity", blurb: "IBM Plex · blue · slate-navy" },
  { id: "b", label: "Modern", blurb: "Manrope · indigo · near-black" },
  { id: "c", label: "Terminal", blurb: "JetBrains Mono · green · sharp" },
];
