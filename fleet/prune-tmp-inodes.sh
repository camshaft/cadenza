#!/usr/bin/env bash
# prune-tmp-inodes.sh — reclaim /tmp INODES from stale ephemera that accumulate without cleanup.
#
# WHY: /tmp is a tmpfs with a FIXED inode budget (~1M here) independent of its byte capacity. Tiny-but-
# numerous files exhaust its inodes at low BYTE usage (seen: 100% inodes / 16% bytes), after which every
# agent's Bash fails ENOSPC (it cannot write its output file) and the fleet wedges. Four classes:
#   A. TOOLBOX TELEMETRY (the PRIMARY accumulator, operator-confirmed): `/tmp/toolbox-telemetry-*` dirs
#      created ~2/min by the internal toolbox EMF wrapper, each holding a few log/metric files, with NO
#      cleanup — hundreds pile up per hour.
#   B. CLAUDE TASK TRANSCRIPTS: `*.output`/`*.jsonl` under `/tmp/claude-<pid>/<session>/…` plus the
#      per-command `/tmp/claude-*-cwd` capture files, across ~20 active agents.
#   C. AGENT SCRATCH DIRS (concierge trend 2026-08-28, 19%→33%/session; broadened after the 2026-09-22
#      fleet-wide 100%-inode wedge): allowlisted agent scratch dirs (`/tmp/mphome`, `shredall`, `shred-*`,
#      `otc`, `vrb*`, `latentleak-*`, `cdz-*-smoke*`, `node-compile-cache`, plus the corpus grade/shred/
#      roundtrip families `*shred*`, `*-grade`, `vg-*`, `rd-probe*`, `wo-*`, `th_*`, `th[0-9]*`, `tb_*`) that
#      A/B/D don't cover. These are IN-USE probe/grade scratch, so Class C is the most conservative: a
#      SEPARATE higher threshold (dormant in normal operation, fires only near the wedge), a long age floor,
#      a fail-safe liveness check, an allowlist (never a blanket /tmp/* sweep — a blanket age-sweep would
#      falsely target epoch-mtimed nix `*-result` GC roots), and own-user only. `prune-stale-targets.sh`
#      reclaims worktree `target/` on /local — a distinct class.
#   D. ORACLE DIFFERENTIAL RUN DIRS (concierge root-cause 2026-08-29, the DOMINANT inode hog behind a
#      near-ENOSPC wedge): `/tmp/oracle-all*`, `oall*`, `surv*` — v-lean-oracle's full-corpus oracle
#      differential run dirs, each a whole corpus tree ≈ 47K INODES, leaked (not cleaned after each run) →
#      ~11 accumulated over ~7h ≈ 522K inodes = the bulk of a 977K-inode wedge (the small classes above are
#      ~3-5 inodes/dir and were NOT the growth). Reaped like Class C (own-user + lsof-idle + age) but with
#      its OWN shorter age floor (ORACLE_STALE_MIN, 2h — a couple leaked 47K-inode dirs already threaten the
#      wall, and the owner confirmed >2h dirs are completed/safe) + the `*-out.txt`/`*-manifest.txt` sibling
#      files. The owner (v-lean-oracle) is routed to stop leaking (clean each run dir on completion); this
#      reaper is the safety net.
#   E. MCS L1 TELEMETRY LOGS (operator note-700, 2026-09-13): `/tmp/mcs-telemetry-l1-*.log` — the MCS
#      `L1_Linux` client (v2.7.4) writes ONE fire-and-forget JSON log FILE per `open_connection` event with
#      no cleanup; the fleet's many agents connect constantly (~35 files/min observed) so these pile up and
#      pressure /tmp inodes exactly like Class A. Reaped like A (own-user FILES, short age floor). IMPORTANT:
#      a SEPARATE 37G `/tmp/mcs-telemetry/` DIRECTORY owned by user `mcs` needs root/sudo to remove — it is
#      NOT touched here (the `-uid $(id -u)` filter can only ever match the fleet user's own log files), and
#      is reported to the operator. Stopping the spray at its source is an upstream MCS-client fix.
#
# SAFETY (per-class gates + guards):
#   1. THRESHOLD-GATED, PER CLASS: A/B sweep only when /tmp inode-use% >= INODE_THRESHOLD_PCT (default
#      80; the maintenance cron runs it at 0 = unconditional). Class C has its OWN, INDEPENDENT gate
#      SCRATCH_THRESHOLD_PCT (default 70) so scratch is reaped ONLY near the wedge even when A/B run
#      unconditionally — dormant during normal operation (zero risk of nuking live scratch). Class D
#      (oracle) has its OWN gate ORACLE_THRESHOLD_PCT (default 60, LOWER than scratch) — it's the dominant
#      inode hog + pure-leak + lsof-protected, so it's reaped earlier, well before the 90% ENOSPC wedge.
#   2. AGE-GUARDED: removes only entries older than a per-class age — a live buffer/transcript/scratch has
#      a recent mtime. TELEMETRY_STALE_MIN (15), STALE_MIN (120), SCRATCH_STALE_MIN (240 = 4h) are knobs.
#      NOTE: telemetry is the PRIMARY accumulator and the fleet generates toolbox-telemetry-* faster than a
#      30min window cleared them net (observed monotonic /tmp inode creep 34%→46% over ~2h), so the window
#      is 15min: standing backlog ≈ generation_rate × window, and a buffer idle 15min is flushed (EMF
#      buffers flush in seconds), so 15min carries no live-buffer risk while ~halving the standing count.
#      A `/tmp/claude-<pid>/` dir is SHARED across sessions (not tied to one agent), so AGE is the signal.
#   3. LIVENESS (Classes C + D): each candidate dir is skipped unless `lsof +D` shows NO live user (open fd
#      or cwd anywhere under it). FAIL-SAFE — missing lsof / any lsof output (users OR an error) → KEEP
#      the dir. Only a clean, empty lsof permits removal, so an active probe/oracle-run dir is never reaped.
#   4. SCOPE: A/B touch only those two ephemeral classes (Claude excludes `journal.jsonl`, the Workflow
#      RESUME journal). Class C touches only allowlisted, own-user dirs; Class D only the oracle-run dir
#      shapes (own-user). Nothing else in /tmp is touched.
#
# DRY-RUN by default (prints WOULD-REMOVE counts). Pass --apply to actually delete.
# Meant to be run periodically (e.g. a maintenance cron) from the materialized hub copy.
set -euo pipefail

