#!/usr/bin/env bash
# baseline-drift-monitor.sh — DETECT-only cron for `.gate-baseline` drift (concierge-greenlit 2026-08-31,
# v-fleet-tooling gate-baseline automation). WHY: the committed `.gate-baseline` drifts behind the corpus
# when new cases land without a periodic `gate --save` — it went 911 titles behind SILENTLY because nothing
# watched the count, so those cases lacked regression protection. This is the CHEAP DETECT half of the split
# design (my lane): a pure grep title-count scan (NO build, NO gate) that NOTIFIES v-corpus-harness when the
# drift exceeds a threshold, so a trusted-store agent can run the heavy `gate --save` + review + land. The
# heavy `gate --save` itself stays TRIGGERED (never cron'd — it's the fleet's heaviest build + needs a warm
# store + review-before-land; a wrong baseline masks regressions).
#
# COUNT METHOD (no build): corpus titles = `(case "…")` / `(platform-case "…")` headers across
# spec/semantics/*.sexp; baseline titles = non-comment lines in .gate-baseline (one `verdict\tdesc` per
# case). drift = corpus - baseline. This is a close PROXY for cdz-corpus baseline-drift's exact
# "missing-from-baseline" set-diff (which equals the count-delta whenever vanished==0 — the normal state,
# enforced by corpusVanishedCheck in localGate); a grep over/under-count of a handful is far below the ~100
# threshold, so the threshold DECISION is unaffected. Checks ONLY .gate-baseline (the full/primary baseline);
# .gate-baseline-rust{,-async} are SUBSETS (rust-runnable cases only) so their lower count is by-design, not drift.
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

corpus="$(grep -rhcE '^\s*\((case|platform-case) "' "$wt"spec/semantics/*.sexp 2>/dev/null | awk '{s+=$1} END{print s+0}')"
baseline="$(grep -vc '^#' "$wt"spec/semantics/.gate-baseline 2>/dev/null || echo 0)"
drift=$((corpus - baseline))
echo "baseline-drift-monitor: corpus=$corpus baseline=$baseline drift=$drift threshold=$THRESHOLD (worktree $(basename "$wt"))"

[ "$drift" -gt "$THRESHOLD" ] || exit 0   # under threshold → nothing to do

# Cooldown: don't re-notify within COOLDOWN while the drift persists (avoid daily spam until a gate --save lands).
now="$(date +%s)"
if [ -f "$STATE" ]; then
  age=$(( now - $(stat -c %Y "$STATE" 2>/dev/null || echo 0) ))
  [ "$age" -lt "$COOLDOWN" ] && { echo "baseline-drift-monitor: drift=$drift > $THRESHOLD but within cooldown (${age}s < ${COOLDOWN}s) — not re-notifying." >&2; exit 0; }
fi

# NOTIFY v-corpus-harness (owns baseline review+land). Plain-text subject/body (no backticks/$() — env-leak
# safe, per the fleet-send discipline). Best-effort; touch the cooldown state regardless so a send hiccup
# doesn't spam. Run from the hub's fleet workspace (any worktree resolves the same hub inbox).
subj="baseline-drift: .gate-baseline is ~$drift titles behind the corpus (corpus $corpus / baseline $baseline, threshold $THRESHOLD) — a gate --save on a trusted store is due (these cases lack regression protection)"
body="Automated fleet-health detect (baseline-drift-monitor cron, v-fleet-tooling). The committed spec/semantics/.gate-baseline is ~$drift case titles behind the corpus (corpus $corpus titles, baseline $baseline titles; threshold $THRESHOLD). Those new cases currently have NO regression protection (a gate would not have a pass/fail expectation for them). ACTION: a trusted-store agent (you, when warm, or route to a trusted-store peer) should run cargo xtask gate --save to regenerate the 3 baselines, then you review + land them. This is the DETECT half of the split automation; the heavy gate --save stays triggered (never cron'd). Re-notify cooldown is ${COOLDOWN}s. Tune the threshold via CDZ_BASELINE_DRIFT_THRESHOLD."
if git -C "$wt" rev-parse >/dev/null 2>&1 && (cd "$wt" && cargo xtask fleet send --to v-corpus-harness --from v-fleet-tooling --kind note --subject "$subj" --body "$body" >/dev/null 2>&1); then
  echo "baseline-drift-monitor: NOTIFIED v-corpus-harness (drift=$drift > $THRESHOLD)."
else
  echo "baseline-drift-monitor: WARN could not send drift notification (will retry next tick past cooldown)." >&2
fi
mkdir -p "$(dirname "$STATE")" 2>/dev/null || true
: > "$STATE" 2>/dev/null || true   # stamp cooldown (mtime = now)
exit 0
