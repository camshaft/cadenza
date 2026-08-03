# PR#959 + PR#961 review comments — emit-once memoizes false during scheme-solve (⚠ correctness) + env-knob nits (v-compiler-perf)

Four Copilot review comments, all `rcdzc/src/lower.rs` emit-once, all `rcdzc(perf):` commits →
v-compiler-perf (route by COMMIT PREFIX, not blame-author — shared git identity;
[[liaison-route-by-commit-prefix-not-blame-shared-git-identity]]).

## Comment 1 (verbatim) — PR#959, lower.rs:11830, ⚠ CORRECTNESS

- (id 3693671598, lower.rs:11830) "`emit_once_callee_eligible` memoizes `false` even when
  `infer::def_scheme` returns `None`. `def_scheme` intentionally returns `None` *without caching* while
  parameter/scheme solving is in progress to avoid poisoning later queries (see its docs), but
  `emit_shared` will permanently cache `false` for that callee. This can change behavior vs the pre-memo
  logic and can prevent emit-once from ever triggering once the scheme becomes determined."

### Liaison verification (confirmed on trunk 8fcd4308a; blame `bf12bddd1` `rcdzc(perf):`)

`emit_once_callee_eligible` (11854-11860): `if let Some(&v)=db.emit_shared.get(&callee) {return v;} let
eligible = ..._uncached(db,callee); db.emit_shared.insert(callee, eligible); eligible` — caches
UNCONDITIONALLY. `_uncached` (11875): `let Some(scheme)=def_scheme(db,callee) else { return false; //
undetermined signature }`. `def_scheme` returns `None` WITHOUT caching while scheme-solving is in progress
(deliberately, to avoid poisoning). So when a callee is queried mid-solve, `_uncached`→false, and the
outer fn PERMANENTLY writes `false` into `emit_shared`. Once the scheme later determines, the cached
`false` sticks → emit-once NEVER fires for that callee (a behavior change vs the pre-memo per-call
recompute). Fix: do NOT memoize when the `false` stems from the `def_scheme==None` in-progress branch —
either return an uncached `false` in that case (distinguish "structurally ineligible" from "not yet
determined") or skip the `emit_shared.insert` when `def_scheme` was `None`. Correctness / perf-correctness.

## Comments 2-4 (verbatim) — PR#961, env-knob nits

- (id 3693808268, lower.rs:11800) "If `CDZ_INLINE_COST_THRESHOLD` is set but fails to parse, the code
  silently falls back to the default. That can invalidate tuning/measurement runs… Consider failing fast
  when the env var is present but invalid."
- (id 3693808368, lower.rs:11812) "If `CDZ_INLINE_MIN_CALLERS` is set but fails to parse, this silently
  falls back to the default. For perf-sweep knobs, it's usually better to error out so an accidental
  typo doesn't produce misleading results."
- (id 3693808320, lower.rs:11890) "This comment still refers to `INLINE_MIN_CALLERS`, but the constant
  was renamed to `INLINE_MIN_CALLERS_DEFAULT` and the effective value now comes from
  `inline_min_callers()`. Updating the comment keeps the documentation consistent."

### Liaison verification (confirmed on trunk 8fcd4308a; blame `c24540b68` `rcdzc(perf):`)

- 11796-11800 + 11808-11812: both knobs do `env::var(...).ok().and_then(|v| v.parse().ok())
  .unwrap_or(DEFAULT)` — a PRESENT-but-unparseable value silently uses the default. For perf-SWEEP knobs
  this is a real footgun: a typo'd `CDZ_INLINE_COST_THRESHOLD=4O` (letter O) measures the DEFAULT while
  the operator thinks they swept 40 → misleading sweep data. Fail-fast (panic/error) when the var is
  PRESENT but invalid (unset → default is fine). Both knobs.
- 11890: the cost-gate comment says "called at ≥ INLINE_MIN_CALLERS sites" but the constant is now
  `INLINE_MIN_CALLERS_DEFAULT` and the effective value comes from `inline_min_callers()` (env-overridable).
  Stale comment; update to the knob name. (Also the 11833 doc "min-callers floor (`inline_min_callers`)"
  is already correct — only the 11890 inline comment lags.)

Owner: **v-compiler-perf** (all `rcdzc(perf):` — `bf12bddd1` memoize + `c24540b68` knobs). Comment 1 is
correctness (don't cache the in-progress `false`); 2-4 are knob-hygiene (fail-fast on invalid env + stale
comment).