TMPDIR_ROOT="${TMPDIR_ROOT:-/tmp}"
INODE_THRESHOLD_PCT="${INODE_THRESHOLD_PCT:-80}"   # A/B sweep only when /tmp inode-use% is at/above this
TELEMETRY_STALE_MIN="${TELEMETRY_STALE_MIN:-15}"   # remove toolbox-telemetry-* older than this (minutes; primary accumulator, kept short so the always-on sweep clears more per pass)
MCS_TELEMETRY_STALE_MIN="${MCS_TELEMETRY_STALE_MIN:-15}"  # Class E: remove own-user /tmp/mcs-telemetry-l1-*.log older than this (minutes; fire-and-forget MCS L1 logs, flushed instantly so 15min carries no live risk)
STALE_MIN="${STALE_MIN:-120}"                      # remove claude task transcripts older than this (minutes)
SCRATCH_THRESHOLD_PCT="${SCRATCH_THRESHOLD_PCT:-70}" # Class C fires ONLY at/above this — INDEPENDENT of INODE_THRESHOLD_PCT
SCRATCH_STALE_MIN="${SCRATCH_STALE_MIN:-240}"      # remove agent-scratch dirs older than this (minutes, default 4h)
ORACLE_STALE_MIN="${ORACLE_STALE_MIN:-120}"        # Class D: remove oracle-run dirs older than this (minutes, default 2h; each ≈47K inodes so shorter than scratch)
ORACLE_THRESHOLD_PCT="${ORACLE_THRESHOLD_PCT:-60}" # Class D fires at/above this — LOWER than scratch (70): oracle dirs are the dominant hog + pure-leak + lsof-protected, so reap the hog earlier (well before the 90% wedge)
# (The old Class-F REPORT upper-age WINDOW bound was removed 2026-09-26: the operator's age-only GC uses a
# pure lower FLOOR, and nix `*-result` GC-roots are now excluded by REAP_EXEMPT_PATTERNS + `-type d`, not by
# an upper window bound.)
# Trend-log threshold (percent): APPEND one trend line to prune-tmp-inodes.trend ONLY when inode-use is at/
# above this (default 85 = WARN). The `.last-run` stamp is OVERWRITE (latest only), so a climb/reversal
# trajectory (e.g. self-clean pulling 6748→2876 over ~6h) is invisible in it — this append log makes the
# per-run trajectory greppable, but ONLY during a pressure episode (silent below WARN) so it stays bounded
# without rotation, exactly like reap-leases.log logs only on a nonzero reap.
TREND_LOG_PCT="${TREND_LOG_PCT:-85}"
# Class F (age-only /tmp GC) reaps own-user dirs older than this many minutes — operator 2026-09-26: "if it's
# older than an hour or two it's free game to get nuked in tmp." Default 120 (2h, the conservative end of the
# operator's 1–2h). Unlike A–E, Class F is NOT class-gated on a pressure threshold: it shares A/B's
# INODE_THRESHOLD_PCT gate (the cron runs at 0 = ALWAYS), so it is an automatic GC that keeps /tmp perpetually
# clear of idle old scratch — no per-agent `rm -rf` (which would hit the harness approval prompt the operator
# wants avoided). The load-bearing safety is the REAP_EXEMPT_PATTERNS (nix/claude) + the lsof-idle guard.
TMP_REAP_MIN="${TMP_REAP_MIN:-120}"

