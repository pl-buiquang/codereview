import { create } from "zustand";
import type { CustomTheme } from "./customTheme";
import { useSettingsStore } from "./settings";

/** Snapshots kept for undo. */
export const HISTORY_LIMIT = 50;
/** A colour picker fires per drag frame: edits of the same field this close count as one step. */
export const COALESCE_MS = 600;

interface HistoryState {
  past: CustomTheme[];
  future: CustomTheme[];
  /** Field of the last edit, for coalescing. */
  lastKey: string | null;
  lastAt: number;
}

/**
 * Undo/redo of the custom theme's edits (not presets or Mode). In memory only: it lives until
 * the app restarts, and is not part of any backup.
 */
export const useThemeHistory = create<HistoryState>(() => ({ past: [], future: [], lastKey: null, lastAt: 0 }));

function setTheme(theme: CustomTheme) {
  useSettingsStore.getState().setCustomTheme(theme);
}

/**
 * Edit the custom theme as one undo step. `update` gets the latest theme (colour drags come
 * faster than renders). Edits with the same `key` within `COALESCE_MS` merge into one step.
 */
export function editCustomTheme(update: (theme: CustomTheme) => CustomTheme, key: string | null = null, now = Date.now()) {
  const current = useSettingsStore.getState().customTheme;
  const next = update(current);
  if (JSON.stringify(next) === JSON.stringify(current)) return;
  const h = useThemeHistory.getState();
  const coalesce = key !== null && h.lastKey === key && now - h.lastAt < COALESCE_MS;
  useThemeHistory.setState({
    past: coalesce ? h.past : [...h.past, current].slice(-HISTORY_LIMIT),
    future: [],
    lastKey: key,
    lastAt: now,
  });
  setTheme(next);
}

export function undoCustomTheme() {
  const h = useThemeHistory.getState();
  const previous = h.past[h.past.length - 1];
  if (!previous) return;
  const current = useSettingsStore.getState().customTheme;
  useThemeHistory.setState({ past: h.past.slice(0, -1), future: [...h.future, current], lastKey: null });
  setTheme(previous);
}

export function redoCustomTheme() {
  const h = useThemeHistory.getState();
  const next = h.future[h.future.length - 1];
  if (!next) return;
  const current = useSettingsStore.getState().customTheme;
  useThemeHistory.setState({ future: h.future.slice(0, -1), past: [...h.past, current], lastKey: null });
  setTheme(next);
}

/** Whether a ⌘Z / Ctrl+Z press should be left to the focused control (text fields undo their own text). */
export function ownsUndo(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.tagName === "TEXTAREA") return true;
  if (target.tagName !== "INPUT") return false;
  const type = (target as HTMLInputElement).type;
  return !["color", "range", "checkbox", "radio", "button"].includes(type);
}
