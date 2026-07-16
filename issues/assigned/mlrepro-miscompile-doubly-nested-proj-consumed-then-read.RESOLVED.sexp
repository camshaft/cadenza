;; MISCOMPILE — SILENT WRONG VALUE (2026-07-15). The DOUBLY-NESTED face of the repeated-projection retain
;; miscompile (mlrepro-miscompile-repeated-proj-of-let-consumed-then-read.sexp). The DIRECT single-level
;; projection `(. t 0)` is FIXED (v-memory-safety, retain the projected child at a consuming nested-compound
;; Proj of a live binder). This DEEPER face is STILL OPEN: the consuming op's operand is a Proj-of-Proj
;; `(. (. t 0) 0)` — the list lives two projections deep — so `operand_is_binder` (the fix's gate, which only
;; matches a DIRECT LocalRef/Param operand) does not fire, and no child dup is emitted.
;;   main 3 => 8, want 7 (same off-by-one-high as the single-level case). cdz check CLEAN.
;; FIX DIRECTION: extend `mark_binder_dups`'s `Core::Proj` arm to walk a Proj CHAIN rooted at the binder
;; (through the intermediate BORROWING projections) and mark each consuming nested-compound leaf whose root
;; binder is live-after — then dup the innermost child. TERRITORY: v-memory-safety (Perceus dup placement).
;; Confirmed broken on clean trunk BEFORE the single-level fix (pre-existing, not a regression).
(do
  (def (build (: i Int64) (: n Int64) (: acc (Tuple (Tuple (List Int64) Int64) Int64)))
    (if (< i n)
      (build (+ i 1) n (tuple (tuple (List.push (. (. acc 0) 0) i) (. (. acc 0) 1)) (+ (. acc 1) 1)))
      acc))
  (def (main (: n Int64))
    (let ((t (build 0 n (tuple (tuple (list) 0) 0))))
      (+ (List.len (List.push (. (. t 0) 0) 99)) (List.len (. (. t 0) 0)))))
  (export main))