# Class C allowlist — ONLY these known agent-scratch dir SHAPES are ever candidates (never a blanket sweep).
# The grade/shred/roundtrip families below were added after a fleet-wide 100%-inode wedge (breaker issue
# 083035 + v-cadenza-backend fyi 083033, 2026-09-22): allshred ~56k, vmem-grade ~21k, rd-probe ~19k,
# wo-lean432 ~15k, vg-*/vgi-shred, th_*/tb_* roundtrip, oshred — cross-agent corpus grade/shred scratch that
# A/B/D didn't cover and the original allowlist missed, so it accumulated unreaped to the wall. sw_*/rx_*
# (per-chapter roundtrip families, same shape as th_*/tb_*) added 2026-09-22 after a board scan found sw_*
# alone at ~32 dirs × ~8.8k ≈ 280K inodes un-reaped. A blanket
# /tmp/* age-sweep stays REFUSED: breaker found many nix `*-result` dirs whose mtime nix normalizes to ~epoch,
# so they read as infinitely-old and a mtime age-floor would falsely target them (and they can be live GC
# roots) — none of these families match `*-result`, and the lsof-idle guard protects any live one regardless.
SCRATCH_PATTERNS=(mphome shredall 'shred-*' otc 'vrb*' 'latentleak-*' 'cdz-*-smoke*' 'node-compile-cache' \
                  '*shred*' '*-grade' 'vg-*' 'rd-probe*' 'wo-*' 'th_*' 'th[0-9]*' 'tb_*' 'sw_*' 'rx_*')
# Class D allowlist — ONLY these oracle differential run-dir SHAPES (v-lean-oracle full-corpus runs).
ORACLE_PATTERNS=('oracle-all*' 'oall*' 'surv*')

