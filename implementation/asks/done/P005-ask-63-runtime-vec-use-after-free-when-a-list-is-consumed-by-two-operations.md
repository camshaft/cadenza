## 63. ✅ FIXED + VALIDATED (2026-07-08) — a built-in `list` value is FREED TOO EARLY (use-after-free / double-drop in the runtime RC) when it is consumed by TWO operations in one function

**✅ RESOLVED — the runtime agent fixed the RC discipline; validated by the conformance loop 2026-07-08.**
On the STABLE toolchain (`stable/cadenza-seed` + `stable/cdz_runtime.wasm`, 55289-byte component):
- The minimal reproducer (`/tmp/vec-share-drop-repro.cdz`) now runs to **`Value("0")`** (was: `Trap` in
  `op_drop`/`vec-push`/`talc::deallocate`) — 100→slot 0, 192→slot 0 ⇒ 0+0 = 0. ✔ expected.
- The pinned corpus case **"a list value consumed by two operations in one function is not freed early"**
  (`spec/semantics/05-compound-types.sexp`) now **PASSES** the behavior-gate (was the deliberate 1 FAIL).
- Multi-consume stress all green: two-op consume ⇒ 12, three-op ⇒ 3, sibling lets `(+ (let((x 2))x)(let((y 1))y))`
  ⇒ 3 (the exact case that broke the ask-62 migration pre-fix).
- **ask-62 (the migration this blocked) is now UNBLOCKED and steps 1+2 LANDED** (IList/FList/DList →
  built-in `list`), value-harness 34 agree / 5 soft / 0 hard / 0 error — full parity. See ask-62.

⚠ Loop-side note (NOT a seed bug): a standalone `CADENZA_RUNTIME` wasm rebuilt with plain
`cargo build` (instead of `cargo component build`) is a CORE MODULE, not a component, and makes EVERY
heap case fail with "value-heap runtime component invalid: failed to parse WebAssembly module" — which
masqueraded as a 103-FAIL regression mid-validation. Always use `stable/` artifacts (or `cargo component
build`). Recorded in memory `runtime-wasm-build-recipe-cargo-component`.

MOVE TO done/. Original report follows.

---

## 63. 🔴 CONFIRMED SEED/RUNTIME BUG (BLOCKING) — a built-in `list` value is FREED TOO EARLY (use-after-free / double-drop in the runtime RC) when it is consumed by TWO operations in one function

**Severity: 🔴 blocking, a genuine miscompile/runtime-corruption.** This is exactly the "block on it, don't work
around" class. It BLOCKS ask-62 (retire the custom cons-list types for the built-in `list`) — the moment the
compiler's param-env becomes a built-in `list` (an RC'd runtime value) instead of a cons-sum-type, any program
with SIBLING lets (`(+ (let ((x 2)) x) (let ((y 1)) y))`) miscompiles: the reader passes the shared env to both
operand-reads, which double-consumes the list and corrupts it — surfacing as an invalid emitted component
(`local.get 192`, a freed-then-reused prelude index leaking into a local slot).

**Minimal reproducer (12 lines, saved `/tmp/vec-share-drop-repro.cdz`) — TRAPS in the runtime allocator:**
```
(module m
  (def (ienv-pos-go xs target k best)
    (match (List.at xs k) ((None _) best) ((Some h) (ienv-pos-go xs target (+ k 1) (if (= h target) k best)))))
  (def (read-operand env nameidx)
    (let ((e2 (List.push env nameidx))) (ienv-pos-go e2 nameidx 0 (- 0 1))))
  (def (read-both env) (+ (* 10 (read-operand env 100)) (read-operand env 192)))   ; env consumed TWICE
  (def (main) (read-both (list))))
```
`emit` → runs → **`Trap`**:
```
0: cdz_runtime.wasm!talc::Talc::deallocate
1: cdz_runtime.wasm!cdz_runtime::op_drop
2: cdz_runtime.wasm!cadenza:runtime/heap#vec-push
```
So a `vec-push` calls `op_drop` and hits `talc::deallocate` on already-freed / mis-refcounted memory — a
use-after-free / double-free in the persistent-vector RC path.

