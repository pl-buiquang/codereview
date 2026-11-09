import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { ConnState, Host, HostState } from "./types";

export const HOST_STATE_EVENT = "host-state";

/** Where `cr serve` should be installed on a host; shown next to the connection test. */
export const SERVE_INSTALL_HINT = "cr serve --install";

const DISCONNECTED: ConnState = { state: "disconnected" };

/** The connection state of every host, kept fresh by the backend's `host-state` events. */
export function useHostStates(): Map<number, ConnState> {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: ["host-states"], queryFn: api.hostStates, refetchInterval: 30_000 });
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let disposed = false;
    let dispose: (() => void) | undefined;
    listen<HostState>(HOST_STATE_EVENT, ({ payload }) => {
      queryClient.setQueryData<HostState[]>(["host-states"], (old) => [
        ...(old ?? []).filter((s) => s.host_id !== payload.host_id),
        payload,
      ]);
    })
      .then((d) => (disposed ? d() : (dispose = d)))
      .catch((e) => console.warn("Could not register the host-state listener", e));
    return () => {
      disposed = true;
      dispose?.();
    };
  }, [queryClient]);
  return new Map((query.data ?? []).map((s) => [s.host_id, s.state]));
}

export const hostsQuery = () => ({ queryKey: ["hosts"], queryFn: api.hostList });

/** The name of host `id`, or a stand-in for one that was removed. */
export function hostName(hosts: Host[] | undefined, id: number | null): string | null {
  if (id == null) return null;
  return hosts?.find((h) => h.id === id)?.name ?? `host #${id}`;
}

export function connLabel(s: ConnState | undefined): string {
  switch (s?.state ?? "disconnected") {
    case "connected":
      return "connected";
    case "connecting":
      return "connecting…";
    case "unreachable":
      return "unreachable";
    case "protocol_mismatch":
      return "protocol mismatch";
    default:
      return "not connected";
  }
}

/** A line explaining the state, when it needs one. */
export function connDetail(s: ConnState | undefined): string | null {
  if (!s) return null;
  if (s.state === "connected") return `cr ${s.version} · protocol ${s.protocol}`;
  if (s.state === "unreachable") return s.error;
  if (s.state === "protocol_mismatch") {
    return `The host runs cr ${s.remote_version} (protocol ${s.remote_protocol}); this app speaks protocol ${s.local_protocol}. Update cr on the host to the app's version, restart its daemon (${SERVE_INSTALL_HINT}), then test the connection.`;
  }
  return null;
}

export const isUsable = (s: ConnState | undefined): boolean => (s ?? DISCONNECTED).state !== "protocol_mismatch";

const CODE_TEXT: Record<string, string> = {
  host_unreachable: "The host could not be reached",
  protocol_mismatch: "The host runs an incompatible cr version",
  host_disabled: "The host is disabled",
  unknown_repo: "The repository is not registered on the host",
  no_worktree: "The host could not prepare a workspace",
  harness_unavailable: "The harness is not installed on the host",
  capability_missing: "The harness on the host cannot honour this run",
  queue_full: "The host is busy",
  conflict: "The host refused a conflicting request",
  timeout: "The host timed out",
  run_not_found: "The host lost the run",
};

/** `host_unreachable: ssh: ...` as a sentence; other text is returned unchanged. */
export function readableRemoteError(error: string): string {
  const m = /^([a-z_]+): ([\s\S]*)$/.exec(error);
  const text = m && CODE_TEXT[m[1]];
  return text ? `${text}: ${m![2]}` : error;
}
