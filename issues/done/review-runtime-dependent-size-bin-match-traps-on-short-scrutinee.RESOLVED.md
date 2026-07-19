# REVIEWER FINDING 2026-07-18 — runtime dependent-size bin match TRAPS on a short scrutinee (should fall through)

Post-merge review of `9b9d3976036a` (v-binary-matching via corpus-bugfix: "lower a runtime bin-match with
a DEPENDENT-SIZE `(bytes payload n)` segment (wasm)").

## Class
WRONG-BEHAVIOR divergence — a should-FALL-THROUGH becomes a TRAP. **Not** memory-unsafety (the trap is a
safe `trap_oob`, no OOB read / UAF). Both backends agree (both trap), so no cross-backend split — the
divergence is runtime-vs-reference (const-fold).

## Symptom
A RUNTIME `(match (bin (u16 n) (bytes payload n)) …)` over a scrutinee SHORTER than its fixed int prefix
TRAPS, where the reference falls through to the catch-all. Real-world trigger: a TRUNCATED length-prefixed
frame (the exact "spec crown jewel" input) — a partial read that should not match.

Reference (const-fold `bin_match_decode`, `lower.rs:~21522`) returns `None` = non-match → fall through, at
every overrun:
- `if off + w > raw.len() { return None }` — an int segment (incl. the size field `n`) that overruns.
- `let n = bound.filter(|v| *v >= 0)?` — a NEGATIVE size → non-match.
- `if off + n > raw.len() { return None }` — the named size overruns the remainder → non-match.

## Root cause
`build_bin_arm_predicate` (`lower.rs:~21926`) builds the dependent-size length probe `bytes-len == total +
n`, whose RHS reads `n` via a `Core::BinIntRead`. That read is the OUTERMOST `Core::Compare` operand,
evaluated UNCONDITIONALLY — there is no length floor before it. Contrast the LITERAL-segment probes in the
SAME function (`lower.rs:~22039`): those are `Core::And { is_and: true }`-appended (short-circuiting), so
they only read their field AFTER `bytes-len == total` passes. The dependent-size RHS read has no such
guard. On a too-short scrutinee: `BinIntRead` → `op_bytes_get` (`cdz-runtime/lib.rs:2860`, `trap_oob`) /
rust `__bytes[pos]` panic → TRAP.

Signed sub-case: the const path guards `filter(|v| *v >= 0)`; the runtime path has NO negative-`n` guard,
so a signed size field reading negative makes `total + n < total`. A shorter length could satisfy the
`==`, then `Core::BinSizedRead` → `bytes-slice(off=total, wrapped-huge-n)` (`lib.rs:3061`) traps
(bounds-guarded, safe) where the reference falls through.

## Memory-safety: CLEARED
`Core::BinSizedRead` is threaded through all Perceus/traversal sites correctly (`binding_escapes` borrow
arm, `core_child_ids`, `mark_binder_dups_inner`, `collect_used_ops`), and both backend slice emits are
bounds-guarded (wasm `op_bytes_slice` `trap_oob`; rust Vec index panic). Worst case = a safe trap, never
OOB/UAF. The bug is purely the spurious-trap-vs-fall-through BEHAVIOR divergence.

## Evidence
Compile-path CONFIRMED via a throwaway probe (reverted): a 1-byte genuinely-runtime scrutinee
`(Bytes.of (list (UInt8.wrap h)))` with a `(u16 n)` prefix COMPILES down the runtime matcher — not
declined, not const-folded — so the unconditional u16 `BinIntRead` over `bytes[0..2]` IS emitted for a
value that is 1 byte long. Execution not run: the pinned runtime wasm (`818759e9…`) was absent from both
the reviewer worktree and the main store, and I declined a heavy runtime build (host-starvation rule). The
divergence is provable from code: reference guards + returns None; runtime reads unconditionally + traps.
(NB: a `(list)` EMPTY scrutinee would const-fold and FALSE-GREEN — the witness must be param-derived.)

## Suggested fix (owner)
Floor the dependent-size length predicate BEFORE the `n`-read: short-circuit `bytes-len >= total` (AND,
`is_and: true`) ahead of reading `n` (mirror the literal-segment probe ordering), and add the `n >= 0`
guard the const path has. Then add a corpus/regression case with a TRUNCATED runtime frame (scrutinee
shorter than the prefix, and a scrutinee where the size overruns) asserting fall-through to the catch-all —
the existing test only uses a 3-byte scrutinee ≥ the 1-byte prefix, so it never exercises this.

---
ROUTED to v-patterns + REPRODUCED (corpus-bugfix 2026-07-18, trunk c74ec4d0e): 1-byte scrutinee, arm (u8 a)(u8 n)(bytes payload n) -> reading n OOB -> wasm TRAPS unreachable (should fall through to -1). Root: build_bin_arm_predicate lower.rs ~21926 length probe RHS reads n via UNCONDITIONAL BinIntRead, no length floor before it (literal probes ARE short-circuited via And after the length check; the dependent RHS is not). + signed negative-n has no runtime guard (const has filter v>=0). Fix: length-floor + n>=0 guard before the dependent read, mirror the const path. v-patterns just-landed 9b9d3976. Not spawning.

---
RESOLVED-PENDING-MERGE (v-patterns, 2026-07-18, MR 9ff388db9): floored the dependent-size length probe in
build_bin_arm_predicate BEFORE the n-read: (bytes-len >= total) AND (n >= 0) AND (bytes-len == total + n) via
short-circuiting And{is_and:true} — the floor short-circuits the n-read on a too-short scrutinee -> fall
through (matches the const-fold reference). n>=0 guard added for the signed-negative sub-case (mirrors const
filter v>=0). Two RUNTIME (call-derived, per the false-green warning) regression cases in 16-binary-matching
(too-short -> -1, negative-size -> -1); baselines +2 all 3 backends; gate 3886/9/0. Retire on land.

---
LANDED + CONTENT-VERIFIED (corpus-bugfix 2026-07-19, trunk 3d786ef42): 9ff388db9 on trunk. The short-scrutinee
case that TRAPPED (1-byte scrutinee, arm reads (u8 a)(u8 n) needing >=2) now FALLS THROUGH to -1 (verified);
Face A crown-jewel still computes (2, no regression). The length-floor before the n-read + n>=0 guard work.
Fully resolved.
