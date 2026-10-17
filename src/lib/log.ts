import { invoke as rawInvoke } from "@tauri-apps/api/core";

export type LogLevel = "error" | "warn" | "info" | "debug";

export const DEFAULT_LOG_LEVEL: LogLevel = "warn";

const RANK: Record<LogLevel, number> = { error: 0, warn: 1, info: 2, debug: 3 };

let current: LogLevel = DEFAULT_LOG_LEVEL;

/** Set by the settings store so lines below the chosen level never cross the IPC bridge. */
export function setClientLogLevel(level: LogLevel) {
  current = level;
}

export function logEnabled(level: LogLevel): boolean {
  return RANK[level] <= RANK[current];
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function emit(level: LogLevel, target: string, message: string) {
  if (!logEnabled(level)) return;
  console[level === "debug" ? "debug" : level](`[${target}] ${message}`);
  if (!isTauriRuntime()) return;
  // Uses the raw invoke: the traced one would log this very call.
  rawInvoke("frontend_log", { level, target, message }).catch(() => {});
}

/** Frontend logger. Lines land in the same file as the backend's (target `ui:<target>`). */
export const log = {
  debug: (target: string, message: string) => emit("debug", target, message),
  info: (target: string, message: string) => emit("info", target, message),
  warn: (target: string, message: string) => emit("warn", target, message),
  error: (target: string, message: string) => emit("error", target, message),
};

/** Compact, bounded rendering of a value for a log line. */
export function brief(value: unknown, max = 300): string {
  let text: string;
  try {
    text = typeof value === "string" ? value : JSON.stringify(value) ?? String(value);
  } catch {
    text = String(value);
  }
  return text.length > max ? `${text.slice(0, max)}…(${text.length} chars)` : text;
}
