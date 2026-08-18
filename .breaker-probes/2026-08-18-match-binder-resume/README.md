# Match BINDER on resume result (2026-08-18)

- `pyr7.sexp` — (match (resume ...) (0 <literal arm>) (r <binder arm>)).
  DECLINED at bank time (uniform, "resume outside a lowered handler arm")
  — the literal+binder MIX is the decline; fixed by 8ee1e7660 (match-
  scrutinee branch beside the let-init branch), flip oracles 173/126.
- `pyr9.sexp` — BARE-binder single arm (match (resume ...) (r ...)).
  PASSES x3 on trunk 3cd560c66 (pre-8ee1e7660): the generic refold
  already handles the single-arm scrutinee; only the literal+binder mix
  re-lowered the resume.

CORRECTION (tick 1753): the tick-1750 micro-ladder overstated the
boundary. It ran against a STALE target/release/cdz (Aug 16, predating
even the 6c52dbc3c let fix); the gate builds fresh release-debug. Fresh
re-run: bare-binder FOLDS, literal+wildcard FOLDS, literal+binder
DECLINES. The filed issue's "bare-binder single arm declines" line was a
stale-binary artifact — corrected with v-effects. Direct-compile
diagnostics must use target/release-debug/cdz (the gate's profile), not
target/release.

UPDATE (tick 1755): pyr7 fix landed as e6eb3831b (match-scrutinee branch
in the two-hole refold). pyr7 flips to PASS x3 with fresh binaries; pyr9
stays pass; full ladder compiles clean. Both binder twins closed
(let 6c52dbc3c, match e6eb3831b). pyr7+pyr9 promote in batch 321.
