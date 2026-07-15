#!/usr/bin/env bash
# Launch the Cadenza fleet↔Slack bridge as a persistent, auto-restarting process.
#
# This is the reproducible launcher for the LIVE bridge (currently the Node implementation; it will point
# at the Rust `cadenza-slack-bridge` binary once that reaches live parity — see DESIGN.md). Run it from a
# durable host (a tmux window, a systemd unit, pm2, …) so the bridge survives crashes and restarts.
#
# It reads Slack credentials from the operator's out-of-repo env file (default ~/.cadenza-env) — tokens are
# NEVER committed. Config via environment (all optional; sane defaults):
#   CADENZA_ENV_FILE     dotenv file with SLACK_BOT_TOKEN + SLACK_APP_TOKEN   (default: ~/.cadenza-env)
#   FLEET_DIR            the shared fleet state dir holding inbox/            (default: <repo>/.claude/fleet)
#   SLACK_BRIDGE_CHANNEL Slack channel/DM id the bridge posts into           (default: unset → DM-only inbound)
#   BRIDGE_IMPL          "node" (bridge.js) or "rust" (cargo run --release)  (default: node)
#   RESTART_DELAY        seconds to wait before restarting after a crash     (default: 5)
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"

env_file="${CADENZA_ENV_FILE:-$HOME/.cadenza-env}"
if [[ -f "$env_file" ]]; then
  # shellcheck disable=SC1090
  set -a; . "$env_file"; set +a
else
  echo "run.sh: no env file at $env_file — the bridge will start fail-soft (watchdog-only for Rust; Node needs tokens)." >&2
fi

export FLEET_DIR="${FLEET_DIR:-$repo_root/.claude/fleet}"
impl="${BRIDGE_IMPL:-node}"
delay="${RESTART_DELAY:-5}"

start() {
  case "$impl" in
    node) node "$here/bridge.js" ;;
    rust) ( cd "$here" && cargo run --release --quiet ) ;;
    *) echo "run.sh: unknown BRIDGE_IMPL='$impl' (want node|rust)" >&2; return 2 ;;
  esac
}

echo "run.sh: launching '$impl' bridge; FLEET_DIR=$FLEET_DIR channel=${SLACK_BRIDGE_CHANNEL:-<none>}"
while true; do
  start
  code=$?
  echo "run.sh: bridge exited ($code); restarting in ${delay}s" >&2
  sleep "$delay"
done
