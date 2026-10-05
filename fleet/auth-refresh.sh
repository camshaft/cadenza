#!/usr/bin/env bash
# auth-refresh.sh — keep the host's short-lived auth cookie alive WITHIN a session (operator note-709, approved
# 2026-09-13). Runs a configured refresh command on a cron so the cookie is silently re-minted (no OTP, no
# hardware tap) from a still-valid session, up to the session ceiling — maximizing fleet uptime between the
# interactive re-auths the ceiling still requires. It does NOT extend the session past the ceiling; this cron
# just stops the cookie lapsing MID-session.
#
# The refresh command is host-specific, so it lives in the local, untracked host config, never in this public
# script: set `FLEET_AUTH_REFRESH_CMD` in `${FLEET_HOST_CONF:-$HOME/.config/fleet/host.conf}`. Unset means the
# feature is off: the script records that in its `.last-run` stamp and exits 0. The command must be safe to
# run UNATTENDED + spuriously (a benign no-op when the cookie is already fresh).
#
# Same tracked->runtime split + silent-cron `.last-run` observability as reap-leases.sh / drain-nudge.sh:
# TRACKED at <repo>/fleet/, RUN from the hub copy `fleet up` materializes into <hub>/.claude/fleet/.
set -uo pipefail

CONF="${FLEET_HOST_CONF:-${HOME:-}/.config/fleet/host.conf}"
# shellcheck disable=SC1090
[ -f "$CONF" ] && . "$CONF"

_stamp="$(dirname "${BASH_SOURCE[0]}")/auth-refresh.last-run"
stamp() {
  printf '%s rc=%s %s\n' "$(date -Is 2>/dev/null || echo now)" "$1" "$2" > "$_stamp" 2>/dev/null || true
}

if [ -z "${FLEET_AUTH_REFRESH_CMD:-}" ]; then
  stamp 0 "off: FLEET_AUTH_REFRESH_CMD unset in $CONF"
  exit 0
fi

# SINGLETON GUARD: flock -n so two fires never overlap a refresh run (a later fire skips; the next retries).
# Lock in $HOME (own-user, persists, not the inode-pressured /tmp). FAIL-OPEN if flock is absent.
if command -v flock >/dev/null 2>&1 && exec 9>"${HOME}/.cdz-auth-refresh.lock" 2>/dev/null; then
  flock -n 9 || exit 0
fi

# Best-effort refresh; stdin from /dev/null so a (should-never-happen) prompt can't hang the cron. A nonzero
# rc — e.g. the underlying session has itself lapsed and needs an interactive re-auth — is NOT alarmed here:
# the next fire retries, and a genuinely lapsed session surfaces loudly via the creds-using tools. The command
# is word-split on purpose so the config can carry a binary plus its flags.
# shellcheck disable=SC2086
_out="$($FLEET_AUTH_REFRESH_CMD </dev/null 2>&1)"
_rc=$?

# SILENT-CRON OBSERVABILITY (matches reap-leases.sh / prune-*.sh; concierge convention 2026-08-29): OVERWRITE
# a `.last-run` next to this script — its MTIME is liveness proof the (silent, `>/dev/null`) cron fired, its
# content the last result. Best-effort, never fails the run.
stamp "$_rc" "$(printf '%s' "$_out" | tail -1)"

exit 0
