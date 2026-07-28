;; READY-TO-LAND (corpus-bugfix, 2026-07-24): a nested-match extension of the ill-typed-scrutinee
;; taint pin (queued 54dea47f5 / PR#849 NMatchCtor guard). Verified PASS x3 on trunk 338c1ec13
;; (wasm/rust/rust-async all → CDZ0203). Uncovered: the corpus pins a TOP-LEVEL ill-typed ctor-match
;; scrutinee, but not one ONE LEVEL DOWN (an inner match whose scrutinee is ill-typed, nested inside a
;; well-typed outer arm) — this pins that the TErr taint propagates UP from a nested match, not just at
;; the top level.
;;
;; STAGED (not committed) to keep my MR stack at 4 while pr-sync gates batch #124. ON my stack draining
;; (54dea47f5 ill-typed-scrutinee pin landed), land this beside it in 05-compound-types.sexp:
;;   1. rebuild not needed (corpus-only); insert the case after the top-level ill-typed-scrutinee case.
;;   2. baseline pass x3 (single-line edit, no --save reorder).
;;   3. gate x3 + roundtrip + --check 0-regression + silent-omission sweep, then MR.
;; Independent of any fix — lands whenever convenient.

(case "an ill-typed inner-match scrutinee taints the outer match"
  (doc    "The nested face of the ill-typed-scrutinee taint: the OUTER match `(match (Some 1) ((Some x) …)
           ((None) 0))` is well-typed and selects its first arm, but that arm's body is itself a match
           whose SCRUTINEE `(Some (+ 1 true))` is ill-typed (`(+ 1 true)` mixes Int64 and Bool). The inner
           TErr must taint the inner match, which taints the arm body, which taints the whole outer match
           → CDZ0203. Pins that the 'any TErr propagates' discipline for a ctor-match scrutinee holds at
           NESTED depth, not just the top level — an ill-typed subtree cannot be laundered by being buried
           one match deeper inside a well-typed outer arm. Companion of the top-level ill-typed-scrutinee
           taint case (the PR#849 NMatchCtor reference-backend guard).")
  (input  (do
            (def (main)
              (match (Some 1)
                ((Some x) (match (Some (+ 1 true)) ((Some y) y) (_ 0)))
                ((None) 0)))
            (export main)))
  (error  CDZ0203))
