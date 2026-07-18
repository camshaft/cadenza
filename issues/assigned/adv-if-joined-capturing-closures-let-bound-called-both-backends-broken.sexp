; BREAKER FINDING 2026-07-18 (trunk 5431f668a) — BOTH-BACKEND miscompile, the 4th face of the
; lambda-init speculative-lift family (3 fixed: Resolved::Lambda init, lambda_body-reducible init,
; compound-holding-lambda init `7fdb1dcb8`): a LET whose init is an IF-JOIN of two CAPTURING lambdas,
; then called:
;
;   (let ((f (if b (fn (x) (+ x n)) (fn (x) (* x n))))) (f 10))
;
;   wasm:  b=true -> TRAPS unreachable; b=false -> 0 (wrong value; want 50)     [n=5]
;   rust:  E0425 `__cap0` unbound x2 — the arms are β-inlined at the call site as
;          (10).checked_add(__cap0) / (10).checked_mul(__cap0) with no capture binding emitted.
;   Same-body variant ((+ x n) both arms): b=true traps, b=false -> 10 (payload n LOST — reads 0).
;
; CONTROLS (verified this base):
;   - joined NON-capturing lambdas, called          -> 11/20 ✓ (both arms correct)
;   - joined capturing lambdas in a RECORD FIELD,
;     projected + called                            -> 107/93 ✓ (the 7fdb1dcb8 fix covers compounds)
;   - single capturing lambda let-bound + called    -> ✓ (long-pinned)
;
; DIAGNOSIS (same disease as 7fdb1dcb8, new syntactic face): `should_keep_binding` classifies the
; let-init via `core_of(init)`; an IF-of-lambdas init speculatively LIFTS the closures (recording
; the capturing `n` occurrences in db.captured_ref), but the call site β-reduces an arm INLINE,
; reusing those poisoned occurrences -> rust emits `__cap0` reads with no env; wasm reads a
; nonexistent capture env (b=false's 0/10 = env slot read as 0; b=true's trap = the overflow guard
; on garbage). The fix's `compound_contains_lambda` short-circuit covers records/tuples but NOT an
; `if` whose ARMS are lambdas — the join face needs the same lift-free treatment (or the keep-path,
; since a genuinely-conditional closure can't fold to one lambda).
;
; SEVERITY: silent wrong value AND spurious trap on wasm; compile-fail on rust. The conditional
; function-choice idiom (`let f = if debug then verbose-fn else quiet-fn`) is common.
;
; Expected (n=5): b=true -> 15, b=false -> 50.
(case "an if-join of two capturing lambdas let-bound and called applies the selected closure"
  (doc    "`(let ((f (if b (fn (x) (+ x n)) (fn (x) (* x n))))) (f 10))` with `n` a def param —
           conditional closure choice, the runtime-select twin of the pinned single-lambda and
           compound-held-lambda bindings. b=true -> 10+n, b=false -> 10*n. Currently the 4th face of
           the speculative-lift family: wasm traps (true) / returns 0 (false, capture env lost), rust
           E0425 unbound `__cap0` (arms β-inlined without their capture bindings); the record-field
           and non-capturing joins are correct. n=5: 15 / 50.")
  (input  (do
            (def (main (: b Bool) (: n Int64))
              (let ((f (if b (fn ((: x Int64)) (+ x n)) (fn ((: x Int64)) (* x n)))))
                (f 10)))
            (export main)))
  (call   main (: true Bool) (: 5 Int64))
  (output (: 15 Int64))
  (call   main (: false Bool) (: 5 Int64))
  (output (: 50 Int64)))
