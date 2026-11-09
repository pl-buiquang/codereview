import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { filterModelGroups, filterModelProviders, OPENCODE_LOGIN_COMMAND } from "../lib/aiProfiles";
import { useSettingsStore } from "../lib/settings";

/** Most models shown at once; typing narrows the list. */
const MAX_SHOWN = 60;

/** The model field of the profile editor: free text that doubles as a search over the harness'
 *  models, grouped by provider, with a refresh and an empty-state hint for opencode. */
export function ModelPicker({ harness, value, onChange, hostId = null }: { harness: string; value: string; onChange: (model: string) => void; hostId?: number | null }) {
  const queryClient = useQueryClient();
  const key = hostId == null ? ["ai-models", harness] : ["ai-models", harness, hostId];
  const models = useQuery({ queryKey: key, queryFn: () => (hostId == null ? api.aiHarnessModels(harness) : api.aiHarnessModels(harness, false, hostId)), staleTime: Infinity, retry: hostId == null });
  const [open, setOpen] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  const providers = useSettingsStore((s) => s.opencodeModelProviders);
  const available = harness === "opencode" ? filterModelProviders(models.data ?? [], providers) : (models.data ?? []);
  const groups = filterModelGroups(available, value);
  const total = available.reduce((n, g) => n + g.models.length, 0);
  let budget = MAX_SHOWN;
  const shown = groups
    .map((g) => {
      const list = g.models.slice(0, Math.max(budget, 0));
      budget -= list.length;
      return { provider: g.provider, models: list };
    })
    .filter((g) => g.models.length > 0);

  const refresh = async () => {
    setRefreshing(true);
    try {
      queryClient.setQueryData(key, await (hostId == null ? api.aiHarnessModels(harness, true) : api.aiHarnessModels(harness, true, hostId)));
    } catch (e) {
      toast.error(String(e));
    } finally {
      setRefreshing(false);
    }
  };
  const copyLogin = () =>
    navigator.clipboard.writeText(OPENCODE_LOGIN_COMMAND).then(
      () => toast.info("Command copied to clipboard"),
      (e) => toast.error(`Could not copy: ${String(e)}`),
    );

  const empty = models.isSuccess && total === 0;
  return (
    <div className="model-picker">
      <label>
        Model
        <input
          className="input"
          placeholder="leave empty for the harness' own default"
          value={value}
          onChange={(e) => {
            onChange(e.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onBlur={() => setTimeout(() => setOpen(false), 150)}
        />
      </label>
      {open && shown.length > 0 && (
        <ul className="model-picker-list" role="listbox" aria-label="Models">
          {shown.map((g) => (
            <li key={g.provider} role="presentation">
              <div className="model-picker-group">{g.provider || "Suggestions"}</div>
              <ul role="presentation">
                {g.models.map((m) => (
                  <li key={m} role="option" aria-selected={m === value}>
                    <button
                      type="button"
                      className="model-picker-option"
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => {
                        onChange(m);
                        setOpen(false);
                      }}
                    >
                      {m}
                    </button>
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      )}
      {open && shown.length > 0 && groups.reduce((n, g) => n + g.models.length, 0) > MAX_SHOWN && (
        <span className="muted model-picker-hint">Showing the first {MAX_SHOWN}; type to narrow the list.</span>
      )}
      {models.isError && <span className="muted model-picker-hint">Could not list models: {String(models.error)}</span>}
      {empty && harness === "opencode" && (
        <span className="muted model-picker-hint">
          opencode lists no model yet. Run <code>{OPENCODE_LOGIN_COMMAND}</code> to add a provider, then refresh.{" "}
          <button type="button" className="btn btn-sm" onClick={copyLogin}>
            Copy command
          </button>
        </span>
      )}
      {harness !== "claude" && (
        <span className="model-picker-hint">
          <button type="button" className="btn btn-sm" onClick={refresh} disabled={refreshing}>
            {refreshing ? "Refreshing…" : "Refresh models"}
          </button>
        </span>
      )}
    </div>
  );
}
