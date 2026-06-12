import { useEffect, useState } from "react";
import { checkForUpdate, installAndRelaunch, type Update } from "../lib/updater";
import { toast } from "../lib/toast";

export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [installing, setInstalling] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    checkForUpdate().then(setUpdate);
  }, []);

  if (!update || dismissed) return null;

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
        onClick={() => setDismissed(true)}
      >
        Dismiss
      </button>
    </div>
  );
}
