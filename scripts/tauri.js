#!/usr/bin/env node
// Wrapper for the tauri CLI. For `build`, auto-prepares the cr sidecar binary
// and injects the externalBin config overlay so `pnpm tauri build` just works.
import { spawnSync } from "child_process";

function run(cmd, args = [], opts = {}) {
  const result = spawnSync(cmd, args, { stdio: "inherit", shell: true, ...opts });
  if (result.status !== null && result.status !== 0) process.exit(result.status);
  if (result.signal) process.exit(1);
}

const [cmd, ...rest] = process.argv.slice(2);

if (cmd === "build") {
  run("bash", ["scripts/prepare-cr-sidecar.sh"]);
  run("tauri", ["build", ...rest, "--config", "src-tauri/tauri.sidecar.conf.json"]);
} else {
  run("tauri", [cmd, ...rest].filter(Boolean));
}