**The trigger, minimized (each removal makes it STOP trapping — so all are load-bearing):**
- A function param that is a `list` (`read-both`'s `env`), passed to **TWO** consuming calls (`read-operand env
  100`, `read-operand env 192`) — i.e. the list value has refcount usage across two ops in one function body.
- Each consumer does `List.push env …` then scans the result (`List.at` in the recursive `ienv-pos-go`).
- Wrapped in **checked arithmetic** (`(+ (* 10 …) …)`) — the `+`/`*` overflow-guard path. Removing the `(* 10)`
  OR inlining `read-both` into `main` OR pushing onto two DIFFERENT lists (not a shared param) makes it NOT trap.
  So it's the interaction: **a shared list value consumed by 2 ops, across a function-call boundary, under checked
  arith** → the drop logic frees the shared backing too early.

**Diagnosis trail (why this is the seed/runtime, NOT my compiler.cdz logic):** the compiler.cdz IList→list
migration (ask-62) is logically correct — `ienv-pos`/`ienv-snoc`/`ienv-len` over a built-in list return the right
values in isolation AND when run through the migrated compiler's own compiled code (`(ienv-pos (ienv-snoc (list)
192) 192 0)` → 0), and the migrated reader builds a structurally-correct 2-let tree (`count-lets` = 2). Yet the
migrated compiler emits `local.get 192` (invalid) for sibling lets. The minimal reproducer above then reproduces
the underlying trap with NO compiler.cdz involved — pure `List.push`/`List.at`/checked-arith. So it is a runtime
RC / persistent-vector `op_drop` bug, independent of the compiler.

**Likely locus (for the runtime agent):** `op_drop` / the RC decrement in `cdz-runtime`'s persistent-vector
(`vec-push`) — when a `list` handle is dup'd for two consumers, the drop count is off by one (a missing `dup`, or
a drop that recurses into shared backing it shouldn't). Cross-check against the Perceus RC discipline: a value used
twice needs a `dup`; a shared persistent-vector node freed by one consumer's drop while the other still references
it is the classic bug. The `#[global_allocator]` talc `deallocate` in the backtrace is the symptom (freeing live
memory), not the cause.

**Acceptance signal.** The reproducer runs to `Value("0")` (100 at slot 0, 192 at slot 0 → 0+0). Then ask-62's
IList→built-in-list migration compiles sibling-lets correctly (the byte gate stays 0-disagree, value-harness 0
error). More broadly: a `list` consumed by N operations in one function is refcounted correctly (no early free).

**✅ CAPTURED AS A CORPUS CASE (2026-07-07, per operator — "we have a repro in the semantics directory, right?").**
Added to `spec/semantics/05-compound-types.sexp`: **"a list value consumed by two operations in one function is not
freed early"** — `(module m (def (scan xs k) (match (List.at xs k) ((None _) 0) ((Some h) (+ h (scan xs (+ k
1)))))) (def (use e n) (scan (List.push e n) 0)) (def (both e) (+ (* 10 (use e 1)) (use e 2))) (def (main) (both
(list))))` with oracle `(: 12 Int64)`. It uses ONLY built-in list ops (no custom types), pins the correct
semantics (a list consumed twice is immutable → both consumers see the empty base → `10*1 + 2` = 12), and it
FAILS the native behavior-gate TODAY (`BEHAVIOR-GATE: FAIL (1 contradict the recorded semantics)` — the reference
compiler TRAPS in `op_drop` while the oracle says 12). So the bug is now a PERMANENT, gate-enforced regression
test that stays red until the RC fix lands — not an ephemeral `/tmp` file. ⚠ NOTE for the compiler agent: this is
a DELIBERATELY-ADDED failing test for THIS open bug, not a spec defect — the behavior-gate's 1 failure IS ask-63.

**Status.** 🔴 CONFIRMED, BLOCKING, seed+runtime. Reproducer minimized, saved, AND pinned as a corpus case (native
behavior-gate red until fixed). ask-62 (built-in-list migration)
is BLOCKED on this — I reverted the migration to keep compiler.cdz gate-green (139/0), and will NOT re-attempt it
until this RC bug is fixed. Related: [[champ-runtime-implemented-native]] (the RC heap), ask-62 (the migration this
blocks), the Perceus/FBIP RC discipline in the runtime.

---

## ✅ FIXED (2026-07-07, conformance loop) — COMPILER-SIDE dup, NOT a runtime change

**Root cause was the COMPILER, not the runtime.** The runtime ops CONSUME their heap arg (documented
FBIP contract, correct as designed); the seed emitted **zero `dup`s** — it treated every heap value as
linear. So a `Kind::Heap` local read by two consuming ops was freed by the first, double-freed by the
second. Nothing wrong with `vec-push`/`op_drop`.

**Fix (crash-safe unblock, per operator "unblock the sibling first, then finish Perceus, contract is
reworkable"):** `gen_name` now emits `dup` after every `Kind::Heap` `local.get`. Each reader gets its
own +1 reference to consume; the local's stored reference is never the one consumed ⇒ the refcount can
NEVER underflow (a double-free is impossible for any number of reads). It over-retains (a leak — the
owning reference isn't reclaimed), which the precise-drop Perceus pass (M2 Phase D, task #9) will close.
Scalars untouched (byte-identical); aliases re-emit their node (fresh construction, no shared handle).

**Verified:** the reproducer runs to `Value("12")`; the pinned corpus case "a list value consumed by
two operations in one function is not freed early" is now PASS; behavior-gate 597/0, ignition PASS,
component-check 599 agree / 0 disagree, cargo test green.

**⇒ ask-62 (built-in-list migration) is UNBLOCKED** — a param-env that is a built-in `list` consumed by
sibling operand-reads no longer double-frees. Re-attempt the IList/FList/DList→built-in-list migration.

**Status: pending-validation** (sibling re-probes: re-run the migration + confirm the byte gate stays
0-disagree). Full Perceus (precise drops so the over-retention leak closes) is task #9, next up.
