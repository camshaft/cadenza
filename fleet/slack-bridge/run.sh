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
#   WATCHDOG_ONLY        (rust impl) =1 → run JUST the fleet watchdog, NO Socket Mode — a safe way to
#                        run the reliability backbone as a 2nd process without a competing Slack conn.
#   RUN_ONCE             =1 → exec the bridge ONCE and do NOT self-restart (let the supervisor — e.g. a
#                        systemd unit with Restart=always — own the restart loop). Default: internal loop.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The default FLEET_DIR must be the SHARED HUB state dir. `.claude/` exists ONLY at the main checkout
# and is shared across worktrees via the common git dir, so a plain "$here/../.." (the WORKTREE root
# when run.sh is launched from a worktree) points at a nonexistent worktree-relative .claude/fleet —
# a silent empty-inbox failure. Derive the hub from the common git dir (…/cadenza/.git → …/cadenza),
# same as install.sh + revive.sh. An explicit FLEET_DIR env still wins (set below).
common_git="$(cd "$here" && git rev-parse --git-common-dir 2>/dev/null || true)"
if [[ -n "$common_git" ]]; then
  repo_root="$(cd "$(dirname "$common_git")" && pwd)"
else
  repo_root="$(cd "$here/../.." && pwd)"
fi

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

# RUN_ONCE=1 → the supervisor (systemd Restart=always) owns restarts; exec once and exit with the
# bridge's own status so the supervisor sees the real exit code. Otherwise run our own restart loop.
if [[ "${RUN_ONCE:-}" == "1" ]]; then
  start
  exit $?
fi

while true; do
  start
  code=$?
  echo "run.sh: bridge exited ($code); restarting in ${delay}s" >&2
  sleep "$delay"
done
