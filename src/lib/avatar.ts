const cache = new Map<string, string>();

function fnv1a(input: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < input.length; i++) {
    h ^= input.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Deterministic GitHub-style 5×5 mirrored pixel avatar as an SVG data URL, memoized per seed. */
export function identicon(seed: string): string {
  const key = seed.trim().toLowerCase();
  const hit = cache.get(key);
  if (hit) return hit;

  const h = fnv1a(key);
  const hue = (h >>> 15) % 360;
  const fg = `hsl(${hue} 55% 52%)`;
  let rects = "";
  for (let y = 0; y < 5; y++) {
    for (let x = 0; x < 3; x++) {
      if (((h >>> (y * 3 + x)) & 1) === 0) continue;
      rects += `<rect x="${x + 1}" y="${y + 1}" width="1" height="1"/>`;
      if (x < 2) rects += `<rect x="${5 - x}" y="${y + 1}" width="1" height="1"/>`;
    }
  }
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 7 7" shape-rendering="crispEdges">` +
    `<rect width="7" height="7" fill="hsl(${hue} 30% 92%)"/><g fill="${fg}">${rects}</g></svg>`;
  const url = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  cache.set(key, url);
  return url;
}
