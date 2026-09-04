#!/usr/bin/env bash
# baseline-drift-monitor.sh — DETECT-only cron for `.gate-baseline` drift (concierge-greenlit 2026-08-31,
# v-fleet-tooling gate-baseline automation). WHY: the committed `.gate-baseline` drifts behind the corpus
# when new cases land without a periodic re-baseline (`nix run .#save-baseline`) — it went 911 titles behind SILENTLY because nothing
# watched the count, so those cases lacked regression protection. This is the CHEAP DETECT half of the split
# design (my lane): a text-only scan (NO gate) that NOTIFIES v-corpus-harness when new-cases-lacking-a-baseline
# exceeds a threshold, so a trusted-store agent can run the heavy re-baseline + review + land. The heavy save
# itself stays TRIGGERED (never cron'd — it's the fleet's heaviest build + needs a warm store + review-before-
# land; a wrong baseline masks regressions).
#
# COUNT METHOD (AUTHORITATIVE, v-corpus-harness #7407): `cdz corpus baseline-drift --baseline <bl> <*.sexp>`
# emits `K missing-from-baseline` = corpus titles with NO baseline line (parsed via the sexpr reader, NOT a
# grep). This REPLACED the original naive grep proxy (`grep '(case "'`), which BROKE when the corpus case-
# header format changed (the title moved off the `(case` line → the grep under-counted 18 vs ~thousands, 2026-
# 09-01). Prefer `--count` (machine int); fall back to prose-parsing "K missing-from-baseline" for a cdz that
# has the subcommand but predates --count (worktree binaries lag their HEADs). Run a WORKTREE's release cdz
# (probe for one exposing `corpus baseline-drift`); fail-open if none. DRIFT = MIN-FLOOR: notify on GROWTH of
# missing-from-baseline above the lowest count ever seen (auto-~0 after a full re-baseline) — timing-robust, so
# the monitor is deployable regardless of re-baseline timing + never fires on a known pre-re-baseline staleness.
# Checks ONLY .gate-baseline (the full/primary); .gate-baseline-rust{,-async} are rust-runnable SUBSETS.
#
# Paths derive from this script's hub location (tracked at <repo>/fleet/, RUN from the <hub>/.claude/fleet/
# copy `fleet up` materializes — same split as cpu-monitor.sh). Reads the corpus/baseline from a current-
# main-ish worktree so the count reflects the tip. FAIL-OPEN + silent unless it notifies.
set -uo pipefail

HUB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKTREES="$(cd "$HUB/../worktrees" 2>/dev/null && pwd || true)"
[ -n "${WORKTREES:-}" ] && [ -d "$WORKTREES" ] || { echo "baseline-drift-monitor: no worktrees dir — skip." >&2; exit 0; }

THRESHOLD="${CDZ_BASELINE_DRIFT_THRESHOLD:-100}"   # notify when (corpus - baseline) title count exceeds this
COOLDOWN="${CDZ_BASELINE_DRIFT_COOLDOWN:-259200}"  # min seconds between notifications while drift persists (3d)
STATE="$HUB/baseline-drift-monitor.last-notify"    # mtime = last notify time (cooldown guard)

# Pick a CURRENT-MAIN-ish worktree (freshest HEAD at/behind origin/main) so the corpus/baseline reflect the
# tip; fall back to the freshest overall. Same selection as warm-keep.sh (a stale worktree → stale count).
main_sha=""
for wt in "$WORKTREES"/*/; do
  [ -d "${wt}spec/semantics" ] || continue
  main_sha="$(git -C "$wt" rev-parse --verify -q origin/main 2>/dev/null || true)"; [ -n "$main_sha" ] && break
done
best="" best_ct=-1 fallback="" fallback_ct=-1
for wt in "$WORKTREES"/*/; do
  [ -d "${wt}spec/semantics" ] || continue
  head="$(git -C "$wt" rev-parse --verify -q HEAD 2>/dev/null || true)"; [ -n "$head" ] || continue
  ct="$(git -C "$wt" show -s --format=%ct HEAD 2>/dev/null || echo 0)"
  [ "$ct" -gt "$fallback_ct" ] && { fallback_ct="$ct"; fallback="$wt"; }
  if [ -n "$main_sha" ] && { [ "$head" = "$main_sha" ] || git -C "$wt" merge-base --is-ancestor "$head" "$main_sha" 2>/dev/null; }; then
    [ "$ct" -gt "$best_ct" ] && { best_ct="$ct"; best="$wt"; }
  fi
done
wt="${best:-$fallback}"
[ -n "$wt" ] && [ -f "${wt}spec/semantics/.gate-baseline" ] || { echo "baseline-drift-monitor: no worktree with spec/semantics/.gate-baseline — skip." >&2; exit 0; }

