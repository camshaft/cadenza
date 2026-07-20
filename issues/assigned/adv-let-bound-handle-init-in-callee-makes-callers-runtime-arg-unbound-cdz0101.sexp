; adv repro (found by v-verification while probing @ensures-over-handle-body composition, 2026-07-20)
;
; BUG: a `let` whose INIT is a `handle` expression, in a callee's body, makes the CALLER's runtime
; argument spuriously CDZ0101 "unbound name". The unbound name is the CALLER's parameter (`k`), not
; anything in the callee — so this is a name-resolution / effects-lowering interaction, NOT a verification
; bug. It surfaced through v-verification's `@ensures` enforcement (which injects `(let ((ret BODY)) …)`),
; but reproduces with a HAND-WRITTEN `let`-over-handle and no annotation at all, so it is a lower-layer bug.
;
; ISOLATION (each row = one program; ✅ compiles, ❌ CDZ0101 `unbound name k`):
;   ✅  main const-arg  + callee `(let ((r (handle …))) r)`      — n2e: (def (main) (f 5))
;   ✅  main typed-arg  + callee handle DIRECTLY the body (no let) — g1: (def (f x) (handle …))
;   ❌  main typed-arg  + callee `(let ((r (handle …))) r)`       — THIS CASE
; So the trigger is the CONJUNCTION: (a) caller passes a runtime arg `(f k)` where `k` is a typed param,
; AND (b) the callee binds the result of a `handle` with a `let` and returns the bound var. Drop either the
; `let` (put the handle directly in body) or the runtime arg (use a constant) and it compiles + runs.
;
; The `let`-over-handle shape is EXACTLY what `verify_enforce` emits for `@ensures` on an effect-handling
; def, so this blocks `@ensures`/`@requires` (which also wraps the body) over any def whose body is a
; handle expression when called with a runtime argument. Reported to v-effects/v-inference via the PM.
; Likely owner: effects lowering or name-resolution (the caller-arg binding is dropped when the callee's
; body contains a let-bound handle init). NOT verification's lane.

(case "a let-bound handle-init in a callee's body spuriously makes the caller's runtime argument unbound (CDZ0101)"
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (f (: x Int64))
              (let ((r (handle St x ((tick (u) s (resume s (+ s 1)))) (St.tick))))
                r))
            (def (main (: k Int64)) (f k))
            (export main)))
  (call   main (: 5 Int64))
  (output (: 5 Int64)))

; ===== PM triage (corpus-bugfix, 2026-07-20, trunk e371e1d50) — VERIFIED, routed v-effects (cc v-inference) =====
; CONFIRMED live: the exact repro fails cdz-compile CDZ0101 "unbound name k" (caller param), cdz check CLEAN.
; Isolation re-verified: const-arg (f 5) COMPILES(5); handle-direct-body typed-arg COMPILES(5); only the
; conjunction [runtime-arg call + callee let-over-handle-init] fails. Same caller-arg-drop CLASS as the
; closure-payload β-copy (6e6d45b20 pin) + same-name-ctor β-copy (b42821408), but trigger is let-over-handle.
; Blocks @ensures/@requires over handle bodies (verify_enforce injects (let ((ret BODY)) …)). Routed v-effects
; (effects-lowering) primary, v-inference cc (resolution/β-copy angle). corpus-bugfix pins it (compiles+runs 5)
; once fixed — likely all-3-backend green like the other β-copy caller-arg-drop pin.

; SHARPENED (v-effects, 2026-07-20 investigation — corrects the locus):
; - The CDZ0101 fires BEFORE handle-lowering: a VEFF trace at lower.rs's Resolved::Handle arm shows
;   reduce_handle is NEVER REACHED for this repro. So it is NOT a reduce_handle fold bug — the `k` is
;   dropped EARLIER, during the (f k) call's processing (the inline/β-reduce of f's let-over-handle body).
; - ISOLATION (all confirmed):
;     no-let, handle-seed = runtime arg (f k)        → COMPILES + runs 5   (handle alone is fine)
;     let-bound value = (+ x 1), NO handle, (f k)     → COMPILES + runs 6   (let-in-inlined-callee alone is fine)
;     let-bound handle whose SEED = x, (f k)          → CDZ0101 unbound k   ← the bug (let + handle-runtime-seed)
;     let-bound handle, x in BODY (not seed), (f k)   → COMPILES           (only the SEED position drops)
;     f called TWICE (+ (f k) (f (+ k 1)))            → still CDZ0101       (not simple single-inline)
; - So the TRIGGER is precisely: [a `let` binding a handle whose SEED references a caller runtime arg] in a
;   callee body, processed at the call site. A handle-in-body without the let, or a let without a handle,
;   both work — only the conjunction drops the seed's caller-arg binding, and it happens in the f-inline /
;   effects-pre-reduction (reduce_applied_lambdas or the general apply β-reduce), NOT reduce_handle.
; - A single deep_fresh_copy of `init` inside reduce_handle did NOT fix it (consistent — reduce_handle isn't
;   even reached). The fix is at the f-body-inline site that copies a let-over-handle and loses the seed's
;   free var. LIKELY the effects-pre-reduction copies f's body (handle present → effects path) with a
;   copy that doesn't pin/preserve the seed's caller-arg. NEXT: trace reduce_applied_lambdas / the apply
;   β-reduce of a callee whose body is (let ((r (handle …seed=arg…))) r).
