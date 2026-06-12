/** First non-empty line of a comment body, trimmed and hard-capped for one-line display. */
export function summaryLine(body: string, max = 80): string {
  const line = body.split("\n").find((l) => l.trim() !== "")?.trim() ?? "";
  return line.length > max ? line.slice(0, max - 1) + "…" : line;
}
