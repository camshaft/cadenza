;; MISCOMPILE — TRAP REORDER (confirmed by corpus-bugfix triage 2026-07-15, trunk@65d4b0a47). The common-
;; constructor if-hoist `hoist_common_ctor` (lower.rs:13588, guard at :13656) rewrites
;;   (if cond (K …p) (K …q))  →  (K …(if cond pᵢ qᵢ))
;; evaluating `cond` once per DIFFERING payload position. The existing guard only blocks the transform
;; when `diff != 1 && !is_trap_free(cond)` — it does NOT cover the case where `diff == 1` but a SHARED
;; (non-differing) payload BEFORE the differing position can trap. In the hoisted form those shared
;; payloads are built OUTSIDE the per-position `if`, so they evaluate BEFORE `cond`. If both a shared
;; payload AND `cond` can trap, the hoist changes WHICH trap is observed.
;;
;; Original `(if cond A B)` evaluates `cond` FIRST. Here `cond = (< (+ i64::MAX e) 5)` OVERFLOWS (checked
;; +), so the program must trap "integer overflow". But the hoisted form evaluates the shared payload
;; `(/ 10 d)` first; at d=0 that traps "integer divide by zero" — the WRONG trap. Verified: main 0 1
;; yields "integer divide by zero" where cond-first semantics demand "integer overflow".
;;
;; Origin: Copilot inline review on PR #375 (discussion r3589185980), confirmed live by corpus-bugfix.
;; FIX DIRECTION: also require `is_trap_free(cond)` (or that the differing position is FIRST / all
;; preceding shared payloads are trap-free) before hoisting a possibly-trapping cond, even when diff==1.
(case "a trapping shared payload before the differing position must not preempt a trapping cond"
  (doc    "(if cond (tuple (/ 10 d) 1) (tuple (/ 10 d) 2)) with cond = (< (+ i64::MAX e) 5) — a checked
           overflow. Original if-semantics evaluate cond FIRST, so at any d the program traps 'integer
           overflow'. The common-constructor hoist rewrites to (tuple (/ 10 d) (if cond 1 2)), evaluating
           the shared (/ 10 d) before cond; at d=0 it traps 'integer divide by zero' — the wrong trap.
           The fix must guard a possibly-trapping cond even when only one payload position differs.")
  (input (do
    (def (main (: d Int64) (: e Int64))
      (if (< (+ 9223372036854775807 e) 5)
          (tuple (/ 10 d) 1)
          (tuple (/ 10 d) 2)))
    (export main)))
  (call main (: 0 Int64) (: 1 Int64))
  (output (trap "integer overflow")))
