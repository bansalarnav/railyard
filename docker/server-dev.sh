#!/usr/bin/env bash
set -euo pipefail

mkdir -p \
  /workspace/.dev-state/cargo-home \
  /workspace/.dev-state/target \
  /workspace/.dev-state/server \
  /run/railyard

if [[ ! -f /workspace/.dev-state/client/servers/dev.json ]]; then
  cargo run --quiet -p railyard-server -- up --foreground &
  bootstrap_server_pid=$!

  cleanup_bootstrap() {
    kill "$bootstrap_server_pid" 2>/dev/null || true
  }
  trap cleanup_bootstrap EXIT INT TERM

  health_attempts=0
  until curl --fail --silent http://127.0.0.1:3001/healthz >/dev/null; do
    if ! kill -0 "$bootstrap_server_pid" 2>/dev/null; then
      wait "$bootstrap_server_pid"
      exit $?
    fi
    health_attempts=$((health_attempts + 1))
    if ((health_attempts >= 600)); then
      echo "Dev server did not become healthy within 120 seconds." >&2
      exit 1
    fi
    sleep 0.2
  done

  # A missing client profile may be a completely fresh checkout or just a
  # deleted local key. Recreate only the disposable dev admin in either case.
  cargo run --quiet -p railyard-server -- user remove dev >/dev/null
  invite_output="$(cargo run --quiet -p railyard-server -- user add dev)"
  invite_blob="$(printf '%s\n' "$invite_output" | grep '^ryd-invite-v1\.' | head -n 1)"

  if [[ -z "$invite_blob" ]]; then
    printf '%s\n' "$invite_output"
    echo "Could not find the dev invite in railyard-server output." >&2
    exit 1
  fi

  RAILYARD_DEV=1 cargo run --quiet -p railyard -- login "$invite_blob" --name dev
  echo "Created the local dev admin. Use ./dev-cli to run authenticated CLI commands."

  kill "$bootstrap_server_pid"
  wait "$bootstrap_server_pid" || true
  trap - EXIT INT TERM
else
  echo "Reusing the local dev admin from .dev-state/client."
fi

exec cargo watch \
  --poll \
  -w apps/server \
  -w packages \
  -w Cargo.toml \
  -w Cargo.lock \
  -x "run -p railyard-server -- up --foreground"
