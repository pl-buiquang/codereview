# Remote hosts

A **host** is another machine running the `cr worker` daemon ([ADR 0007](../adr/0007-remote-agent-daemon.md)).
A profile that points at a host runs its harness there: plain routines (babysit, the issue routines, your own)
and one-shot features (theme suggestions, session summaries) execute on the host, with the host's
checkouts, tools and harness logins. The app keeps a **mirror** of each remote run, so the transcript, status,
cost and notifications look like those of a local run ([ADR 0008](../adr/0008-remote-hosts-and-run-mirror.md)).

What stays on this machine: review chats (they keep a session and edit a local worktree), interactive
terminal sessions, and **system routines** (focus, AI review, board overview, plan issue, image diff report),
which feed app features from local state. The pickers refuse a remote profile for them.

## Set up a host

You need SSH access to the host (key or agent; the app never prompts for a password), and on the host `git`,
`gh` (logged in) and at least one harness (`claude`, `opencode` or `codex`, each logged in there).

1. **Install `cr` on the host**, from the same build as the app (see *Versions* below). Put it where the
   daemon's environment finds it, normally `~/.local/bin/cr`: harness processes started by the daemon inherit
   its `PATH`, and a remote run calls `cr` (board, focus, `cr notify`) from the harness' shell.
2. **Register the repositories** the routines will run on, with the clone on the host:
   `cr repo add ~/code/widgets`. The host never clones by itself: a request for a repository it does not
   know fails with `unknown_repo`. The repository needs a GitHub `origin` remote (the request names
   `owner/name`).
