; adv-66 (breaker, 2026-08-03, HIGH wasm soundness — OOB memory fault, the adv-54 kept-binding
; family, Bytes.compact face): a let-bound Bytes.compact result read TWICE (an = then an order-
; compare via Bytes.concat) memory-faults on wasm; rust computes 11 for the same program.
;
; observed (trunk 4ca6470d0, differential-confirmed via gate):
;   n=10: wasm 'out of bounds memory access' (fault addr past linear memory); rust 11.
;   n=2:  wasm 'unreachable' trap; rust 11. Deterministic run-to-run. n=3/5 via the CONST driver
;   (cdz run) pass — the shrink is NOT monotone in n on the const path, but the GATE (runtime n)
;   faults at BOTH n=2 and n=10, so the runtime-arg path is uniformly broken.
; single reads are fine: compact+len alone (100), eq(rope,flat) alone (1), order-compare alone
;   (1) — the trigger is the SECOND read of the SAME let-bound compact result, where the first
;   read's consume freed/moved the leaf and the second read (feeding Bytes.concat) chases it.
; root shape: adv-54's is_runtime_computation keep-list added StrSlice/StrToBytes but the commit
;   note documented "the other Bytes/List/... heap ops share the SAME latent copy-propagate shape
;   but forcing them kept regressed 3 cases — their kept-binding EMIT has its own bug (follow-up)".
;   Bytes.compact is exactly that deferred family: the binding is copy-propagated, the compact
;   re-runs per read, and the recompute consumes the shared rope source -> second read OOB.
; expected: 11 (eq true; rope < flat+"B" true) at every n, both backends.
(case "adv-66 a let-bound Bytes.compact result read twice computes on wasm (no OOB)"
  (input  (do
            (def (build-rope (: n Int64) (: acc Bytes))
              (if (> n 0) (build-rope (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (let ((rope (build-rope n (Bytes.of (list)))))
                (let ((flat (Bytes.compact rope)))
                  (+ (if (= rope flat) 1 0)
                     (* 10 (if (< rope (Bytes.concat flat (Bytes.of (list (UInt8.wrap 66))))) 1 0))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 11 Int64))
  (call   main (: 2 Int64)) (output (: 11 Int64)))

; --- TRIGGER NARROWING (breaker sweep, trunk 107be31f0) — 5 near-identical siblings all PASS ---
; The fault needs BOTH reads in exact roles: (1) eq(rope, flat) THEN (2) an ORDER-compare whose RIGHT
; operand is concat(flat, …) with ROPE on the LEFT. Siblings that PASS (perimeter pins post-fix, breaker
; fam1-6): concat-result double-read (11); slice-VIEW double-read (11); List.concat 3-way (1073); compact
; + TWO order-compares no eq (11); compact eq-then-LEN (101); compact eq-then-concat+len WITHOUT rope-on-left
; compare (111). So it's the ROPE's SECOND deep-walk (order-compare left operand) after an eq against its
; own compact that faults — not flat's. breaker reads this as consistent with the compact-copy-propagate
; root (recomputed compact consumes rope on read 1, read 2 walks freed rope); v-runtime's dx is a Perceus
; dup/drop miscount on the kept binding. Two hypotheses for v-wasm-opt to reconcile — both point at the
; ROPE deep-walk-after-eq-consume seam. Perimeter siblings ready as pins once the shape converges.
