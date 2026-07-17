; BREAKER FINDING 2026-07-17 (trunk 1c255812b) — BOTH-BACKEND emit bug: a CAPTURING closure stored
; in a LET-BOUND compound (record OR tuple), projected out and called, produces a BROKEN artifact:
;   wasm       -> compiles, but the component is INVALID: `failed to compile: wasm[0]::function[2]`,
;                 wasm-tools: "func 2 failed to validate: type mismatch: expected i32, found i64".
;                 The emitted main does `local.get 0 (the i64 param); i32.const 1; call arr-get` —
;                 it passes the CAPTURED i64 VALUE where the capture-record's i32 heap HANDLE
;                 belongs in the arr-get(env, idx) read.
;   rust       -> compiles to Rust that names an UNBOUND `__cap0`: rustc E0425
;                 (`(10).checked_add(__cap0)` in main with no `__cap0` anywhere) — the closure was
;                 inlined at the call site but its capture environment binding was never emitted.
;   rust-async -> same unbound `__cap0`.
; Same failure at O0/O1/O2/O3 (not an opt-pass artifact).
;
; MINIMAL: (let ((r (record (f (fn (x) (+ x n)))))) ((. r f) 10))  with n a def param.
; The three boundary controls all WORK (each returns 11 for n=1):
;   - direct let-bound capturing closure, no compound:  (let ((g (fn (x) (+ x n)))) (g 10))
;   - INLINE compound (not let-bound): ((. (record (f (fn (x) (+ x n)))) f) 10)   [beta-reduced away]
;   - NON-capturing closure in a let-bound record: (let ((r (record (f (fn (x) (+ x 1)))))) ((. r f) n))
; So the trigger is exactly: closure-with-captures + stored in a let-bound compound + projected + called.
; Also reproduces with a TUPLE ((. r 0)), with the capture being a LET-LOCAL instead of a param, with an
; extra plain field in the record, and when the projection is called twice.
;
; Corpus gap: 09-functions.sexp pins a function stored in a tuple/record and called — but only a
; NON-capturing (fn (x) (+ x 1)) built INLINE; the capturing + let-bound face was never covered.
;
; Expected: main(1) = 10 + 1 = 11 on all backends (as the controls do).
(case "a capturing closure stored in a let-bound record is projected and called"
  (doc    "`(let ((r (record (f (fn (x) (+ x n)))))) ((. r f) 10))` — the closure captures the def
           parameter `n`, is stored in a record field, the record is let-bound, and the projected
           function is applied. Must behave exactly like the inline-record and direct-let controls
           (the call sees the capture): n=1 -> 11. Currently the wasm emit passes the captured i64
           where the capture-env i32 handle belongs (invalid module, func 2 type mismatch) and the
           rust emit references an unbound `__cap0` (E0425) — a broken artifact on every backend,
           every opt level.")
  (input  (do
            (def (main (: n Int64))
              (let ((r (record (f (fn (x) (+ x n))))))
                ((. r f) 10)))
            (export main)))
  (call   main (: 1 Int64))
  (output (: 11 Int64)))
