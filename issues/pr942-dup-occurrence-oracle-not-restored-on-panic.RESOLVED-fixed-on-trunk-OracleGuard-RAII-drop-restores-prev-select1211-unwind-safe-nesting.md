# PR#942 review comment — DUP_OCCURRENCE_ORACLE thread-local leaks on panic (cross-compile contamination) (v-wasm-opt)

Mirrored from GitHub PR#942 review comment (Copilot), id `3687515459`.
File: `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:1198` — select.rs → v-wasm-opt. Blame
`19fba99b2` "rcdzc(perf): collect_dup_sites O(binders×body-nodes)→O(N) via shared occurrence-bitset
early-prune".

## Comment (verbatim)

- (id 3687515459, select.rs:1198) "`collect_dup_sites` sets `DUP_OCCURRENCE_ORACLE` and clears it at the
  end, but if `mark_binder_dups` panics (e.g. via an assertion failure) the thread-local will remain set
  and could affect later compilations/tests running on the same thread. Using a small RAII guard that
  restores the previous value on `Drop` makes this robust to unwinding and avoids hidden cross-call
  state."

## Liaison verification (confirmed on trunk 512bf5610)

`collect_dup_sites` (select.rs:1191-1198): `DUP_OCCURRENCE_ORACLE.with(|o| *o.borrow_mut() =
Some((index, bitsets)));` then `for &binder in binders { mark_binder_dups(…, sites); }` then
`DUP_OCCURRENCE_ORACLE.with(|o| *o.borrow_mut() = None);`. The clear at the end is SKIPPED on an unwind:
if `mark_binder_dups` (or `build_occurrence_bitsets` mid-way) PANICS, the `= None` never runs, leaving the
thread-local `Some((stale index, bitsets))`. A later `collect_dup_sites` (or any reader of the oracle) on
the SAME THREAD then sees a STALE oracle from the panicked run → cross-compilation contamination. This is
the same PROCESS-STATE-CONTAMINATION class as the per-compile metric-counter trap
([[process-global-atomic-metric-counter-contaminated-by-parallel-test-harness]]) — a thread-local that
must be scoped-clean regardless of unwind. Especially reachable under the test harness (a panicking
assertion is a normal test outcome; catch_unwind or a multi-test thread reuse would carry the stale
oracle). Fix (Copilot's, standard): an RAII guard whose `Drop` restores the PREVIOUS value (not just
`None` — supports nesting) — set on entry, restored on scope exit including panic. Robustness/hygiene,
behavior-neutral on the happy path.

Owner: **v-wasm-opt** (select.rs `collect_dup_sites`, `19fba99b2`). Wrap the oracle set/clear in a
Drop-restoring RAII guard so a panic in `mark_binder_dups` can't leak the thread-local into later runs.