# Class F reap-EXEMPT list — the MINIMAL set the operator's age-only sweep must NOT touch (operator 2026-09-26:
# "I don't even think we need patterns in there. If it's older than an hour or two it's free game to get
# nuked in tmp" — so the old scratch/foreign ALLOWLIST is gone; everything else >TMP_REAP_MIN idle is fair
# game). Only CORRECTNESS/LIVENESS exemptions remain — NOT the "which scratch is ours" allowlist:
#   - nix bookkeeping: `*-result` / `result` (GC-root out-links — deleting one UNPINS a build's store output;
#     the SYMLINK forms are also excluded by the sweep's `-type d`) + `nix-shell.*`/`nix-develop-*`/
#     `nix-build-*` LIVE-session dirs (a running build's dir can be >2h old yet held). This is the one caveat
#     concierge flagged: a pure mtime floor would false-target these (nix normalizes GC-root mtimes to ~epoch).
#   - `claude-*`: LIVE-AGENT session roots (`/tmp/claude-<pid>/`, journal.jsonl + transcripts). NOT scratch —
#     rm -rf'ing one breaks a running agent; Class B already manages these (stale-transcript reap that
#     PRESERVES journal.jsonl), so Class F must leave the whole tree to B.
# The three named NON-FLEET TOOLS below (a2a-client / MembrainDev* / S3TurboCacheModel*) are a CONSERVATIVE
# default, NOT the old scratch allowlist: they are the user's other tools' /tmp state, deleting them is
# irreversible + outward-facing, /tmp is not under pressure, and the operator's explicit "pure-everything vs
# skip-just-nix" confirmation is still pending (concierge is relaying). Sparing just these three named tools
# until that confirmation is a 3-line exemption (negligible inodes) — drop this line the moment the operator
# says "pure everything." EVERYTHING ELSE >TMP_REAP_MIN (fleet scratch, `tmp.*` mktemp leftovers, unknown idle
# dirs) is free game per the ruling; the lsof-idle guard still spares anything with an open fd/cwd.
REAP_EXEMPT_PATTERNS=('*-result' 'result' 'nix-shell.*' 'nix-develop-*' 'nix-build-*' 'claude-*' \
                      'a2a-client' 'MembrainDev*' 'S3TurboCacheModel*')

iuse_pct() { df -i "$TMPDIR_ROOT" | awk 'NR==2 {gsub(/%/,"",$5); print $5}'; }

# True (0) iff a scratch dir has NO live user. FAIL-SAFE: no lsof, or ANY lsof output (a user row OR an
# error message), means "treat as in-use" → return non-zero → the caller KEEPS the dir. Only a clean,
# empty lsof (no fd/cwd anywhere under the dir) returns 0 = removable.
scratch_dir_is_idle() {
  local d="$1" out
  command -v lsof >/dev/null 2>&1 || return 1
  out="$(lsof +D "$d" 2>&1)"
  [ -z "$out" ]
}

APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

iuse="$(iuse_pct)"
iuse="${iuse:-0}"
printf 'prune-tmp-inodes: %s inode-use=%s%% ab-threshold=%s%% scratch-threshold=%s%% telemetry-stale=%smin claude-stale=%smin scratch-stale=%smin oracle-stale=%smin apply=%s\n' \
  "$TMPDIR_ROOT" "$iuse" "$INODE_THRESHOLD_PCT" "$SCRATCH_THRESHOLD_PCT" "$TELEMETRY_STALE_MIN" "$STALE_MIN" "$SCRATCH_STALE_MIN" "$ORACLE_STALE_MIN" "$APPLY"

