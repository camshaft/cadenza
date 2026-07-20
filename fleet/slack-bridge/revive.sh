#!/usr/bin/env bash
# Bring the two hosted daemons back up WITHOUT depending on systemd — the reliable fallback path.
#
# WHY: the systemd --user units (systemd/) only auto-start on boot if the user's systemd MANAGER is
# running, and on some hosts it isn't (dead session bus after a reboot / OOM under load; restarting it
# needs root/polkit — not doable unattended). This script is the manual backbone the v-slack-bridge
# loop uses to revive comms in that case. It is IDEMPOTENT: a daemon already alive is left untouched.
#
# It hosts, detached (setsid), each daemon's own auto-restart loop:
#   1. the Slack bridge  — via run.sh, which already re-launches the bridge on crash.
#   2. the watchdog-only daemon (WATCHDOG_ONLY=1, no Socket Mode) — wrapped in a restart loop here,
#      since the binary has no tracked launcher of its own.
#
# Env (all optional; sane defaults):
#   CADENZA_ENV_FILE     dotenv file with the Slack tokens              (default: ~/.cadenza-env)
#   SLACK_BRIDGE_CHANNEL channel/DM the bridge posts into               (default: from the env file)
#   LOG_DIR              where the two daemon logs are written          (default: /tmp)
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"          # …/fleet/slack-bridge
bin="$here/target/release/cadenza-slack-bridge"
log_dir="${LOG_DIR:-/tmp}"
env_file="${CADENZA_ENV_FILE:-$HOME/.cadenza-env}"

# FLEET_DIR must be the SHARED HUB state dir (`.claude/` exists only at the main checkout, shared across
# worktrees via the common git dir) — a worktree-relative path points at a nonexistent inbox. Derive the
# hub from the common git dir (…/cadenza/.git → …/cadenza), same as systemd/install.sh.
common_git="$(cd "$here" && git rev-parse --git-common-dir 2>/dev/null || true)"
if [[ -n "$common_git" ]]; then
  hub_root="$(cd "$(dirname "$common_git")" && pwd)"
else
  hub_root="$(cd "$here/../.." && pwd)"
fi
fleet_dir="$hub_root/.claude/fleet"

# Channel: explicit env wins, else pull from the env file (SLACK_BRIDGE_CHANNEL or the SLACK_CHANNEL alias).
channel="${SLACK_BRIDGE_CHANNEL:-}"
if [[ -z "$channel" && -f "$env_file" ]]; then
  channel="$(sed -n 's/^[[:space:]]*SLACK_BRIDGE_CHANNEL=//p; s/^[[:space:]]*SLACK_CHANNEL=//p' "$env_file" | tail -1 | tr -d '"'"'"'')"
fi

echo "revive.sh: hub_fleet_dir=$fleet_dir channel=${channel:-<none>} log_dir=$log_dir"

# --- 1. Slack bridge (run.sh owns its own restart loop) -----------------------------------------
if pgrep -f "[b]ridge.js" >/dev/null 2>&1; then
  echo "revive.sh: bridge already up ($(pgrep -f '[b]ridge.js' | tr '\n' ' '))— leaving it."
else
  echo "revive.sh: bridge DOWN — launching run.sh (node, detached)…"
  CADENZA_ENV_FILE="$env_file" FLEET_DIR="$fleet_dir" SLACK_BRIDGE_CHANNEL="$channel" BRIDGE_IMPL=node \
    setsid bash "$here/run.sh" >"$log_dir/slack-bridge.log" 2>&1 </dev/null &
  echo "revive.sh: launched bridge runner (log: $log_dir/slack-bridge.log)"
fi

# --- 2. Watchdog-only daemon (restart loop wrapped here) ----------------------------------------
if pgrep -f "target/release/[c]adenza-slack-bridge" >/dev/null 2>&1; then
  echo "revive.sh: watchdog daemon already up ($(pgrep -f 'target/release/[c]adenza-slack-bridge' | tr '\n' ' '))— leaving it."
elif [[ ! -x "$bin" ]]; then
  echo "revive.sh: watchdog binary missing ($bin) — build it: (cd $here && cargo build --release). Skipping watchdog." >&2
else
  echo "revive.sh: watchdog daemon DOWN — launching WATCHDOG_ONLY restart loop (detached)…"
  CADENZA_ENV_FILE="$env_file" FLEET_DIR="$fleet_dir" WATCHDOG_ONLY=1 BIN="$bin" \
    setsid bash -c '
      set -a; [ -f "$CADENZA_ENV_FILE" ] && . "$CADENZA_ENV_FILE"; set +a
      while true; do "$BIN"; echo "watchdog exited ($?); restarting in 5s" >&2; sleep 5; done
    ' >"$log_dir/watchdog-daemon.log" 2>&1 </dev/null &
  echo "revive.sh: launched watchdog runner (log: $log_dir/watchdog-daemon.log)"
fi

# --- verify ------------------------------------------------------------------------------------
# BOTH daemons must be verified — a bridge-only check reports apparent success when the watchdog
# silently fails to come up (bad tokens, crash loop). Exit non-zero if EITHER is down so a caller /
# cron can detect an incomplete revival instead of trusting a partial one.
sleep 6
echo "revive.sh: post-launch liveness —"
echo "  bridge:   $(pgrep -f '[b]ridge.js' | tr '\n' ' ' || true)"
echo "  watchdog: $(pgrep -f 'target/release/[c]adenza-slack-bridge' | tr '\n' ' ' || true)"
rc=0
if pgrep -f "[b]ridge.js" >/dev/null 2>&1; then
  echo "revive.sh: bridge UP ✓"
else
  echo "revive.sh: bridge STILL DOWN ✗ — check $log_dir/slack-bridge.log (tokens? node on PATH?)" >&2
  rc=1
fi
if pgrep -f "target/release/[c]adenza-slack-bridge" >/dev/null 2>&1; then
  echo "revive.sh: watchdog UP ✓"
elif [[ ! -x "$bin" ]]; then
  echo "revive.sh: watchdog SKIPPED — binary missing ($bin); build it: (cd $here && cargo build --release)." >&2
  rc=1
else
  echo "revive.sh: watchdog STILL DOWN ✗ — check $log_dir/watchdog-daemon.log (tokens? bin crash?)" >&2
  rc=1
fi
exit $rc
