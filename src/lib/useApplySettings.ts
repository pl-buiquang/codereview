import { useEffect } from "react";
import { effectiveTheme, useSettingsStore } from "./settings";

/**
 * Direction is hardcoded to "a" until Phase 1 adds the `direction` store field;
 * Phase 1 swaps this for a `useSettingsStore((s) => s.direction)` read.
 */
const DIRECTION = "a";

/** Apply the active theme as `cr cr-{dir} {mode}` classes on <html> + colorScheme. */
function applyTheme(mode: "dark" | "light") {
  const root = document.documentElement;
  root.className = `cr cr-${DIRECTION} ${mode}`;
  root.style.colorScheme = mode;
}

/**
 * Reflect persisted appearance settings onto the document: the active theme as
 * root classes (re-resolving live when following the OS) and the
 * `--diff-font-size` variable. Call once near the app root.
 */
export function useApplySettings() {
  const themeMode = useSettingsStore((s) => s.themeMode);
  const diffFontSize = useSettingsStore((s) => s.diffFontSize);

  useEffect(() => {
    const apply = () => applyTheme(effectiveTheme(themeMode));
    apply();
    if (themeMode === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: light)");
      mq.addEventListener("change", apply);
      return () => mq.removeEventListener("change", apply);
    }
  }, [themeMode]);

  useEffect(() => {
    document.documentElement.style.setProperty("--diff-font-size", `${diffFontSize}px`);
  }, [diffFontSize]);
}