# ── Classes A + B: gated on INODE_THRESHOLD_PCT (the cron runs this at 0 = unconditional). ────────────
if [ "$iuse" -ge "$INODE_THRESHOLD_PCT" ]; then
  # Class A: toolbox EMF telemetry buffers (`/tmp/toolbox-telemetry-*`, whole dirs).
  telemetry="$(find "$TMPDIR_ROOT" -maxdepth 1 -name 'toolbox-telemetry-*' -mmin +"$TELEMETRY_STALE_MIN" 2>/dev/null | wc -l)"

  # Class E: MCS L1 telemetry logs — own-user FILES only (`-uid`), so the mcs-owned dir is never a candidate.
  mcs_logs="$(find "$TMPDIR_ROOT" -maxdepth 1 -type f -uid "$(id -u)" -name 'mcs-telemetry-l1-*.log' -mmin +"$MCS_TELEMETRY_STALE_MIN" 2>/dev/null | wc -l)"

  # Class B: Claude task-root dirs (`/tmp/claude-<pid>/`). Enumerate as DIRS so the top-level
  # `claude-*-cwd` FILES are not treated as roots. `-print0`/read handles a (unlikely) space in a path.
  roots=()
  while IFS= read -r -d '' r; do roots+=("$r"); done \
    < <(find "$TMPDIR_ROOT" -maxdepth 1 -type d -name 'claude-*' -print0 2>/dev/null)
  transcripts=0
  if [ "${#roots[@]}" -gt 0 ]; then
    transcripts="$(find "${roots[@]}" -type f \( -name '*.output' -o -name '*.jsonl' \) \
      ! -name 'journal.jsonl' -mmin +"$STALE_MIN" 2>/dev/null | wc -l)"
  fi
  cwds="$(find "$TMPDIR_ROOT" -maxdepth 1 -type f -name 'claude-*-cwd' -mmin +"$STALE_MIN" 2>/dev/null | wc -l)"

  if [ "$APPLY" = 1 ]; then
    # A: remove whole stale telemetry dirs (they are self-contained buffers).
    find "$TMPDIR_ROOT" -maxdepth 1 -name 'toolbox-telemetry-*' -mmin +"$TELEMETRY_STALE_MIN" \
      -exec rm -rf {} + 2>/dev/null || true
    # E: remove own-user MCS L1 telemetry logs (fire-and-forget files; -uid guards against the mcs-owned dir).
    find "$TMPDIR_ROOT" -maxdepth 1 -type f -uid "$(id -u)" -name 'mcs-telemetry-l1-*.log' \
      -mmin +"$MCS_TELEMETRY_STALE_MIN" -delete 2>/dev/null || true
    # B: remove stale claude transcript files, then reclaim their emptied dir trees + the cwd files.
    if [ "${#roots[@]}" -gt 0 ]; then
      find "${roots[@]}" -type f \( -name '*.output' -o -name '*.jsonl' \) \
        ! -name 'journal.jsonl' -mmin +"$STALE_MIN" -delete 2>/dev/null || true
      find "${roots[@]}" -type d -empty -delete 2>/dev/null || true
    fi
    find "$TMPDIR_ROOT" -maxdepth 1 -type f -name 'claude-*-cwd' -mmin +"$STALE_MIN" -delete 2>/dev/null || true
    after="$(iuse_pct)"
    printf 'prune-tmp-inodes: removed %s telemetry dir(s) + %s transcript file(s) + %s cwd file(s) + empty dirs; inode-use now %s%%\n' \
      "$telemetry" "$transcripts" "$cwds" "${after:-?}"
    printf 'prune-tmp-inodes: removed %s MCS L1 telemetry log(s) (>%smin, own-user; the mcs-owned dir needs root)\n' \
      "$mcs_logs" "$MCS_TELEMETRY_STALE_MIN"
  else
    printf 'prune-tmp-inodes: WOULD remove %s telemetry dir(s) (>%smin) + %s transcript file(s) + %s cwd file(s) (>%smin, excl journal.jsonl); rerun with --apply\n' \
      "$telemetry" "$TELEMETRY_STALE_MIN" "$transcripts" "$cwds" "$STALE_MIN"
    printf 'prune-tmp-inodes: WOULD remove %s MCS L1 telemetry log(s) (>%smin, own-user); rerun with --apply\n' \
      "$mcs_logs" "$MCS_TELEMETRY_STALE_MIN"
  fi
else
  printf 'prune-tmp-inodes: inode-use %s%% below A/B threshold %s%% — skipping telemetry/transcript sweep.\n' "$iuse" "$INODE_THRESHOLD_PCT"
fi

