import { describe, expect, it } from "vitest";
import { connDetail, connLabel, hostName, isUsable, readableRemoteError } from "./hosts";
import type { Host } from "./types";

const host = (id: number, name: string): Host => ({
  id, name, type: "ssh", ssh_target: name, remote_endpoint: "7777", enabled: true, last_seen: null, last_protocol: null, last_version: null,
});

describe("hosts helpers", () => {
  it("names a host, or a removed one by its id", () => {
    expect(hostName([host(1, "koza")], 1)).toBe("koza");
    expect(hostName([host(1, "koza")], 9)).toBe("host #9");
    expect(hostName(undefined, null)).toBeNull();
  });

  it("labels and explains each connection state", () => {
    expect(connLabel(undefined)).toBe("not connected");
    expect(connLabel({ state: "connected", protocol: 1, version: "0.6.0" })).toBe("connected");
    expect(connDetail({ state: "connected", protocol: 1, version: "0.6.0" })).toBe("cr 0.6.0 · protocol 1");
    expect(connDetail({ state: "unreachable", error: "refused" })).toBe("refused");
    const mismatch = connDetail({ state: "protocol_mismatch", remote_protocol: 2, remote_version: "0.9.0", local_protocol: 1 });
    expect(mismatch).toContain("cr 0.9.0 (protocol 2)");
    expect(mismatch).toContain("protocol 1");
    expect(mismatch).toContain("cr serve --install");
  });

  it("treats only a protocol mismatch as unusable", () => {
    expect(isUsable({ state: "unreachable", error: "x" })).toBe(true);
    expect(isUsable({ state: "protocol_mismatch", remote_protocol: 2, remote_version: "x", local_protocol: 1 })).toBe(false);
  });

  it("turns remote error codes into sentences", () => {
    expect(readableRemoteError("host_unreachable: ssh: refused")).toBe("The host could not be reached: ssh: refused");
    expect(readableRemoteError("unknown_repo: acme/x")).toBe("The repository is not registered on the host: acme/x");
    expect(readableRemoteError("The host could not be reached: already readable")).toBe("The host could not be reached: already readable");
    expect(readableRemoteError("something else")).toBe("something else");
  });
});
