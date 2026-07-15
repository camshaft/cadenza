;; WITNESS (2026-07-15, v-inference) — the MUTUAL-RECURSION face of the recursive-generic-producer
;; element-tie gap (sibling of mlrepro-decline-recursive-generic-producer-drops-element-tie): a
;; MUTUALLY-recursive generic PRODUCER pair declines even at a SINGLE instantiation, where the
;; SELF-recursive single-instantiation producer WORKS (fixed by the A+B list-pattern shaping, landed).
;;
;; `toa`/`tob` build a `List (T a)` from a `List a`, wrapping each element and bouncing between the two
;; defs. `cdz type toa` → `(-> (List _) (List (T _)))` — the result element `_` DISCONNECTED from the
;; argument element `_` (same signature shape as the self-recursive `from-list : List a -> Iter a`). But
;; unlike self-recursion, this declines at ONE instantiation (Int64 only), consumed by `cnt`:
;;   (cnt (toa (list n n n)))   →   CDZ0201 "a generic type argument is undetermined" (at monomorphization)
;;
;; SHARPER SCOPE than the self-recursive sibling: self-recursion at a SINGLE element type COMPILES+RUNS
;; (the landed A+B fix — the recursive self-call `(from-list t)` flows the element tie via
;; `apply_scheme_to_args`'s generic-scheme seed-skip). MUTUAL recursion breaks the tie even single-
;; instantiation because the cross-call `(tob t)` / `(toa t)` is to a DIFFERENT def whose scheme is itself
;; being solved — the mutual group's schemes are computed with no shared substitution between them, so
;; neither cross-call carries the element tie back. So the mutual-recursion producer is a STRICTLY harder
;; case: it fails where the self-recursive one now succeeds.
;;
;; SAME ROOT + FIX as the self-recursive sibling: `compute_def_scheme` types param + result via separate
;; `type_of` with no shared subst, and the result element var is never tied to the param's. The
;; `apply_type` seed-skip-freshen lever (tick 9/10 attempts) fixes the SELF-recursive ≥2-inst case but has
;; NO clean local discriminator (freshen `(None)`/`Map.empty` placeholders vs preserve determined args
;; needs VAR PROVENANCE = shared-subst/real-HM — see recursive-generic-producer-element-tie-gap memory).
;; The mutual case additionally needs the group's schemes solved under a SHARED subst (a real fixpoint over
;; the mutual-recursion group), so it is gated on the same real-HM work.
;;
;; `cdz check` PASSES; the decline is at LOWERING/monomorphization. WORKAROUND: a monomorphic `T` over a
;; fixed element type (the same workaround the self-recursive sibling ships). Promote to the graded corpus
;; when the tie lands.
(do
  (type T (A a) (B a))
  (def (toa xs) (match xs ((list) (list)) ((list h .. t) (List.push (tob t) (T.A h)))))
  (def (tob xs) (match xs ((list) (list)) ((list h .. t) (List.push (toa t) (T.B h)))))
  (def (cnt xs) (match xs ((list) 0) ((list h .. t) (+ 1 (cnt t)))))
  (def (main (: n Int64)) (cnt (toa (list n n n))))
  (export main))
