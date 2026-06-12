import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type { Update };

/**
 * One-shot update check. Resolves null when up to date, when running in dev,
 * or on ANY failure (offline, bad endpoint, …). Never throws — app startup
 * must not surface updater noise.
 */
export async function checkForUpdate(): Promise<Update | null> {
  if (import.meta.env.DEV) return null;
  try {
    return await check();
  } catch (err) {
    console.warn("update check failed:", err);
    return null;
  }
}

/** Download + verify + install the update, then restart the app. Throws on failure. */
export async function installAndRelaunch(update: Update): Promise<void> {
  await update.downloadAndInstall();
  await relaunch();
}
