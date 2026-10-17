import { useEffect } from "react";
import { api } from "./api";
import { setClientLogLevel } from "./log";
import { effectiveTheme, useSettingsStore } from "./settings";

/** Apply the active theme as `cr cr-{dir} {mode}` classes on <html> + colorScheme. */
function applyTheme(direction: string, mode: "dark" | "light") {
  const root = document.documentElement;
  root.className = `cr cr-${direction} ${mode}`;
  root.style.colorScheme = mode;
}

/**
 * Reflect persisted appearance settings onto the document: the active theme as
 * root classes (re-resolving live when following the OS) and the
 * `--diff-font-size` variable. Call once near the app root.
 */
export function useApplySettings() {
  const direction = useSettingsStore((s) => s.direction);
  const mode = useSettingsStore((s) => s.mode);
  const diffFontSize = useSettingsStore((s) => s.diffFontSize);
  const logLevel = useSettingsStore((s) => s.logLevel);

  useEffect(() => {
    const apply = () => applyTheme(direction, effectiveTheme(mode));
    apply();
    if (mode === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: light)");
      mq.addEventListener("change", apply);
      return () => mq.removeEventListener("change", apply);
    }
  }, [direction, mode]);

  useEffect(() => {
    document.documentElement.style.setProperty("--diff-font-size", `${diffFontSize}px`);
  }, [diffFontSize]);

  // The backend owns the log file; push the chosen level to it (and to the
  // frontend logger) at startup and whenever it changes.
  useEffect(() => {
    setClientLogLevel(logLevel);
    if ("__TAURI_INTERNALS__" in window) api.setLogLevel(logLevel).catch(() => {});
  }, [logLevel]);
}
