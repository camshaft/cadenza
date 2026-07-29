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

;; THIRD FACE (breaker #41 third route, CONFIRMED corpus-bugfix trunk 2cb5af98f both backends): a bare
;; match on the withheld ctor NESTED INSIDE A GUARD condition also bypasses. (match (mk k) ((guard w
;; (match w ((T v) (> v 20)) (_ false))) 1) (_ -1)) -> COMPILES, reads private payload (value 1 at k=5,
;; since 50>20; wasm+rust). Same bare-pattern resolver presumably, but guard desugaring is its own lowering
;; -> distinct regression row. breaker: 3 doors total (direct, eval, guard-nested); ONE resolver choke-point
;; gate should clear all three, but the fix verification must SWEEP all three. ON FIX: gate all 3 rows x3
;; -> (error CDZ0214); pin all into 11-modules.

(case "a guard-nested BARE pattern on a withheld constructor is also rejected (encapsulation soundness, guard-desugar route)"
  (input  (do
        (import "temp" (Temp mk))
        (def (main (: k Int64)) (match (mk k) ((guard w (match w ((T v) (> v 20)) (_ false))) 1) (_ -1)))
        (export main)))
  (module "temp"
    (do
      (type Temp (T Int64))
      (def (mk (: c Int64)) (T (* c 10)))
      (export Temp)
      (export mk)))
  (call   main (: 5 Int64)) (error CDZ0214))

;; FIX BUILT (v-inference, 2026-07-29): commit 5059069d4 — gated at lower::pattern_constraints (the SHARED
;; match lowering), so BOTH the direct ((T v)) AND eval-quasiquote faces close in ONE place (verified via
;; 2-file harness: bare AND qualified both -> CDZ0214; rcdzc lib 2372/0). Shared-lowering choke-point should
;; ALSO close the guard-nested 3rd face (guard desugars to a nested match through the same lowering) — VERIFY
;; that row on land. HELD: v-inference's local full-gate times out under batch load; STACKS on CDZ0215.
;; SEQUENCE: my #label migration (queued) + this fix + CDZ0215 land stacked. ON LAND (5059069d4 on trunk):
;; gate all 3 rows (direct + eval + guard-nested) x3 -> (error CDZ0214); pin into 11-modules beside :1470.

;; 3-FACE VERIFIED (v-inference, 2026-07-29): the single lower::pattern_constraints gate closes ALL 3
;; faces — direct ((T v)) + eval-quasiquote + guard-nested — all -> CDZ0214 (2-file harness, rcdzc 2372/0).
;; Prediction confirmed: one resolver choke-point clears all 3 doors. FIX SHA now = 38c12a630 (amended
;; from 5059069d4 in the 3-face sweep). Still HELD on the CDZ0215 lockstep + a fresh-store gate.
;; ON LAND (38c12a630 on trunk): gate all 3 rows x3 -> (error CDZ0214); pin into 11-modules beside :1470.
