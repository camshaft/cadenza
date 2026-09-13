#!/usr/bin/env bash
# aea-refresh.sh — keep the Midway AEA posture cookie alive WITHIN a session (operator note-709, approved
# 2026-09-13). Runs `mwinit --refresh-aea` on a cron so the ~2h AEA cookie is silently re-minted (no OTP,
# no hardware tap) from a still-valid session, up to the ~12-20h session ceiling — maximizing fleet uptime
# between the interactive re-auths the ceiling still requires. It does NOT extend the session past the
# ceiling (that needs an interactive `mwinit -o` / hardware tap; the long-term A5 OBO LLRT fix is a separate
# track) — this cron just stops the AEA cookie lapsing MID-session.
#
# `--refresh-aea` re-mints the AEA cookie from the EXISTING valid Midway session, so it is safe to run
# UNATTENDED + spuriously (a benign no-op refresh when the cookie is already fresh — verified non-interactive:
# "Refreshing AEA cookie in Midway cookie file", exit 0, cookie mtime advances).
#
# Same tracked->runtime split + silent-cron `.last-run` observability as reap-leases.sh / drain-nudge.sh:
# TRACKED at <repo>/fleet/, RUN from the hub copy `fleet up` materializes into <hub>/.claude/fleet/.
set -uo pipefail

# SINGLETON GUARD: flock -n so two fires never overlap an mwinit run (a later fire skips; the next retries).
# Lock in $HOME (own-user, persists, not the inode-pressured /tmp). FAIL-OPEN if flock is absent.
if command -v flock >/dev/null 2>&1 && exec 9>"${HOME}/.cdz-aea-refresh.lock" 2>/dev/null; then
  flock -n 9 || exit 0
fi

# Best-effort refresh; stdin from /dev/null so a (should-never-happen) prompt can't hang the cron. A nonzero
# rc — e.g. the underlying session has itself lapsed and needs an interactive re-auth — is NOT alarmed here:
# the next fire retries, and a genuinely lapsed session surfaces loudly via the creds-using tools. Capture
# the last output line for the .last-run stamp.
MWINIT="$(command -v mwinit || echo /usr/bin/mwinit)"
_out="$("$MWINIT" --refresh-aea </dev/null 2>&1)"
_rc=$?

# SILENT-CRON OBSERVABILITY (matches reap-leases.sh / prune-*.sh; concierge convention 2026-08-29): OVERWRITE
# a `.last-run` next to this script — its MTIME is liveness proof the (silent, `>/dev/null`) cron fired, its
# content the last result. Best-effort, never fails the run.
_stamp="$(dirname "${BASH_SOURCE[0]}")/aea-refresh.last-run"
printf '%s rc=%s %s\n' \
  "$(date -Is 2>/dev/null || echo now)" "$_rc" "$(printf '%s' "$_out" | tail -1)" \
  > "$_stamp" 2>/dev/null || true

exit 0
