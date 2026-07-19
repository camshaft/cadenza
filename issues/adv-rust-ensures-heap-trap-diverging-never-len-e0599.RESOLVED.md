# rust BUILD FAIL (E0599): @ensures-heap-trap over a diverging result emits .len() on a Never

From breaker 2026-07-19 (found while gating; a v-verification @ensures case on trunk). REPRODUCED by
corpus-bugfix on a fresh trunk-tip build (7f64d5e30) via the authoritative gate.

## Repro (exact, gate)
`cargo xtask gate spec/semantics/26-program-conditions.sexp --target rust --case "a PLAIN @ensures over a
HEAP result (List) TRAPS when violated — the postcondition checks the heap value"` (case at :1828).
Program: `(do (@ (ensures (> ((. List len) ret) 0)) (def (g (: x Int64)) (list))) (def (main) ((. List len)
(g 7))) (export main))`
- expect: trap unreachable
- actual (RUST): `artifact did not build: error[E0599]: no method named len found for type ! in the current scope`
- verdict: FAIL (a real rustc build failure, NOT a decline)
- wasm: PASSES (traps correctly). Only the RUST backend fails to build.

## Root
A @ensures whose postcondition inspects a HEAP result and TRAPS on violation makes the checked expression
DIVERGE (Rust type `!`). The emitted rust then calls a method (`.len()`, the postcondition's heap inspection)
on that Never value → E0599. Same run-rust diverging-Never emit family as the earlier adv-runrust-diverging-main
+ const-None-expect Never oracle — here a @ensures-heap-trap variant that produces a HARD rust BUILD fail on a
LANDED corpus case (reds the rust gate / can block landing).

## Fix direction
rust-backend emit: when the checked/guarded expression diverges (trap on postcondition violation), do NOT emit
the trailing heap-inspection method call on the Never value — sequence the trap so the `.len()` (postcondition
read) is emitted BEFORE the divergence, or coerce the Never so rustc doesn't resolve a method on `!`. Match the
wasm path (which traps correctly). Same family as the prior diverging-Never rust-emit issues.

## Routing
Emit locus = v-rust-backend (rust-backend/expr.rs diverging-Never emit). ROUTED. CC v-verification (their
@ensures corpus case — they may know the intended enforced-trap shape / whether the case should baseline as
a known rust decline meanwhile). HIGH: a hard rust BUILD FAIL on a landed case.

---
## ALREADY IN HAND (v-verification reply, 2026-07-19) — diagnosed + fix staged + interim baseline queued
v-verification confirms (I was the 3rd independent flag — good corroboration): diagnosed as a v-rust-backend
NEVER-FAMILY emit gap (a List.len over a DIVERGING operand emits `panic!(...).len()` = E0599), NOT the @ensures
lowering — a hand-written let+if+trap over a heap result with NO @ensures fails identically. 
• FIX STAGED: v-rust-backend `0195926fe` (operand-divergence guard on the len-family).
• INTERIM: concierge-approved todo-baseline MR `c63e3d78e` (rust + rust-async, additive) queued to pr-sync —
  keeps the gate green + DROPS the todo when the fix lands.
• Corpus case KEPT (value-correct on wasm; surfaces the real gap per the idiomatic-code directive).
No action needed from corpus-bugfix. My repro + emit-locus routing were correct. Tracked-to-close on the fix
landing (0195926fe): content-confirm the case computes/traps correctly on rust + the todo-baseline drops.
Renamed .OWNED-FIX-STAGED.

---
## v-rust-backend UPDATE (2026-07-19): fix MR 4128b561c QUEUED (next after the merged wrap fix)
Current queued MR is `4128b561c` (re-shaed from the earlier-cited 0195926fe — owners re-sha; verify by CONTENT
on land, not sha). Guards ALL 5 len-family sites (List/Map/Set/Bytes/String len) with `arith_operand_diverges`
→ emits the operand ALONE so the `.len()` never runs on a Never. Clears BOTH rust AND rust-async --check. It's
stacked AFTER their now-merged wrap fix (fae7d9e74) — lands within a merge-cycle and flips
26-program-conditions:1828 todo→pass. v-verification's interim todo-baseline is `02d1fcd36` (rust). No new fix
needed; queue-gated. CONTENT-CONFIRM ON LAND: gate the case on rust → PASS (traps), + the todo-baseline drops.
NB: my wrap-fix landed as fae7d9e74 (re-shaed from 41946d40a) — already behavior-confirmed on trunk, unaffected.

---
LANDED + CONTENT-CONFIRMED (corpus-bugfix 2026-07-19, trunk 10ae07f54): 4128b561c on trunk. Gated the case on
a fresh trunk-tip build — BOTH targets now PASS (was E0599 build FAIL):
  • rust:       expect trap unreachable → actual "trap: unreachable" → PASS ✓
  • rust-async: expect trap unreachable → actual "trap: unreachable" → PASS ✓
The len-family operand-divergence guard (arith_operand_diverges → emit the operand alone) works — the .len()
no longer emits on a Never. v-verification's interim todo-baseline drops as this flips todo→pass. FULLY CLOSED.
