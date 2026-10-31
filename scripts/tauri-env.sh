#!/usr/bin/env bash
# Provide the Tauri updater signing env (TAURI_SIGNING_PRIVATE_KEY[_PASSWORD]).
# The key password is kept in the OS keyring (macOS Keychain / libsecret via secret-tool).
#
#   scripts/tauri-env.sh store             save the key password to the keyring
#   scripts/tauri-env.sh clear             remove it from the keyring
#   scripts/tauri-env.sh pnpm tauri build  run a command with the env set
#   eval "$(scripts/tauri-env.sh --export)"  export the env into the current shell
#
# Override the key location with CODEREVIEW_KEY_FILE (default ~/.tauri/codereview.key).
set -euo pipefail

KEY_FILE="${CODEREVIEW_KEY_FILE:-$HOME/.tauri/codereview.key}"
SERVICE="codereview-tauri-signing"
ACCOUNT="${USER:-codereview}"

keyring_get() {
  case "$(uname -s)" in
    Darwin) security find-generic-password -s "$SERVICE" -a "$ACCOUNT" -w 2>/dev/null ;;
    *) command -v secret-tool >/dev/null && secret-tool lookup service "$SERVICE" account "$ACCOUNT" 2>/dev/null ;;
  esac
}

keyring_set() {
  case "$(uname -s)" in
    Darwin) security add-generic-password -U -s "$SERVICE" -a "$ACCOUNT" -w "$1" ;;
    *)
      command -v secret-tool >/dev/null || { echo "secret-tool not found (install libsecret-tools)" >&2; return 1; }
      printf %s "$1" | secret-tool store --label="CodeReview updater key password" service "$SERVICE" account "$ACCOUNT"
      ;;
  esac
}

keyring_clear() {
  case "$(uname -s)" in
    Darwin) security delete-generic-password -s "$SERVICE" -a "$ACCOUNT" >/dev/null ;;
    *) secret-tool clear service "$SERVICE" account "$ACCOUNT" ;;
  esac
}

ask_password() {
  local prompt="$1" pw=""
  if { : </dev/tty; } 2>/dev/null; then
    read -rsp "$prompt" pw </dev/tty >&2 || true
    echo >&2
    printf %s "$pw"
  elif [[ -n "${SSH_ASKPASS:-}" ]]; then
    "$SSH_ASKPASS" "$prompt"
  elif [[ ! -t 0 ]]; then
    # no terminal: take the password from stdin (e.g. piped from a password manager)
    IFS= read -r pw || true
    printf %s "$pw"
  else
    echo "no terminal or SSH_ASKPASS available to ask for the password" >&2
    return 1
  fi
}

load_env() {
  [[ -f "$KEY_FILE" ]] || { echo "key file not found: $KEY_FILE" >&2; exit 1; }
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")"
  if [[ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]]; then
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(keyring_get || true)"
  fi
  if [[ -z "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" ]]; then
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(ask_password "Updater key password (empty if none): ")"
  fi
  export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
}

case "${1:-}" in
  store)
    pw="$(ask_password "Updater key password: ")"
    [[ -n "$pw" ]] || { echo "empty password, nothing stored" >&2; exit 1; }
    keyring_set "$pw"
    echo "stored in keyring ($SERVICE)" >&2
    ;;
  clear)
    keyring_clear
    echo "removed from keyring" >&2
    ;;
  --export)
    load_env
    printf 'export TAURI_SIGNING_PRIVATE_KEY=%q\nexport TAURI_SIGNING_PRIVATE_KEY_PASSWORD=%q\n' \
      "$TAURI_SIGNING_PRIVATE_KEY" "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD"
    ;;
  ""|-h|--help)
    sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
    ;;
  *)
    load_env
    exec "$@"
    ;;
esac
