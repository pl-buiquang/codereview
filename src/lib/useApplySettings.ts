import { useEffect } from "react";
import { api } from "./api";
import { setClientLogLevel } from "./log";
import { effectiveTheme, useSettingsStore } from "./settings";
import { customThemeStyle, type CustomTheme } from "./customTheme";
import type { ThemeChoice } from "./themes";

/** localStorage copy of the custom theme's inline styles, replayed by the FOUC script in index.html. */
export const CUSTOM_THEME_CACHE_KEY = "codereview-custom-theme";

/** Inline style properties the custom theme put on <html>, to clear before the next apply. */
let appliedCustom: string[] = [];

function clearCustomStyle(root: HTMLElement) {
  for (const name of appliedCustom) root.style.removeProperty(name);
  appliedCustom = [];
  // The FOUC script set properties this module never saw: they are in the cache.
  try {
    const cached = JSON.parse(localStorage.getItem(CUSTOM_THEME_CACHE_KEY) ?? "null");
    for (const mode of ["dark", "light"]) for (const name of Object.keys(cached?.[mode] ?? {})) root.style.removeProperty(name);
  } catch {
    // no cache to read
  }
}

/**
 * Apply the active theme as `cr cr-{dir} {mode}` classes on <html> + colorScheme. The custom
 * theme rides on its base direction's classes, with its overrides as inline properties.
 */
function applyTheme(choice: ThemeChoice, custom: CustomTheme, mode: "dark" | "light") {
  const root = document.documentElement;
  const direction = choice === "custom" ? custom.base : choice;
  root.className = `cr cr-${direction} ${mode}`;
  root.style.colorScheme = mode;
  clearCustomStyle(root);
  if (choice !== "custom") {
    try {
      localStorage.removeItem(CUSTOM_THEME_CACHE_KEY);
    } catch {
      // no storage: nothing to drop
    }
    return;
  }
  for (const [name, value] of Object.entries(customThemeStyle(custom, mode))) {
    root.style.setProperty(name, value);
    appliedCustom.push(name);
  }
  try {
    localStorage.setItem(
      CUSTOM_THEME_CACHE_KEY,
      JSON.stringify({
        base: custom.base,
        dark: customThemeStyle(custom, "dark"),
        light: customThemeStyle(custom, "light"),
      }),
    );
  } catch {
    // no storage: the first paint just lacks the overrides
  }
}

/**
 * Reflect persisted appearance settings onto the document: the active theme as
 * root classes (re-resolving live when following the OS) and the
 * `--diff-font-size` variable. Call once near the app root.
 */
export function useApplySettings() {
  const direction = useSettingsStore((s) => s.direction);
  const customTheme = useSettingsStore((s) => s.customTheme);
  const mode = useSettingsStore((s) => s.mode);
  const diffFontSize = useSettingsStore((s) => s.diffFontSize);
  const logLevel = useSettingsStore((s) => s.logLevel);
  const aiEnabled = useSettingsStore((s) => s.aiEnabled);

  useEffect(() => {
    const apply = () => applyTheme(direction, customTheme, effectiveTheme(mode));
    apply();
    if (mode === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: light)");
      mq.addEventListener("change", apply);
      return () => mq.removeEventListener("change", apply);
    }
  }, [direction, customTheme, mode]);

  useEffect(() => {
    document.documentElement.style.setProperty("--diff-font-size", `${diffFontSize}px`);
  }, [diffFontSize]);

  // The backend owns the log file; push the chosen level to it (and to the
  // frontend logger) at startup and whenever it changes.
  useEffect(() => {
    setClientLogLevel(logLevel);
    if ("__TAURI_INTERNALS__" in window) api.setLogLevel(logLevel).catch(() => {});
  }, [logLevel]);

  // The backend refuses to start AI work while the switch is off; keep it in step.
  useEffect(() => {
    if ("__TAURI_INTERNALS__" in window) api.setAiEnabled(aiEnabled).catch(() => {});
  }, [aiEnabled]);
}