# ── Class C: agent-scratch dirs, ARMED at its OWN higher gate SCRATCH_THRESHOLD_PCT (independent of the
# A/B gate), so scratch is reaped ONLY near the wedge — dormant in normal operation even when the cron
# runs A/B unconditionally. Allowlisted + age-guarded + liveness-checked + own-user. ─────────────────
if [ "$iuse" -ge "$SCRATCH_THRESHOLD_PCT" ]; then
  # Build the `-name p1 -o -name p2 …` group from the allowlist.
  name_args=()
  for p in "${SCRATCH_PATTERNS[@]}"; do
    [ "${#name_args[@]}" -gt 0 ] && name_args+=(-o)
    name_args+=(-name "$p")
  done
  scratch_cands=()
  while IFS= read -r -d '' d; do scratch_cands+=("$d"); done \
    < <(find "$TMPDIR_ROOT" -maxdepth 1 -type d -uid "$(id -u)" \
          \( "${name_args[@]}" \) -mmin +"$SCRATCH_STALE_MIN" -print0 2>/dev/null)
  scratch_idle=0
  scratch_live=0
  if [ "${#scratch_cands[@]}" -gt 0 ]; then
    for d in "${scratch_cands[@]}"; do
      if scratch_dir_is_idle "$d"; then
        [ "$APPLY" = 1 ] && rm -rf "$d" 2>/dev/null || true
        scratch_idle=$((scratch_idle + 1))
      else
        scratch_live=$((scratch_live + 1))
      fi
    done
  fi
  verb="WOULD remove"
  [ "$APPLY" = 1 ] && verb="removed"
  printf 'prune-tmp-inodes: scratch (>=%s%%): %s %s idle allowlisted dir(s), KEPT %s live/held (of %s candidate(s), age>%smin, own-user)\n' \
    "$SCRATCH_THRESHOLD_PCT" "$verb" "$scratch_idle" "$scratch_live" "${#scratch_cands[@]}" "$SCRATCH_STALE_MIN"
else
  printf 'prune-tmp-inodes: scratch class DORMANT — inode-use %s%% below scratch threshold %s%% (fires only near the wedge).\n' "$iuse" "$SCRATCH_THRESHOLD_PCT"
fi

# ── Class D: oracle differential run dirs — the DOMINANT inode hog (≈47K each). Armed at its OWN
# ORACLE_THRESHOLD_PCT (LOWER than scratch — reap the hog early), reaped own-user + lsof-idle + age (its OWN
# shorter ORACLE_STALE_MIN floor), plus the small `*-out.txt`/`*-manifest.txt` sibling files. ─────────
if [ "$iuse" -ge "$ORACLE_THRESHOLD_PCT" ]; then
  # Build the `-name p1 -o -name p2 …` group from the oracle allowlist.
  oracle_name_args=()
  for p in "${ORACLE_PATTERNS[@]}"; do
    [ "${#oracle_name_args[@]}" -gt 0 ] && oracle_name_args+=(-o)
    oracle_name_args+=(-name "$p")
  done
  # Candidate DIRS (each ≈47K inodes), own-user, aged.
  oracle_cands=()
  while IFS= read -r -d '' d; do oracle_cands+=("$d"); done \
    < <(find "$TMPDIR_ROOT" -maxdepth 1 -type d -uid "$(id -u)" \
          \( "${oracle_name_args[@]}" \) -mmin +"$ORACLE_STALE_MIN" -print0 2>/dev/null)
  oracle_idle=0
  oracle_live=0
  if [ "${#oracle_cands[@]}" -gt 0 ]; then
    for d in "${oracle_cands[@]}"; do
      if scratch_dir_is_idle "$d"; then
        [ "$APPLY" = 1 ] && rm -rf "$d" 2>/dev/null || true
        oracle_idle=$((oracle_idle + 1))
      else
        oracle_live=$((oracle_live + 1))
      fi
    done
  fi
  # Sibling artifact FILES (`oracle-all*-out.txt`, `oall*-manifest.txt`, …): tiny, no liveness needed — the
  # allowlist patterns already match them as top-level files; age-guard + own-user is enough.
  oracle_files="$(find "$TMPDIR_ROOT" -maxdepth 1 -type f -uid "$(id -u)" \
    \( "${oracle_name_args[@]}" \) -mmin +"$ORACLE_STALE_MIN" 2>/dev/null | wc -l)"
  if [ "$APPLY" = 1 ]; then
    find "$TMPDIR_ROOT" -maxdepth 1 -type f -uid "$(id -u)" \
      \( "${oracle_name_args[@]}" \) -mmin +"$ORACLE_STALE_MIN" -delete 2>/dev/null || true
  fi
  verb="WOULD remove"
  [ "$APPLY" = 1 ] && verb="removed"
  printf 'prune-tmp-inodes: oracle (>=%s%%): %s %s idle oracle-run dir(s) + %s sibling file(s), KEPT %s live/held (of %s dir candidate(s), age>%smin, own-user)\n' \
    "$ORACLE_THRESHOLD_PCT" "$verb" "$oracle_idle" "$oracle_files" "$oracle_live" "${#oracle_cands[@]}" "$ORACLE_STALE_MIN"
