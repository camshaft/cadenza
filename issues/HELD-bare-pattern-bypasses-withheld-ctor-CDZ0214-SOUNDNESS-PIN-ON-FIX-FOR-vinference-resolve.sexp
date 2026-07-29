; FINDING (breaker, 2026-07-29): the abstract-type (ADT) encapsulation is BYPASSED by a BARE
; constructor pattern. The QUALIFIED match `((C.A n) ...)` on a withheld ctor rejects CDZ0214
; (pinned at 11-modules :1470); the BARE spelling `((T v) ...)` of the SAME match COMPILES and
; reads the private payload — both backends (50 = the smart ctor's scaled internals at k=5).
;
;   module temp: (type Temp (T Int64)) (def (mk c) (T (* c 10))) (export Temp) (export mk)
;     — bare handle export = ABSTRACT; ctor T withheld.
;   importer: (match (mk k) ((T v) v) (_ -1))   -> 50  (!!)  should be CDZ0214
;   importer: (match (mk k) ((Temp.T v) v))     -> CDZ0214 ✔ (the :1470 pin)
;
; The bare-pattern resolver looks up the ctor in the scrutinee TYPE's variant set without the
; per-name visibility check the qualified selector path applies. Soundness of the smart-
; constructor discipline is gone: any importer reads (and by rebuild, forges) invariant-bearing
; internals with one spelling change. The :1470 doc even says "Soundness was always intact" —
; true only for the qualified path.
;
; GRADED REPRO (expected = the reject; FAILS today with value 50):
(case "a BARE pattern on a withheld constructor is rejected like the qualified spelling"
  (input  (do
        (import "temp" (Temp mk))
        (def (main (: k Int64)) (match (mk k) ((T v) v) (_ -1)))
        (export main)))
  (module "temp"
    (do
      (type Temp (T Int64))
      (def (mk (: c Int64)) (T (* c 10)))
      (export Temp)
      (export mk)))
  (error  CDZ0214))

;; ------------------------------------------------------------------------------------------
;; TRIAGED-CONFIRMED (corpus-bugfix, trunk 514ef49d0, BOTH backends): SOUNDNESS hole confirmed.
;;   BARE ((T v)) pattern -> COMPILES (143 bytes), reads private payload 50 (wasm+rust). BUG.
;;   QUALIFIED ((Temp.T v)) -> correctly rejects CDZ0214 "constructor `T` is not exported to this
;;     file ... cannot be constructed or matched through `T`". (the :1470 pin)
;; ROOT (breaker, confirmed plausible): the bare-pattern resolver looks up the ctor in the scrutinee
;;   TYPE's variant set WITHOUT the per-name visibility gate the qualified selector path applies.
;;   The qualified path (Temp.T) routes through the withheld-ctor check; the bare (T) path doesn't.
;; SEVERITY: encapsulation SOUNDNESS — the smart-constructor / ADT discipline (:1231-1245 'MUST NOT
;;   construct or match') is bypassed by one spelling change; load-bearing for the verification kernel's
;;   trust story (HOL Thm/Term shape cited at :1470). PRIORITY-WORTHY per breaker.
;; OWNER: v-inference (rcdzc resolve — pattern-resolution path missing the withheld-ctor gate).
;; NOTE: breaker's original case lacked a (call main ...) so it graded `todo` (0-arg export). Fixed here
;;   with (call main (: 5 Int64)) so it's a proper graded (error CDZ0214) reject case.
;; ON FIX (v-inference adds the visibility gate to the bare-pattern path): gate x3 -> (error CDZ0214);
;;   pin into 11-modules beside the :1470 qualified-reject pin; baseline x3 (a correct reject keys `pass`).

(case "a BARE pattern on a withheld constructor is rejected like the qualified spelling (encapsulation soundness)"
  (input  (do
        (import "temp" (Temp mk))
        (def (main (: k Int64)) (match (mk k) ((T v) v) (_ -1)))
        (export main)))
  (module "temp"
    (do
      (type Temp (T Int64))
      (def (mk (: c Int64)) (T (* c 10)))
      (export Temp)
      (export mk)))
  (call   main (: 5 Int64)) (error CDZ0214))

;; SECOND FACE (breaker #41 second route, CONFIRMED corpus-bugfix trunk 514ef49d0 both backends): the
;; EVAL/metaprogramming path ALSO bypasses the withheld-ctor gate. (eval (quasiquote (match (unquote
;; (mk k)) ((T v) v) (_ -1)))) in the importer -> COMPILES (143 bytes), reads private payload 50 (wasm+rust).
;; So the fix's gate must sit where BOTH the direct resolver AND eval's quasiquote-reconstructed match
;; consult ctor visibility (or eval reuses the fixed resolver and inherits it — a regression row either way).
;; ON FIX: gate BOTH the direct AND eval rows x3 -> (error CDZ0214); pin both into 11-modules.

(case "an eval-reconstructed BARE pattern on a withheld constructor is also rejected (encapsulation soundness, metaprogramming route)"
  (input  (do
        (import "temp" (Temp mk))
        (def (main (: k Int64)) (eval (quasiquote (match (unquote (mk k)) ((T v) v) (_ -1)))))
        (export main)))
  (module "temp"
    (do
      (type Temp (T Int64))
      (def (mk (: c Int64)) (T (* c 10)))
      (export Temp)
      (export mk)))
  (call   main (: 5 Int64)) (error CDZ0214))
