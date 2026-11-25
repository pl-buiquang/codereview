import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import {
  CR_PLATFORMS,
  RELEASES_URL,
  installCommand,
  isReleaseVersion,
  releaseTag,
  workerRestartCommand,
  workerSetupCommand,
  type CrPlatform,
} from "../lib/crRemote";
import type { CliStatus, Host } from "../lib/types";

function copy(text: string, what: string) {
  navigator.clipboard.writeText(text).then(
    () => toast.info(`Copied ${what}`),
    (e) => toast.error(String(e)),
  );
}

function CommandBlock({ label, command }: { label: string; command: string }) {
  return (
    <div className="cli-command">
      <div className="cli-command-head">
        <span>{label}</span>
        <button className="btn btn-sm" onClick={() => copy(command, label.toLowerCase())}>
          Copy
        </button>
      </div>
      <pre className="cli-command-text">{command}</pre>
    </div>
  );
}

/** How the installed `cr` compares with this app. */
function installedState(s: CliStatus): { text: string; ok: boolean } {
  const v = s.installed?.version;
  if (!s.installed) return { text: "not on PATH", ok: false };
  if (!v) return { text: "version unknown", ok: false };
  if (s.installed_info?.build === "debug") return { text: "debug build", ok: false };
  if (v === s.app_version) return { text: "same as the app", ok: true };
  return { text: `differs from the app (${s.app_version})`, ok: false };
}