3. **Start the daemon as a service**: `cr worker --install` (a systemd user unit, `cr-worker.service`; macOS needs a
   launchd agent, see [the CLI reference](cli.md#remote-agent-daemon-cr-worker)). On a server also run
   `loginctl enable-linger $USER` once, or the user service (and the runs it adopted) stops at logout.
   `cr worker --status` shows the unit and what the daemon answers.
4. **Add an SSH alias** to `~/.ssh/config` on your machine, so the app only needs a name:

   ```
   Host koza
       HostName koza.tv
       User lynks
       IdentityFile ~/.ssh/id_ed25519
   ```

   The app runs `ssh -N -L ... koza` with `BatchMode=yes` and `ControlMaster=no`, so anything that would ask a
   question (a passphrase not in the agent, an unknown host key) makes the connection fail with
   `host_unreachable` and ssh's own message. Run `ssh koza true` once in a terminal to accept the host key.
5. **Add the host in Settings → AI → Hosts**: a name, the SSH alias, and the daemon's endpoint on the host:
   the **absolute** socket path (`echo $XDG_RUNTIME_DIR/codereview/worker.sock` on the host, typically
   `/run/user/1000/codereview/worker.sock`; ssh expands neither `~` nor variables) or, if you started the
   daemon with `--listen 127.0.0.1:PORT`, that port. **Test connection** opens the tunnel and reports the
   host's `cr` version, protocol, harnesses and load.
6. **Create a profile** (Settings → AI → Profiles → *Runs on* the host). The harness and model pickers load from
   the host, so you only see what is installed there. Pick that profile for a plain routine in Sessions → Config
   (or make it the default profile; system routines then ask for a local one).

### A PATH that only interactive shells have

Non-interactive ssh and systemd services do not read `~/.bashrc`, so tools installed by scripts that edit it
(`~/.local/bin` for `claude`, `~/.opencode/bin` for `opencode`) are not on their `PATH`. `cr worker` adds the
usual per-user directories itself at start-up (`~/.local/bin`, `~/bin`, `~/.cargo/bin`, `~/.claude/local`,
`~/.opencode/bin`) when they exist, so the service finds the harnesses. A harness installed somewhere else is
reported as not installed (the host shows it as unavailable in the profile editor): either link it into
`~/.local/bin`, or add `Environment=PATH=...` to the unit. For your own use of `cr` over ssh, call it by full
path (`ssh koza ~/.local/bin/cr worker --status`).

## Versions

The app and the host must speak the same protocol. `GET /v1/info` reports an integer `protocol` and the `cr`
version; when the number differs the host is marked **protocol mismatch** in Settings, with both versions
and the fix, and nothing is sent to it. Update `cr` on the host to the app's version, restart the daemon
(`systemctl --user restart cr-worker`; running harnesses survive a restart), then press **Test connection**.
Updating the host is manual.

## What a remote run does

1. The app renders the prompt and system prompt locally, builds the request (neutral access and tool
   policy, the model, the timeout), names the workspace by repository and PR (`owner/name`, never a local path),
   and ships a **context snapshot** (the context's board card and linked cards, the review's focus) that
   answers `cr board list|show`, `cr board meta list` and `cr focus show` inside the run.
2. It posts the run with a stable idempotency key and follows the event stream, copying each event into the same
   run row and transcript a local run has. The host's store is authoritative while the run is live.
3. `cr board meta add|rm`, `cr board link|track`, `cr focus set|clear` and `cr notify` inside the run become
   **outbox** ops on the host. The app applies each op exactly once (and acknowledges it) while mirroring: a
   board link shows up on the card, a notification fires on your desktop.
4. Stop in the run header stops it on the host.

If the connection drops, the run stays *Running* with a **disconnected** badge, and the mirror reconnects with a
backoff and resumes after the last event it stored. Quitting the app does not stop remote runs; at the next start
the app looks every unfinished remote run up again: it resumes, or finishes a run that ended meanwhile, or marks
a run the host no longer knows as failed (*the host does not know this run any more*). A host that cannot be
reached when you launch fails the run at once with `host_unreachable`; use **Run again** once it is back.

Contexts without a GitHub repository (the board, a local virtual-PR review) cannot run remotely.

## Remote sessions

Home → Sessions also lists the sessions running on each enabled host, read-only: the daemon's
`GET /v1/sessions` returns what its harnesses report (`Harness::live_sessions`, the same discovery the
local list uses), so a session started in a terminal on the host is listed whether or not the app
started anything there. Each row shows the host badge, the harness, status, working directory, repository,
branch, and last activity; the header gets a host filter.

- **Copy ssh** is the only action. It copies `ssh -t <alias> '$SHELL -lc "..."'`, run through the host's
  login shell because harness binaries are often on its PATH only. A session in a tmux pane gets
  `tmux attach` selecting that pane (Claude reports the pane; opencode and Codex do not, so they
  get a resume command: `cd <cwd>; opencode --session <id>` or `codex resume <id>`, best effort, and it starts a second
  client on the session rather than attaching to the running one). A background Claude job gets `claude attach <job>`.
- **No local actions**: focus, resume in a local terminal, stop, summaries and board-card links are not
  offered for a remote row. Only metadata crosses the wire: no transcripts and no summaries.
- **A host that cannot be asked** shows a muted row (*unreachable*, *protocol mismatch*, *connecting...*,
  *disabled*, or the error) instead of failing the list. Looking at Sessions opens the tunnel to
  every enabled host on demand, like a launch does; a host the tunnel monitor is already retrying is not
  dialled again by the poll. Disabled hosts are skipped.
- History is not available for remote sessions (the API lists live sessions only), and the
  counts in the sidebar and on board cards stay local.

## Limits of protocol 1

- Remote chats and interactive sessions do not exist yet. `GET /v1/sessions` only lists live sessions (read-only, see above); there is no endpoint to start, stop or stream one.
- Harness-native routine settings that have no neutral form are not sent: Claude's `extra_args`
  (the allowed-tool list and the permission mode, mapped to its neutral access level, are). A warning is logged.
- A one-shot on a host runs in a scratch directory; it cannot see this machine's files.
- Usage of remote one-shots and runs is recorded locally with the host (Settings → Usage lists "on host").

## Troubleshooting

| Message | Meaning |
|---|---|
| `host_unreachable: ...` | The tunnel could not be opened or the daemon did not answer; the text after the colon is ssh's (or the client's) own message. Check `ssh <alias> true`, that the daemon runs (`cr worker --status` on the host) and the endpoint path. |
| `protocol_mismatch: ...` | See *Versions*. |
| `host_disabled: ...` | The host is switched off in Settings, or was removed. |
| `unknown_repo` | Register the repository on the host with `cr repo add`. |
| `harness_unavailable` | The harness is not installed or not on the daemon's PATH there. |
| `capability_missing` | The harness on the host cannot give the access or tools the routine asks for. |
| `no_worktree` | The host could not prepare the workspace (no clone, or `gh` there cannot verify the PR head). |
| `queue_full` | The host already has 100 runs queued or running. |
