#!/usr/bin/env bash
# Install the Cadenza fleet systemd --user units so the Slack bridge + watchdog daemon auto-start on
# boot and survive crashes (Restart=always) and logout (loginctl enable-linger). This closes the
# reboot comms-gap that previously required a manual/agent relaunch of the two /tmp runner loops.
#
# Idempotent: re-run any time (e.g. after moving the repo or changing the channel). It substitutes the
# machine-specific paths into the tracked *.service.in templates, writes them under
# ~/.config/systemd/user/, then enables + (re)starts both units.
#
# Env (all optional):
#   SLACK_BRIDGE_CHANNEL  Slack channel/DM id the bridge posts into (default: read from ~/.cadenza-env,
#                         else empty → DM-only inbound).
#   NO_START=1            install + enable the units but do NOT start them now (just arm for next boot).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"          # …/fleet/slack-bridge/systemd
repo_root="$(cd "$here/../../.." && pwd)"                      # this checkout's root (may be a WORKTREE)
unit_dir="$HOME/.config/systemd/user"

# The runtime fleet state (inbox/) lives at the HUB's `.claude/fleet`, NOT this checkout's — `.claude/`
# is gitignored and exists only at the main checkout, shared across worktrees via the common git dir. So
# FLEET_DIR must be the HUB path even when install.sh runs from a worktree. Derive the hub from the
# common git dir (…/cadenza/.git → …/cadenza); fall back to repo_root if it's already the main checkout.
common_git="$(cd "$repo_root" && git rev-parse --git-common-dir 2>/dev/null || true)"
if [[ -n "$common_git" ]]; then
  hub_root="$(cd "$(dirname "$common_git")" && pwd)"          # parent of the shared .git
else
  hub_root="$repo_root"
fi
fleet_dir="$hub_root/.claude/fleet"
if [[ ! -d "$fleet_dir" ]]; then
  echo "install.sh: WARNING: fleet dir $fleet_dir does not exist — is $hub_root the main checkout? Continuing." >&2
fi

# Resolve the channel: explicit env wins, else pull SLACK_BRIDGE_CHANNEL / SLACK_CHANNEL from the env file.
env_file="${CADENZA_ENV_FILE:-$HOME/.cadenza-env}"
channel="${SLACK_BRIDGE_CHANNEL:-}"
if [[ -z "$channel" && -f "$env_file" ]]; then
  channel="$(sed -n 's/^[[:space:]]*SLACK_BRIDGE_CHANNEL=//p; s/^[[:space:]]*SLACK_CHANNEL=//p' "$env_file" | tail -1 | tr -d '"'"'"'')"
fi

# Capture the current PATH so `node`/`cargo` resolve under systemd's otherwise-minimal environment.
current_path="$PATH"

echo "install.sh: repo_root=$repo_root fleet_dir=$fleet_dir channel=${channel:-<none>} unit_dir=$unit_dir"
mkdir -p "$unit_dir"

# Ensure the watchdog binary exists (the unit ExecStart points straight at it — no cargo-run wrapper).
if [[ ! -x "$repo_root/fleet/slack-bridge/target/release/cadenza-slack-bridge" ]]; then
  echo "install.sh: building the watchdog binary (release) …"
  ( cd "$repo_root/fleet/slack-bridge" && cargo build --release )
fi

subst() {  # subst <template> <dest>
  sed -e "s#@REPO_ROOT@#$repo_root#g" \
      -e "s#@FLEET_DIR@#$fleet_dir#g" \
      -e "s#@HOME@#$HOME#g" \
      -e "s#@PATH@#$current_path#g" \
      -e "s#@CHANNEL@#$channel#g" \
      "$1" > "$2"
  echo "  wrote $2"
}

subst "$here/cadenza-slack-bridge.service.in" "$unit_dir/cadenza-slack-bridge.service"
subst "$here/cadenza-watchdog.service.in"     "$unit_dir/cadenza-watchdog.service"

# Survive logout / start on boot without an interactive login session.
loginctl enable-linger "$USER" 2>/dev/null || echo "install.sh: enable-linger failed (may need sudo); continuing."

systemctl --user daemon-reload
systemctl --user enable cadenza-slack-bridge.service cadenza-watchdog.service

if [[ "${NO_START:-}" == "1" ]]; then
  echo "install.sh: NO_START=1 — units enabled for next boot but not started now."
else
  systemctl --user restart cadenza-slack-bridge.service cadenza-watchdog.service
  echo "install.sh: started both units. Status:"
  systemctl --user --no-pager status cadenza-slack-bridge.service cadenza-watchdog.service | sed -n '1,6p' || true
fi

echo "install.sh: done. Manage with: systemctl --user {status,restart,stop} cadenza-slack-bridge cadenza-watchdog"
echo "            logs: journalctl --user -u cadenza-slack-bridge -f"
