import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { connDetail, connLabel, hostsQuery, readableRemoteError, WORKER_INSTALL_HINT, useHostStates } from "../lib/hosts";
import type { Host, HostInput, HostTest } from "../lib/types";

const BLANK: HostInput = { id: null, name: "", ssh_target: "", remote_endpoint: "", enabled: true };

/** Settings → AI → Hosts: the machines running `cr worker` that profiles can point at. */
export function AiHostsSection() {
  const queryClient = useQueryClient();
  const hosts = useQuery(hostsQuery());
  const states = useHostStates();
  const [draft, setDraft] = useState<HostInput | null>(null);
  const [tests, setTests] = useState<Record<number, HostTest | string | "pending">>({});

  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["hosts"] });
    queryClient.invalidateQueries({ queryKey: ["host-states"] });
  };
  const save = useMutation({
    mutationFn: api.hostSave,
    onSuccess: () => {
      refresh();
      setDraft(null);
    },
    onError: (e) => toast.error(String(e)),
  });
  const remove = useMutation({ mutationFn: api.hostDelete, onSuccess: refresh, onError: (e) => toast.error(String(e)) });

  const test = (h: Host) => {
    setTests((t) => ({ ...t, [h.id]: "pending" }));
    api
      .hostTest(h.id)
      .then((r) => setTests((t) => ({ ...t, [h.id]: r })))
      .catch((e) => setTests((t) => ({ ...t, [h.id]: readableRemoteError(String(e)) })))
      .finally(refresh);
  };
  const confirmDelete = async (h: Host) => {
    if (await confirmDialog({ title: "Remove host", message: `Remove “${h.name}”? Its past runs stay in the history.`, confirmLabel: "Remove", danger: true }))
      remove.mutate(h.id);
  };
  const copyInstall = () =>
    navigator.clipboard.writeText(WORKER_INSTALL_HINT).then(
      () => toast.info("Command copied to clipboard"),
      (e) => toast.error(`Could not copy: ${String(e)}`),
    );

  return (
    <section className="settings-group" data-testid="ai-hosts">
      <h3>Hosts</h3>
      <span className="settings-hint muted">
        A host is another machine running the <code>cr worker</code> daemon, reached through an SSH tunnel with your
        <code> ~/.ssh/config</code>. Profiles pointing at a host run plain routines and one-shot features there. Nothing
        secret is stored. On the host, run <code>{WORKER_INSTALL_HINT}</code>{" "}
        <button type="button" className="btn btn-sm" onClick={copyInstall}>
          Copy
        </button>
      </span>
      {hosts.isLoading && <span className="muted">Loading…</span>}
      {hosts.isError && <span className="error">{String(hosts.error)}</span>}
      {(hosts.data ?? []).map((h) => {
        const state = states.get(h.id);
        const detail = connDetail(state);
        const t = tests[h.id];
        return (
          <div key={h.id} className="settings-row settings-row-stack ai-host-row">
            <span className="ai-host-head">
              <strong>{h.name}</strong>{" "}
              <span className={state?.state === "connected" ? "badge" : state?.state === "unreachable" || state?.state === "protocol_mismatch" ? "badge badge-danger" : "badge"}>
                {h.enabled ? connLabel(state) : "disabled"}
              </span>
              <span className="muted">
                {" "}
                {h.ssh_target} · {h.remote_endpoint}
              </span>
              <span className="ai-profile-actions">
                <button className="btn btn-sm" disabled={t === "pending" || !h.enabled} onClick={() => test(h)}>
                  {t === "pending" ? "Testing…" : "Test connection"}
                </button>
                <button className="btn btn-sm" onClick={() => setDraft({ id: h.id, name: h.name, ssh_target: h.ssh_target, remote_endpoint: h.remote_endpoint, enabled: h.enabled })}>
                  Edit
                </button>
                <button className="btn btn-sm" onClick={() => confirmDelete(h)}>
                  Remove
                </button>
              </span>
            </span>
            {detail && <span className={`settings-hint ${state?.state === "connected" ? "muted" : "form-warning"}`}>{detail}</span>}
            {!detail && h.last_version && (
              <span className="settings-hint muted">
                Last seen {h.last_seen ?? "never"} · cr {h.last_version}
              </span>
            )}
            {t && t !== "pending" && typeof t === "string" && <span className="settings-hint form-warning">{t}</span>}
            {t && typeof t === "object" && (
              <span className="settings-hint muted" data-testid="host-test-result">
                Connected to {t.host || h.name}: cr {t.version} (protocol {t.protocol}); harnesses: {t.harnesses.join(", ") || "none installed"}; {t.running} running, {t.queued} queued (max {t.max_concurrency}).
              </span>
            )}
          </div>
        );
      })}
      {!draft && (
        <button className="btn btn-sm" onClick={() => setDraft({ ...BLANK })}>
          Add host
        </button>
      )}
      {draft && (
        <form
          className="settings-row settings-row-stack ai-profile-form"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate(draft);
          }}
        >
          <label>
            Name
            <input className="input" value={draft.name} required onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
          </label>
          <label>
            SSH target
            <input
              className="input"
              placeholder="alias from ~/.ssh/config"
              value={draft.ssh_target}
              required
              onChange={(e) => setDraft({ ...draft, ssh_target: e.target.value })}
            />
          </label>
          <label>
            Daemon socket or port on the host
            <input
              className="input mono"
              placeholder="/run/user/1000/codereview/worker.sock"
              value={draft.remote_endpoint}
              required
              onChange={(e) => setDraft({ ...draft, remote_endpoint: e.target.value })}
            />
          </label>
          <span className="settings-hint muted">
            Absolute path of the socket (run <code>echo $XDG_RUNTIME_DIR/codereview/worker.sock</code> on the host; ssh
            expands neither <code>~</code> nor variables), or a loopback port.
          </span>
          <label className="settings-check">
            <input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} /> Enabled
          </label>
          <span className="ai-profile-actions">
            <button type="button" className="btn btn-sm" onClick={() => setDraft(null)}>
              Cancel
            </button>
            <button type="submit" className="btn btn-sm btn-primary" disabled={save.isPending}>
              {draft.id == null ? "Add" : "Save"}
            </button>
          </span>
        </form>
      )}
    </section>
  );
}
