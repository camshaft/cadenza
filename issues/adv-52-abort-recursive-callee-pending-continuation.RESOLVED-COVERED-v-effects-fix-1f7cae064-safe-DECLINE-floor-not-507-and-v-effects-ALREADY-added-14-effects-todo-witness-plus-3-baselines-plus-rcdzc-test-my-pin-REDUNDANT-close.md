; adv-52 (breaker tick 1094) — SOUNDNESS MISCOMPILE on ALL THREE targets: an ABORTIVE perform
; reached through a RECURSIVE callee, with a PENDING continuation in the handle body, does NOT
; abandon — the abortive arm's value flows back INTO the pending continuation as if it RESUMED.
;
; Observed (trunk 59aebb94f, wasm + rust + rust-async identical):
;   (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (go 2) 999999)) 7)
;   where (def (go n) (if (= n 0) (Mx.bail 5) (go (- n 1))))
;   EXPECTED: bail aborts -> handle value = 500; main = 507
;   GOT:      1000506 = 500 + 999999 + 7  — the pending (+ _ 999999) RAN with the arm value.
;
; SHRINK MATRIX (tick 1094, /tmp/breaker-shrink2):
;   s2 abort DIRECT in body + pending add            -> 507 CORRECT (abandons)
;   s1 abort via NON-recursive callee + pending add  -> 507 CORRECT
;   s3 abort via recursive callee, NO pending        -> 507 CORRECT
;   s4 abort via RECURSIVE callee + pending add      -> 1000506 WRONG (all 3 targets)  <- THIS
;   s5 s4 + accumulator arg                          -> WRONG (same shape)
; Trigger = RECURSIVE callee containing the abortive perform × PENDING continuation in the handle
; body. The recursive-specialization path apparently compiles the abortive op like a TAIL-RESUMING
; one (the arm value becomes the callee's return value and the pending continuation consumes it)
; instead of br-ing out of the handle block. NOT a decline — accept+wrong-value, the miscompile
; class. Note the fresh 59aebb94f mixed-arm pins cover the DIRECT-in-body abort (my s2 twin) —
; this is the recursive-callee face those pins don't reach.
;
; Severity HIGH: silent wrong value on every backend; the early-exit-from-a-recursive-walk idiom
; (search that bails on found, validation that bails on first error) is exactly this shape.

(case "an abortive perform in a recursive callee abandons the pending continuation in the handle body"
  (doc    "The abort must escape the whole handle body: `go` recurses to the abortive `Mx.bail 5`,
           whose arm yields 500 as the HANDLE's value — the pending `(+ _ 999999)` is abandoned →
           507. Today all three targets wrongly RESUME the pending add with the arm value (1000506).
           Graded against the SPEC; red until fixed.")
  (input  (do
            (effect Mx (op bail (-> Int64 Int64)))
            (def (go (: n Int64)) (if (= n 0) (Mx.bail 5) (go (- n 1))))
            (def (main)
              (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (go 2) 999999)) 7))
            (export main)))
  (call   main) (output (: 507 Int64)))
