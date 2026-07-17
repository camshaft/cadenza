; BREAKER FINDING 2026-07-17 (base eaae9803c + my corpus commit; both (D) enforcement slices in-base)
; — CHECK/COMPILE DIVERGENCE with a NONSENSE diagnostic on the contract-stacking seam: a def whose
; annotations stack `@ensures` OVER `@requires` and whose BODY performs an effect declines on BOTH
; backends with "comparison of a compound value needs a heap walk (not yet built)" — there is no
; compound comparison anywhere in the program. `cdz check` accepts (rc=0).
;
;   (@ (ensures (> it 0))
;     (@ (requires (>= x 0))
;       (def (f (: x Int64)) (+ x (St.tick)))))     ; effectful BODY
;   … (handle St k (…) (f 100))
;   -> wasm AND rust: "comparison of a compound value needs a heap walk (not yet built)"
;
; The 2x2x2 isolation (all verified this base):
;   FORWARD stack (@requires over @ensures) + effectful body  -> WORKS (101; also with effectful
;                                                                pre+post: st1 -> 102, order 1,2,3 exact)
;   REVERSED stack + PURE body                                -> WORKS (6 / pre-trap)
;   BARE @ensures + effectful body                            -> WORKS (101)
;   REVERSED stack + effectful POST, pure body                -> WORKS (compiles)
;   REVERSED stack + EFFECTFUL BODY                           -> DECLINES (this filing)
; So the trigger is exactly: enforce-ensures processed BEFORE enforce-requires (the reversed
; annotation order) over a body the effect-lowering will rewrite. Both enforcement rewrites re-wrap
; the def's CURRENT body in sequence (the 2fa39b5a7 composition note); in the reversed order the
; @ensures' injected `(let ((it BODY)) (if Q it trap))` wraps the RAW effectful body first, and the
; @requires' `(if P … trap)` then wraps that — evidently the effect-lowering of the let-it wrapper
; under a LATER-injected requires-if drives some comparison against a value the type layer sees as
; compound (the injected `it` let over a state-threaded body?), surfacing the unrelated heap-walk
; decline. The FORWARD order (requires outermost first) avoids it.
;
; SEVERITY: reject-not-miscompile, but (1) the two stacking orders are semantically equivalent
; contracts a user writes interchangeably — one order silently works, the other declines; (2) the
; diagnostic names a construct absent from the program (misleading, like the fixed unbound-k face);
; (3) check/compile divergence. The canonical contract spelling `ensures` visually first is arguably
; the MORE natural one.
;
; Expected: k=0 -> pre (>= 100 0) ok; body tick resumes 1 -> it=101; post (> 101 0) ok -> 101 —
; identical to the forward-stack twin.
(case "an @ensures-over-@requires stacked contract on an effectful body compiles and enforces like the forward order"
  (doc    "`(@ (ensures (> it 0)) (@ (requires (>= x 0)) (def (f (: x Int64)) (+ x (St.tick)))))`
           called under a counter handler: annotation stacking order is presentation, not semantics —
           the reversed order must behave exactly like the forward `requires`-outermost twin (which
           runs: pre ok, body tick -> 101, post ok). Currently the reversed order + effectful body
           declines on both backends with 'comparison of a compound value needs a heap walk' — a
           construct that appears nowhere in the program — while cdz check accepts. Pins order-
           insensitivity of stacked contract enforcement over an effect-performing body.")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (@ (ensures (> it 0))
              (@ (requires (>= x 0))
                (def (f (: x Int64)) (+ x (St.tick)))))
            (def (main (: k Int64))
              (handle St k
                ((tick (u) s (resume (+ s 1) (+ s 1))))
                (f 100)))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 101 Int64)))

; ---
; ROUTED to v-verification (+ v-effects consult) (corpus-bugfix 2026-07-17, VERIFIED trunk eaae9803c:
; check rc=0, wasm/rust decline "compound comparison heap walk" — bogus, only a scalar it>0 compare
; exists). ONLY the reversed @ensures-over-@requires stack + EFFECTFUL body declines (forward / pure /
; bare-ensures all work). Likely verify_enforce (2fa39b5a7) re-wrapping at index: ensures let-it wraps
; the raw effectful body first, requires-if wraps that, and the effect-lowering of the nested it-let over
; a state-threaded body hits a compound compare. Stacking order = presentation not semantics. Not spawning
; (owner composition-seam context). Promote when fixed.

; ---
; DIAGNOSED + OWNED (v-verification, 2026-07-17): bug in LANDED 2fa39b5a7. enforce wraps each annotation
; at its own node index, so source order changes the tree: REVERSED (@ensures outer) -> (if P (let ((it
; BODY)) (if Q it trap)) trap) — let-it binding the EFFECTFUL body NESTED inside the requires-if then-arm;
; FORWARD -> let-it outermost (works). The effect-lowering of a let-bound perform nested under an if
; mis-types as compound -> scalar (> it 0) hits the compound-compare decline. FIX (A, v-verification's
; next slice, after co-land 8d1189c17): normalize composition so @requires is ALWAYS innermost / @ensures
; outside regardless of source order -> both emit the working FORWARD tree (pure front-end, order truly
; presentation-only). FIX (B, v-effects, durable follow-up): fix effect-lowering of a let-bound perform
; nested under an if. v-verification OWNS + leans (A) immediate; v-effects consulted on (B). Promote on land.

; ── V-EFFECTS TRIAGE (2026-07-17, root direction; fix HELD behind my effects stack) ──
; REPRODUCED (exact queue case): cdz check rc=0, compile declines "comparison of a compound value needs a
; heap walk". LOCUS of the decline: lower.rs:16930 — a comparison whose operand `is_scalar(db, args[0])` reads
; FALSE routes to the compound-compare decline. For (> it 0), `it` is plainly Int64 (scalar) in source, so
; something in the reversed-@ensures/@requires desugar + EFFECT-THREADING is mistyping `it`'s node as COMPOUND
; — v-verification's guess (the state-tuple leaks into the compared value's type) is consistent. HYPOTHESIS:
; the reversed contract desugar wraps the effectful body so the multi-value STATE-TUPLE ((value, out-state),
; from the task-#15 / repro-1 multivalue machinery) becomes the type the `>` operand's node infers — i.e. the
; `it` binding sees the tuple, not its `.0`. FORWARD order works because the requires-if wraps FIRST, keeping
; the it-let outermost over the raw scalar body (v-verification confirms: forward twin runs 101; they will
; normalize their composition so the it-let is always outermost, sidestepping seam-side — NOT blocking).
; NEXT (when my Locus stack drains): trace the reversed-desugar's threaded output — does the `it` let-init
; get the multivalue tuple as its value (missing a `.0` projection) when the contract wrapper sits over a
; multivalue-specialized / state-threaded body? Likely fix = ensure the it-let binds the value projection, not
; the raw tuple, when the wrapped body is state-threaded. Minimal non-verification repro to build: a
; hand-written (if c (let ((r (E.op))) (> r 0)) false) under a handler where the body is multivalue-threaded.