export function CliSection() {
  const queryClient = useQueryClient();
  const statusQuery = useQuery({ queryKey: ["cli-status"], queryFn: api.cliStatus });
  const hostsQuery = useQuery({ queryKey: ["hosts"], queryFn: api.hostList });
  const reinstall = useMutation({
    mutationFn: api.crReinstall,
    onSuccess: (note) => {
      toast.success(note);
      queryClient.invalidateQueries({ queryKey: ["cli-status"] });
    },
    onError: (e) => toast.error(`Reinstall failed:\n${String(e)}`),
  });

  const [platform, setPlatform] = useState<CrPlatform>("x86_64-unknown-linux-gnu");
  const hosts: Host[] = hostsQuery.data ?? [];
  const [alias, setAlias] = useState<string>("");
  const target = alias || hosts[0]?.ssh_target || "";

  if (statusQuery.isLoading) return <p className="muted">Checking cr…</p>;
  if (statusQuery.isError || !statusQuery.data)
    return <p className="error">Could not check cr: {String(statusQuery.error)}</p>;

  const s = statusQuery.data;
  const state = installedState(s);
  const info = s.installed_info;
  const dbMismatch = info != null && info.db !== s.app_db;
  const tag = releaseTag(s.app_version);
  const linux = CR_PLATFORMS.find((p) => p.key === platform)?.linux ?? true;

  return (
    <>
      <section className="settings-group">
        <h3>cr on this machine</h3>
        <p className="muted">
          <code>cr</code> is the command-line twin of the app: it reads and writes the same database, and the AI
          skills, routines and chat drive the app through it.
        </p>
        <div className="settings-row">
          <span>App version</span>
          <code className="env-path">{s.app_version}</code>
        </div>
        <div className="settings-row">
          <span>Bundled cr</span>
          {s.sidecar ? (
            <code className="env-path" title={s.sidecar.path}>
              {s.sidecar.version ?? "?"} · {s.sidecar.path}
            </code>
          ) : (
            <span className="muted">
              {s.debug_build ? "None in a debug build (pnpm tauri dev)" : "None next to the app binary"}
            </span>
          )}
        </div>
        <div className="settings-row">
          <span>Installed cr</span>
          {s.installed ? (
            <span className="settings-control">
              <code className="env-path" title={s.installed.path}>
                {s.installed.version ?? "?"} · {s.installed.path}
              </code>
              <span className={state.ok ? "env-ok" : "error"}>{state.text}</span>
            </span>
          ) : (
            <span className="error">Not found on PATH</span>
          )}
        </div>
        <div className="settings-row">
          <span>Its database</span>
          {info ? (
            <span className="settings-control">
              <code className="env-path" title={info.db}>
                {info.db}
              </code>
              <span className="muted">({info.db_source === "default" ? "default" : info.db_source})</span>
              {dbMismatch && <span className="error">not the app's database</span>}
            </span>
          ) : (
            <span className="muted">
              {s.installed ? "Unknown: this cr predates `cr info`, reinstall it" : "—"}
            </span>
          )}
        </div>
        <div className="settings-row">
          <span>App database</span>
          <code className="env-path" title={s.app_db}>
            {s.app_db}
          </code>
        </div>
        <div className="settings-row">
          <span>Install to</span>
          <span className="settings-control">
            <code className="env-path">{s.install_dest ?? "—"}</code>
            <button
              className="btn btn-sm btn-primary"
              disabled={!s.sidecar || reinstall.isPending}
              title={
                s.sidecar
                  ? "Copy this app's bundled cr over the installed one, whatever its version"
                  : "No bundled cr in this build"
              }
              onClick={() => reinstall.mutate()}
            >
              {reinstall.isPending ? "Reinstalling…" : "Reinstall cr"}
            </button>
            <button className="btn btn-sm" onClick={() => statusQuery.refetch()} disabled={statusQuery.isFetching}>
              Recheck
            </button>
          </span>
        </div>
        <p className="muted">
          The app installs its bundled <code>cr</code> on launch when the installed one is missing or older:
          where <code>cr</code> already is on <code>PATH</code>, else <code>~/.cargo/bin</code>, else{" "}
          <code>~/.local/bin</code>. It never downgrades; <b>Reinstall cr</b> does, e.g. to replace a stray dev build.
        </p>
      </section>

      <section className="settings-group">
        <h3>Which database cr uses</h3>
        <ol className="cli-db-order">
          <li>
            <code>--db &lt;path&gt;</code> on the command line;
          </li>
          <li>
            else <code>CODEREVIEW_DB</code>{" "}
            {s.env_db ? (
              <>
                (set for the app: <code>{s.env_db}</code>)
              </>
            ) : (
              <span className="muted">(not set for the app)</span>
            )}
            ;
          </li>
          <li>
            else the app-data default: <code>com.codereview.app/codereview.db</code> for a release <code>cr</code>,{" "}
            <code>com.codereview.app.dev/codereview-dev.db</code> for a debug one (the dev app's).
          </li>
        </ol>
        <p className="muted">
          The app follows the same rule without <code>--db</code>, so a release app and a release <code>cr</code>{" "}
          share one database. Run <code>cr info</code> in a terminal to see what a given shell resolves. Inside a
          remote run, <code>cr</code> uses no database at all: writes go to the run's outbox.
        </p>
      </section>

      <section className="settings-group">
        <h3>Install on a remote host</h3>
        <p className="muted">
          A{" "}
          <a
            href={`${RELEASES_URL}/tag/${tag}`}
            onClick={(e) => {
              e.preventDefault();
              api.openUrl(`${RELEASES_URL}/tag/${tag}`).catch((err) => toast.error(String(err)));
            }}
          >
            release
          </a>{" "}
          ships a standalone <code>cr</code> per platform.
          Install the one of this app's version ({tag}) on each host that runs routines; the app refuses a host
          whose <code>cr</code> speaks another protocol.
          {!isReleaseVersion(s.app_version) && (
            <>
              {" "}
              This app is a local build ({s.app_version}), so the commands fetch release {tag}.
            </>
          )}
        </p>
        <div className="settings-row">
          <span>Host</span>
          <span className="settings-control">
            {hosts.length > 0 ? (
              <select className="select" value={target} onChange={(e) => setAlias(e.target.value)}>
                {hosts.map((h) => (
                  <option key={h.id} value={h.ssh_target}>
                    {h.name} ({h.ssh_target})
                    {h.last_version ? ` · cr ${h.last_version}` : ""}
                  </option>
                ))}
              </select>
            ) : (
              <input
                className="input"
                placeholder="ssh alias"
                value={alias}
                onChange={(e) => setAlias(e.target.value)}
              />
            )}
            <select className="select" value={platform} onChange={(e) => setPlatform(e.target.value as CrPlatform)}>
              {CR_PLATFORMS.map((p) => (
                <option key={p.key} value={p.key}>
                  {p.label}
                </option>
              ))}
            </select>
          </span>
        </div>
        {hosts.length > 0 && (
          <div className="cli-hosts">
            {hosts.map((h) => {
              const same = h.last_version != null && releaseTag(h.last_version) === tag;
              return (
                <div key={h.id} className="settings-row">
                  <span>{h.name}</span>
                  {h.last_version ? (
                    <span className={same ? "env-ok" : "error"}>
                      cr {h.last_version}
                      {same ? "" : ` (app is ${tag})`}
                    </span>
                  ) : (
                    <span className="muted">not reached yet (Test connection in Settings → AI → Hosts)</span>
                  )}
                </div>
              );
            })}
          </div>
        )}
        <CommandBlock label="Install or update cr" command={installCommand(target, tag, platform)} />
        {linux ? (
          <>
            <CommandBlock label="First time: start the daemon" command={workerSetupCommand(target)} />
            <CommandBlock label="After an update: restart the daemon" command={workerRestartCommand(target)} />
          </>
        ) : (
          <p className="muted">
            On macOS hosts, run the daemon as a launchd agent (see <code>docs/features/cli.md</code>, “Remote agent
            daemon”).
          </p>
        )}
        <p className="muted">
          Then register the repositories routines run on (<code>ssh {target || "<host>"} '~/.local/bin/cr repo add
          ~/code/&lt;repo&gt;'</code>) and add the host in Settings → AI → Hosts.
        </p>
      </section>
    </>
  );
}
