; BREAKER FINDING 2026-07-17 (trunk d1d09dfcc) — a CHECK/COMPILE DIVERGENCE (spurious decline of a
; well-typed program) + a BOGUS DIAGNOSTIC on one face. The trigger shape is a handle whose seed is a
; RUNTIME parameter, whose arm is the IDENTITY (`resume s s` — state neither read into the value with
; arithmetic nor advanced), and whose body is a BARE perform:
;
;     (def (main (: k Int64))
;       (handle St k ((get (u) s (resume s s))) (St.get)))
;
;   cdz check  -> PASSES rc=0 (well-typed; the value is plainly Int64 = the seed)
;   wasm/rust  -> DECLINE "function return type has no machine representation" /
;                 "result type Any has no native Rust representation"
;   Expected: k=9 -> 9 (the handler resumes the seed).
;
; WORSE — the HELPER face emits a NONSENSE diagnostic naming the CALLER'S ARGUMENT with no span:
;     (def (f (: n Int64)) (handle St n ((get (u) s (resume s s))) (St.get)))
;     (def (main (: k Int64)) (f k))
;   -> `cdz: error [CDZ0101]: unbound name `k``   (no file:line:col; k IS bound — it is main's param)
; A user seeing "unbound name k" for a well-formed program has no path to the real cause (the
; handler's un-resolved result type). rust backend: same bogus unbound-k.
;
; HEALED by ANY of (each verified):
;   - the arm ADVANCING state:      ((get (u) s (resume (+ s 1) (+ s 1))))   -> runs (10)
;   - a COMPOUND body:              (+ 0 (St.get))                            -> runs (9)
;   - a LITERAL seed:               (handle St 5 …)                           -> runs (6)
;   - a let-wrapped body does NOT heal: (let ((v (St.get))) v)                -> same decline
; So the specialization/typing of the handle's result only grounds when the arm or body forces the
; state/value type through an operation; the pure pass-through (seed -> resume s -> body's perform
; result) leaves the handle's type variable at Any into lower/emit, where it declines (or worse,
; mis-reports). cdz check has already solved it as Int64 — the divergence is between infer and the
; monomorphization/lowering path's own re-derivation.
;
; SEVERITY: reject-not-miscompile (no wrong value) — but it rejects a WELL-TYPED program the checker
; accepts, and the helper face's diagnostic is actively misleading (names a bound variable in the
; caller, no span). Two fix loci: (1) ground the handle result type from the solved scheme rather
; than re-deriving in lower; (2) whatever emits CDZ0101 there must never name a bound occurrence.
;
; Expected under fix: k=9 -> 9 on all backends for both faces.
(case "a runtime-seeded handler whose identity arm resumes the seed through a bare perform yields the seed"
  (doc    "`(handle St k ((get (u) s (resume s s))) (St.get))` with `k` a boundary parameter: the arm
           resumes the current state unchanged, so the body's single perform reads the seed — k=9 -> 9.
           cdz check accepts (Int64); the backends currently DECLINE ('function return type has no
           machine representation' — the handle's result left at Any by the lowering's re-derivation),
           and the helper-wrapped face mis-diagnoses `unbound name k` (a bound caller variable, no
           span). Advancing the state, a compound body, or a literal seed each heal it — the pure
           pass-through is the only face that fails, and it is well-typed.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (handle St k
                ((get (u) s (resume s s)))
                (St.get)))
            (export main)))
  (call   main (: 9 Int64))
  (output (: 9 Int64)))

; ---
; ROUTED (corpus-bugfix 2026-07-17, trunk d1d09dfcc, VERIFIED): check rc=0, wasm declines "function
; return type has no machine representation" — handle result left at Any in lower (infer solved Int64).
; PRIMARY -> v-effects (ground handle result from solved scheme in lower, don't re-derive). DIAGNOSTIC
; locus -> v-inference (CDZ0101 must not name a bound occurrence / emit spanless). Not spawning a fixer
; (3-fixer cap). Promote when fixed.

; ---
; LOCUS 1 RESOLVED-PENDING-MERGE (corpus-bugfix 2026-07-17, per v-effects note): the check/compile
; divergence (main-direct handle declining "no machine representation") is FIXED in ce559d74a. Root:
; reduce_handle THREAD path returned the perform's resume value (seed k, a deep_fresh_copy) with NO
; lexical chain -> bare-name seed read UNBOUND -> Poison -> Any -> decline. Fix = reparent_under_handle_site
; on the threaded result (same anchoring E5/multi-shot use). k=9 -> 9, unit-test-gated. STACKED on
; v-effects' pending task-#15 MR (same-file effects.rs) — sends to pr-sync only AFTER #15 lands.
; LOCUS 2 (helper face (f k) -> bogus CDZ0101 unbound k) is DISTINCT (inline-scope bug, param-seed
; binding lost when a handle in a non-recursive helper is inlined) — v-effects split it to
; queue/adv-handle-in-inlined-helper-loses-caller-param-binding-cdz0101.sexp. Verify+promote Locus1 on land.
