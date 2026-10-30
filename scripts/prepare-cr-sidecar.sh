#!/usr/bin/env bash
# Place the cr binary in src-tauri/binaries/ so `pnpm tauri build` can bundle
# it as a sidecar. Only needed for local production builds — `pnpm tauri dev`
# does not bundle sidecars and works without this script.
set -euo pipefail

TRIPLE="$(rustc -Vv | grep '^host:' | awk '{print $2}')"
EXT=""
[[ "$TRIPLE" == *"windows"* ]] && EXT=".exe"

echo "Building cr for $TRIPLE..."
cargo build --release --bin cr --manifest-path src-tauri/Cargo.toml

mkdir -p src-tauri/binaries
cp "src-tauri/target/release/cr${EXT}" "src-tauri/binaries/cr-${TRIPLE}${EXT}"
chmod +x "src-tauri/binaries/cr-${TRIPLE}${EXT}" || true

# Config override used by `pnpm tauri build` to inject externalBin. Skipped on Linux:
# the bundler already ships the `cr` [[bin]] as /usr/bin/cr, and the sidecar would
# add the same path twice (the rpm bundler rejects that).
if [[ "$TRIPLE" == *"linux"* ]]; then
  echo '{}' > src-tauri/tauri.sidecar.conf.json
else
  echo '{"bundle":{"externalBin":["binaries/cr"]}}' > src-tauri/tauri.sidecar.conf.json
fi

echo "Done → src-tauri/binaries/cr-${TRIPLE}${EXT}"