# AUTHORITATIVE corpus-ahead count via `cdz corpus baseline-drift` (v-corpus-harness #7407). The old naive
# grep proxy (`grep '(case "'`) BROKE when the corpus case-header format changed (the title moved off the
# `(case` line onto the next line) → it under-counted (18 vs ~thousands). baseline-drift parses the corpus
# via the sexpr reader (immune to that) + emits `K missing-from-baseline` = corpus titles with NO baseline
# line = new cases lacking regression protection = the corpus-AHEAD-of-baseline drift this monitor exists to
# detect. Needs a cdz bin exposing `corpus baseline-drift`; worktree binaries LAG their HEADs (some stale, and
# some predate #7407's --count), so PROBE the picked wt's cdz first, then any worktree's, for a capable one.
CDZ=""
for cand in "${wt}target/release/cdz" "$WORKTREES"/*/target/release/cdz; do
  [ -x "$cand" ] || continue
  if "$cand" corpus baseline-drift --help >/dev/null 2>&1; then CDZ="$cand"; break; fi
done
[ -n "$CDZ" ] || { echo "baseline-drift-monitor: no worktree cdz exposes 'corpus baseline-drift' yet — skip (fail-open; a rebuilt worktree provides one)." >&2; exit 0; }

# Corpus files SHELL-EXPANDED (baseline-drift opens the literal glob otherwise). Prefer --count (machine int,
# #7407); fall back to parsing the prose "K missing-from-baseline" for an older cdz that has the subcommand
# but predates --count (binary-freshness reality — validated 2026-09-01).
sexps=( "${wt}"spec/semantics/*.sexp )
BL="${wt}spec/semantics/.gate-baseline"
missing="$("$CDZ" corpus baseline-drift --baseline "$BL" "${sexps[@]}" --count 2>/dev/null)"
case "$missing" in
  ''|*[!0-9]*)   # --count unsupported / non-integer → prose fallback (grep the "N missing-from-baseline" line)
    missing="$("$CDZ" corpus baseline-drift --baseline "$BL" "${sexps[@]}" 2>/dev/null \
      | grep -oE '[0-9]+ missing-from-baseline' | grep -oE '^[0-9]+' | head -1)"
    ;;
esac
case "$missing" in ''|*[!0-9]*) echo "baseline-drift-monitor: could not parse a missing-from-baseline count from cdz — skip (fail-open)." >&2; exit 0;; esac

# MIN-FLOOR drift (timing-robust — no re-baseline gate): notify on GROWTH above the LOWEST missing-count ever
# seen (= best baseline coverage achieved; ~0 after a full re-baseline). This makes the monitor deployable
# regardless of re-baseline timing: FIRST run initializes floor=missing → growth 0 → does NOT fire on the
# current pre-re-baseline staleness; when a re-baseline lands (missing→~0) the floor auto-ratchets DOWN, and
# thereafter growth = NEW corpus titles with no baseline line. Threshold = "new unprotected cases above floor".
FLOOR_STATE="$HUB/baseline-drift-monitor.floor"
floor="$(cat "$FLOOR_STATE" 2>/dev/null)"; case "$floor" in ''|*[!0-9]*) floor="$missing";; esac
[ "$missing" -lt "$floor" ] && floor="$missing"       # ratchet down to the new best coverage
printf '%s' "$floor" > "$FLOOR_STATE" 2>/dev/null || true
growth=$(( missing - floor ))
echo "baseline-drift-monitor: missing-from-baseline=$missing floor=$floor growth=$growth threshold=$THRESHOLD (worktree $(basename "$wt"))"

# SILENT-CRON OBSERVABILITY (.last-run every run; mtime = fired proof, distinct from .last-notify). Records
# the authoritative metric. Placed BEFORE the under-threshold early-exit so a quiet pass still records liveness.
printf '%s missing=%s floor=%s growth=%s threshold=%s\n' \
  "$(date -Is 2>/dev/null || echo now)" "$missing" "$floor" "$growth" "$THRESHOLD" \
  > "$HUB/baseline-drift-monitor.last-run" 2>/dev/null || true

[ "$growth" -gt "$THRESHOLD" ] || exit 0   # growth above the coverage floor under threshold → nothing to do

# Cooldown: don't re-notify within COOLDOWN while the growth persists (avoid daily spam until a re-baseline lands).
now="$(date +%s)"
if [ -f "$STATE" ]; then
  age=$(( now - $(stat -c %Y "$STATE" 2>/dev/null || echo 0) ))
  [ "$age" -lt "$COOLDOWN" ] && { echo "baseline-drift-monitor: growth=$growth > $THRESHOLD but within cooldown (${age}s < ${COOLDOWN}s) — not re-notifying." >&2; exit 0; }
fi

# NOTIFY v-corpus-harness (owns baseline review+land). Plain-text subject/body (no backticks/$() — env-leak
# safe, per the fleet-send discipline). Best-effort; touch the cooldown state regardless so a send hiccup
# doesn't spam. Run from the picked worktree (any worktree resolves the same hub inbox).
subj="baseline-drift: ~$missing corpus titles have no .gate-baseline line ($growth NEW above the coverage floor $floor, threshold $THRESHOLD) — new cases lacking regression protection; a whole-corpus re-baseline is due"
body="Automated fleet-health detect (baseline-drift-monitor cron, v-fleet-tooling). Authoritative count via 'cdz corpus baseline-drift': $missing corpus titles have NO line in spec/semantics/.gate-baseline (missing-from-baseline). Of those, $growth are NEW growth above the lowest-seen coverage floor ($floor) — newly-added cases with no regression protection (a gate has no pass/fail expectation for them). ACTION: a trusted-store agent should run the whole-corpus re-baseline (nix run .#save-baseline; the old 'cargo xtask gate --save' was deleted in #8318) to regenerate the 3 baselines, then review + land. DETECT-only cron; the heavy save stays triggered (never cron'd). Re-notify cooldown ${COOLDOWN}s. Tune the threshold via CDZ_BASELINE_DRIFT_THRESHOLD."
if (cd "$wt" && cargo xtask fleet send --to v-corpus-harness --from v-fleet-tooling --kind note --subject "$subj" --body "$body" >/dev/null 2>&1); then
  echo "baseline-drift-monitor: NOTIFIED v-corpus-harness (growth=$growth > $THRESHOLD, missing=$missing floor=$floor)."
else
  echo "baseline-drift-monitor: WARN could not send drift notification (will retry next tick past cooldown)." >&2
fi
mkdir -p "$(dirname "$STATE")" 2>/dev/null || true
: > "$STATE" 2>/dev/null || true   # stamp cooldown (mtime = now)
exit 0
