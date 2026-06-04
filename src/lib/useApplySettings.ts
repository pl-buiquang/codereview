import { useEffect } from "react";
import { effectiveTheme, useSettingsStore } from "./settings";

/**
 * Reflect persisted appearance settings onto the document: `data-theme` on
 * <html> (with a live listener when following the OS) and the `--diff-font-size`
 * CSS variable. Call once near the app root.
 */
export function useApplySettings() {
  const theme = useSettingsStore((s) => s.theme);
  const diffFontSize = useSettingsStore((s) => s.diffFontSize);

  useEffect(() => {
    const root = document.documentElement;
    const apply = () => {
      root.dataset.theme = effectiveTheme(theme);
    };
    apply();
    if (theme === "system") {
      const mq = window.matchMedia("(prefers-color-scheme: light)");
      mq.addEventListener("change", apply);
      return () => mq.removeEventListener("change", apply);
    }
  }, [theme]);

  useEffect(() => {
    document.documentElement.style.setProperty("--diff-font-size", `${diffFontSize}px`);
  }, [diffFontSize]);
}
