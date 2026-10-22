import { useEffect, useState } from "react";
import { checkForUpdate, installAndRelaunch, type Update } from "../lib/updater";
import { toast } from "../lib/toast";

/** How often a running app re-checks for a release (it used to check only at launch). */
export const UPDATE_CHECK_INTERVAL_MS = 60 * 60 * 1000;

export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [installing, setInstalling] = useState(false);
  // Keyed by version, so dismissing v1 doesn't hide a newer v2 found later.
  const [dismissedVersion, setDismissedVersion] = useState<string | null>(null);

  useEffect(() => {
    // A failed or up-to-date check resolves null: never clear an update already found.
    const check = () => checkForUpdate().then((u) => u && setUpdate(u));
    check();
    const timer = window.setInterval(check, UPDATE_CHECK_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, []);

  if (!update || dismissedVersion === update.version) return null;

  const install = () => {
    setInstalling(true);
    installAndRelaunch(update).catch((e) => {
      toast.error(`Update failed: ${e}`);
      setInstalling(false);
    });
  };

  return (
    <div className="update-banner">
      <span className="update-banner-text">Update available: v{update.version}</span>
      <button
        type="button"
        className="btn btn-sm btn-primary"
        onClick={install}
        disabled={installing}
      >
        {installing ? "Installing…" : "Install & relaunch"}
      </button>
      <button
        type="button"
        className="btn btn-sm btn-ghost"
        onClick={() => setDismissedVersion(update.version)}
      >
        Dismiss
      </button>
    </div>
  );
}