else
  printf 'prune-tmp-inodes: oracle class DORMANT — inode-use %s%% below oracle threshold %s%% (fires before the wedge).\n' "$iuse" "$ORACLE_THRESHOLD_PCT"
fi

# ── Class F: AGE-ONLY /tmp GC (operator-authorized 2026-09-26: "if it's older than an hour or two it's free
#    game to get nuked in tmp … I'd rather it just automatic" — no per-agent `rm -rf` → no harness approval
#    prompt). This SUPERSEDES the old pattern-allowlisted Class F: there is NO scratch/foreign allowlist and
#    NO keep-list of "which scratch is ours" — just AGE plus the minimal nix/claude correctness exemption.
#    Reaps EVERY own-user /tmp DIR older than TMP_REAP_MIN that is neither REAP_EXEMPT nor lsof-held. Scoped
#    to `-type d` (the inode pressure is scratch dir-trees; this also excludes nix `result` SYMLINKS). It is
#    AUTOMATIC: shares A/B's INODE_THRESHOLD_PCT gate (the maintenance cron runs at 0 = ALWAYS), so it is a
#    background GC keeping /tmp perpetually clear of idle old dirs. Load-bearing safety = REAP_EXEMPT_PATTERNS
#    (nix GC-roots/live-sessions + claude live-agent roots) + the bulk lsof-idle guard. The candidate COUNT
#    (`uncovered`) is stamped/trended regardless of the gate so the volume stays visible on the board.
reap_excl=()
for p in "${REAP_EXEMPT_PATTERNS[@]}"; do
  [ "${#reap_excl[@]}" -gt 0 ] && reap_excl+=(-o)
  reap_excl+=(-name "$p")
done
uncov_cands=()
while IFS= read -r -d '' d; do uncov_cands+=("$d"); done \
  < <(find "$TMPDIR_ROOT" -maxdepth 1 -mindepth 1 -type d -uid "$(id -u)" \
        -mmin +"$TMP_REAP_MIN" \
        -not \( "${reap_excl[@]}" \) -print0 2>/dev/null)
