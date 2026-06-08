import { useEffect } from "react";
import {
  resolveActiveTheme,
  tokenVar,
  TOKEN_ROLES,
  UI_VAR,
  useSettingsStore,
  type Theme,
  type UiColors,
} from "./settings";

/** Write every token of a resolved theme onto <html> as inline CSS variables. */
function applyTheme(theme: Theme) {
  const root = document.documentElement;
  (Object.keys(UI_VAR) as (keyof UiColors)[]).forEach((key) => {
    root.style.setProperty(UI_VAR[key], theme.ui[key]);
  });
  TOKEN_ROLES.forEach((role) => {
    root.style.setProperty(tokenVar(role), theme.syntax[role]);
  });
  if (theme.codeFont) root.style.setProperty("--code-font", theme.codeFont);
  else root.style.removeProperty("--code-font");
  root.style.colorScheme = theme.base;
}

/**
 * Reflect persisted appearance settings onto the document: the active theme's
 * colors/font as inline CSS variables (re-resolving live when following the OS)
 * and the `--diff-font-size` variable. Call once near the app root.
 */
export function useApplySettings() {
  const themeMode = useSettingsStore((s) => s.themeMode);
  const customThemes = useSettingsStore((s) => s.customThemes);
  const darkThemeId = useSettingsStore((s) => s.darkThemeId);
  const lightThemeId = useSettingsStore((s) => s.lightThemeId);
  const diffFontSize = useSettingsStore((s) => s.diffFontSize);

  useEffect(() => {
    const apply = () =>
      applyTheme(resolveActiveTheme({ themeMode, customThemes, darkThemeId, lightThemeId }));
    apply();
    if (themeMode === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: light)");
      mq.addEventListener("change", apply);
      return () => mq.removeEventListener("change", apply);
    }
  }, [themeMode, customThemes, darkThemeId, lightThemeId]);

  useEffect(() => {
    document.documentElement.style.setProperty("--diff-font-size", `${diffFontSize}px`);
  }, [diffFontSize]);
}
