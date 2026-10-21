export interface SnoozeOption {
  label: string;
  ms: number;
}

/** One line of comma-separated durations: a number then h(ours), d(ays) or w(eeks), e.g. "2h, 1d, 1w". */
export const SNOOZE_OPTIONS_RE = /^\s*\d+[hdw](\s*,\s*\d+[hdw])*\s*$/;
export const DEFAULT_SNOOZE_OPTIONS = "2h, 1d, 1w";

const UNIT_MS = { h: 3_600_000, d: 86_400_000, w: 604_800_000 } as const;

export function isValidSnoozeOptions(raw: string): boolean {
  return SNOOZE_OPTIONS_RE.test(raw);
}

/** Snooze durations from the setting; falls back to the defaults when the line is invalid. */
export function parseSnoozeOptions(raw: string): SnoozeOption[] {
  const line = isValidSnoozeOptions(raw) ? raw : DEFAULT_SNOOZE_OPTIONS;
  return line.split(",").map((part) => {
    const label = part.trim();
    return { label, ms: Number(label.slice(0, -1)) * UNIT_MS[label.slice(-1) as keyof typeof UNIT_MS] };
  });
}