uncovered="${#uncov_cands[@]}"
if [ "$iuse" -ge "$INODE_THRESHOLD_PCT" ]; then
  # LIVENESS via ONE bulk `lsof` pass, NOT `lsof +D` per-dir: `lsof +D <dir>` recursively descends each dir
  # (O(files)) and a per-candidate loop spawns lsof once per dir — over a large backlog that is minutes-long
  # and would pile up (this cron has no flock singleton). Instead enumerate all open files ONCE (`lsof -F n`
  # reads /proc fd/cwd tables — fast, no dir descent), collect the TOP-LEVEL /tmp names holding an open
  # fd/cwd, then O(1)-test each candidate. FAIL-SAFE: no lsof binary → cannot verify → treat EVERY candidate
  # as live (reap NOTHING), the same "when unsure, KEEP" rule as Class C/D.
  declare -A uncov_live_set=()
  uncov_lsof_ok=1
  if command -v lsof >/dev/null 2>&1; then
    while IFS= read -r _n; do
      [ -n "$_n" ] && uncov_live_set["$_n"]=1
    done < <(lsof -w -F n 2>/dev/null | sed -n "s#^n${TMPDIR_ROOT}/\([^/]*\).*#\1#p")
  else
    uncov_lsof_ok=0
  fi
  uncov_idle=0
  uncov_live=0
  for d in "${uncov_cands[@]}"; do
    _name="${d##*/}"
    if [ "$uncov_lsof_ok" = 0 ] || [ -n "${uncov_live_set[$_name]:-}" ]; then
      uncov_live=$((uncov_live + 1)) # unverifiable (no lsof) OR holds an open fd/cwd → KEEP
    else
      [ "$APPLY" = 1 ] && rm -rf "$d" 2>/dev/null || true
      uncov_idle=$((uncov_idle + 1))
    fi
  done
  verb="WOULD remove"
  [ "$APPLY" = 1 ] && verb="removed"
  printf 'prune-tmp-inodes: age-GC (>=%s%%, >%smin, own-user): %s %s idle dir(s), KEPT %s live/held (of %s candidate(s); nix + claude exempt, lsof-idle guarded)\n' \
    "$INODE_THRESHOLD_PCT" "$TMP_REAP_MIN" "$verb" "$uncov_idle" "$uncov_live" "$uncovered"
else
  printf 'prune-tmp-inodes: age-GC DORMANT — inode-use %s%% below threshold %s%% (%s dir(s) >%smin tracked).\n' \
    "$iuse" "$INODE_THRESHOLD_PCT" "$uncovered" "$TMP_REAP_MIN"
fi

# Heartbeat (best-effort, never fails the prune): OVERWRITE a `.last-run` file next to the script. Its MTIME
# is a liveness proof the (silent, `>/dev/null`) cron actually FIRED — mirroring how the cpu-monitor's
# samples.tsv freshness proves ITS cron is alive — and its content shows the mode + current /tmp pressure.
# So "is this cron firing + keeping /tmp low?" is answerable after the fact WITHOUT cron mail (concierge
# silent-cron observability, 2026-08-29). Overwrite (not append) → bounded, no rotation needed.
printf '%s apply=%s inode-use=%s%% uncovered-stale-scratch=%s\n' "$(date -Is)" "$APPLY" "$iuse" "${uncovered:-?}" \
  > "$(dirname "${BASH_SOURCE[0]}")/prune-tmp-inodes.last-run" 2>/dev/null || true

# TREND APPEND (best-effort): during a pressure episode (inode-use >= TREND_LOG_PCT) append one tab-separated
# line so the CLIMB/REVERSAL trajectory the overwrite `.last-run` can't show is greppable. DEDUP consecutive
# IDENTICAL readings (same inode-use% AND same uncovered) — a SUSTAINED plateau at/above WARN (e.g. sitting
# at 85% for days awaiting a decision) would otherwise append ~96 identical lines/day forever, defeating the
# "bounded" intent. Logging only TRANSITIONS keeps it truly bounded AND makes it a cleaner change-log (one
# line per distinct reading). Same events-only discipline as reap-leases.log (#9690).
if [ "$iuse" -ge "$TREND_LOG_PCT" ]; then
  _trend="$(dirname "${BASH_SOURCE[0]}")/prune-tmp-inodes.trend"
  _sig="${iuse}:${uncovered:-?}"
  # Read the prior signature ONLY when the file exists — a `tail` of a missing file fails the pipe under
  # `set -o pipefail` and would abort (via `set -e`) BEFORE the append, dropping an episode's FIRST line.
  _last_sig=""
  if [ -f "$_trend" ]; then
    _last_sig="$(tail -1 "$_trend" | sed -n 's/.*inode-use=\([0-9]*\)%.*uncovered=\([^[:space:]]*\).*/\1:\2/p')"
  fi
  if [ "$_sig" != "$_last_sig" ]; then
    printf '%s\tinode-use=%s%%\tuncovered=%s\tapply=%s\n' "$(date -Is 2>/dev/null || echo now)" "$iuse" "${uncovered:-?}" "$APPLY" \
      >> "$_trend" 2>/dev/null || true
  fi
fi
