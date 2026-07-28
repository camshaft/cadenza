;; HELD PIN (corpus-bugfix, 2026-07-25) — do NOT land until v-rust-backend fixes the E0382.
;; Origin: breaker FINDING (inbox issue 000000016937). CONFIRMED on trunk 8b6a415c1 (reproduced by
;; corpus-bugfix): a MUTUALLY-RECURSIVE tree-fold that REBUILDS an Ast list and then matches it with a
;; PAYLOAD-binding list pattern ((list (Ast.Int a)) …) NO-BUILDS on BOTH rust targets with
;; error[E0382]: borrow of moved value `__ms19.0` (the match-scrutinee temp), while WASM compiles and
;; computes correctly (→ 102). A backend-divergent hard build-fail — cannot be baseline-tolerated.
;;
;; DISCRIMINATOR (breaker's 6-row matrix, corpus-bugfix-verified wasm=102/rust=E0382 on the repro):
;;   ✗ RECURSION × (list-pattern with a PAYLOAD binder over a rebuilt list) → E0382 __ms19.0
;;     (repros single-element (m4) + with a fresh fall-through value (m5) — the xs2 REUSE is NOT required)
;;   ✓ same nested match over a call result WITHOUT recursion (m3) → builds
;;   ✓ the recursion WITHOUT the payload-binding inner match (m6) → builds
;;   ✓ the inner match shape as a standalone fn on a param (m1) → builds
;;   ⇒ the trigger is RECURSION × payload-binding-list-match-on-a-rebuilt-list: the rust emit moves the
;;     matched list into the probe chain and then RE-BORROWS it (__ms19.0 = the scrutinee temp). Neither
;;     half alone breaks. Likely match-scrutinee move/borrow sequencing under recursive specialization.
;;   • SHARPENED then CORRECTED (breaker #25 perimeter, corpus-bugfix-verified on trunk 8b6a415c1):
;;     - a WILDCARD-element list pattern `(list _one)` — NO payload binder — ALSO E0382 (payload binder
;;       not required); a rebuilt-TUPLE literal probe `(tuple 1 kk)` under the same recursion BUILDS.
;;     - CORRECTION (breaker, later): the list pattern is NOT required either — `(match (List.at xs2 0)
;;       ((Some (Ast.Int a)) …) (_ …reuse xs2…))` (Option-payload destructure + sibling-arm xs2 reuse, NO
;;       list pattern) ALSO E0382s (corpus-bugfix-verified: wasm 102 / rust E0382). And a len-guard+at
;;       WORKAROUND that destructures the at-results and reuses xs2 in a sibling arm ALSO breaks. But
;;       len-read + reuse alone BUILDS, and len-read only BUILDS.
;;     ⇒ CORRECTED discriminator: under recursion, a match whose arm DESTRUCTURES A PAYLOAD DERIVED FROM
;;       xs2 (list-pattern probe OR List.at Option) moves xs2's temp into the probe chain, and a SIBLING
;;       arm's xs2 REUSE then sees the moved value. Likely TWO related move-points: (a) list-pattern probe
;;       (moves even without a sibling reuse — the m5 no-reuse row); (b) derived-payload-destructure +
;;       sibling reuse. BUILD workaround: read len/at into SCALARS first, no payload destructure in the
;;       same match as the reuse. v-rust-backend to test both shapes.
;; IMPACT: the natural bottom-up rewrite pass (constant-fold / simplify / rename with element-payload
;;   dispatch) — the exact shape of a compiler-in-Cadenza pass — cannot target rust today; binder-free or
;;   non-recursive spellings work. OWNER: v-rust-backend (rcdzc rust emit, scrutinee move/borrow).
;; ORACLE: wasm computes → 102; rust MUST match (→ 102) once the E0382 is fixed.
;; ON LAND (v-rust-backend's fix on trunk): rebuild cdz; gate the case x3 (→ 102, rust now BUILDS);
;;   pin into 20-structural-editing.sexp (the rewrite-pass family) beside the recursive Ast-walk cases;
;;   baseline x3; roundtrip + silent-omission + --check; MR; notify v-rust-backend + breaker.

(case "a mutually-recursive fold matching a rebuilt list with a payload binder builds and computes"
  (input (do
        (def (fold node)
          (match node
            ((Ast.List xs)
              (match (fold-list xs (list) 0)
                ((tuple xs2 k)
                  (match xs2
                    ((list (Ast.Int a)) (tuple (Ast.Int a) (+ k 1)))
                    (_ (tuple (Ast.List xs2) k))))))
            (other (tuple other 0))))
        (def (fold-list (: xs (List Ast)) (: acc (List Ast)) (: k Int64))
          (match xs
            ((list) (tuple acc k))
            ((list h .. t)
              (match (fold h)
                ((tuple h2 k2) (fold-list t (List.push acc h2) (+ k k2)))))))
        (def (main (: n Int64))
          (match (fold (Ast.List (list (Ast.Int (BigInt.of n)))))
            ((tuple _r k) (+ (* k 100) n))))
        (export main)))
  (call main (: 2 Int64)) (output (: 102 Int64)))
