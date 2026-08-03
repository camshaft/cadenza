# PR #1780 review comments — rcdzc/src/{infer,tests}.rs (v-inference) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1780 (MERGED — the #1775 Set.of range-check work).

## 1. `first` bound-then-silenced in Set.of range-check (Copilot, infer.rs:10076) — cleanliness [same as #1775]
Same unused-`first` + `let _ = first;` noise I filed on #1775 — destructure with `..`/`_`. LOW.

## 2. Test name "set or map literal" but exercises Set.of + Map.insert constructors (Copilot, tests.rs:42128) — doc/naming
Rename to reflect the constructors actually tested (Set.of / Map.insert), not "literal". LOW.

## 3. Map.insert range-check test only asserts the out-of-range KEY, not value (Copilot, tests.rs:42150) — test-coverage
The range-check covers both inserted keys AND values, but the test only asserts the out-of-range key case;
add a value-out-of-range assertion so both arms are pinned. LOW-MED/test-coverage. Fix-forward.
