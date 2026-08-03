; adv-64 (breaker, 2026-08-03, MED wrong-VALUE-FORM — const-vs-runtime divergence in the rendered
; TYPE; follow-up to the #1542 adv-63b fix): a scalar-erased newtype value escaping a PARAMETERIZED
; def now renders with the ERASED inner type, dropping the declared nominal — while the NULLARY
; (const) path keeps it. The #1542 fix note claims "Scalar newtype escapes+renders (: 5 W) like
; nullary" — contradicted by test.
;
; observed (trunk 2dbf91fb9, wasm):
;   (def (main) (Mk 5))            -> (: 5 W)                    nullary CORRECT
;   (def (main (: k Int64)) (Mk k)) -> (: 5 Int64)               param'd WRONG (nominal dropped)
;   DES Duration face: (def (main (: n UInt64)) (secs n)) -> (: 5000000000 UInt64) not Duration
; expected: the declared nominal in both paths — the corpus's own 27-DES:26 pin ("a task never
; handles a bare UInt64") renders (: 5000000000 Duration) and is only nullary by accident.
; rust: renders bare 5 for BOTH paths (no type label in its value form) so no rust divergence —
; this is the wasm value-form boundary only, but it IS a const-vs-runtime divergence within wasm.
; root shape: #1542's scalar value-form fallthrough boxes the bare Ty::Int result — the value-form
; TEMPLATE is built from the ERASED scalar type instead of the pre-erasure Ty::Nominal.
(case "adv-64 a scalar-erased newtype escaping a parameterized def renders its NOMINAL type like the nullary path"
  (input  (do
            (type Duration (Duration UInt64))
            (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
            (def (main (: n UInt64)) (secs n))
            (export main)))
  (call   main (: 5 UInt64)) (output (: 5000000000 Duration)))

; --- SCOPE-SHARPENING (breaker addendum, confirmed one-branch-local) ---
; A COMPOUND (heap-payload) newtype escaping a param'd def renders CORRECTLY on BOTH paths:
;   (type P (P (Tuple Int64 Int64))) param'd -> (: (tuple 5 6) P), same as nullary (nominal kept).
; So the nominal-drop is EXACTLY the SCALAR fallthrough branch #1542 added (boxed bare-Ty::Int); the
; compound/recursive-sum escape route passes the nominal fine. One-branch fix: the scalar value-form
; template needs the pre-erasure Ty::Nominal, not the stripped scalar. Corpus value-form convention
; (nominal-in-TYPE-slot, erased payload as value) confirms this pin's (: 5000000000 Duration) is right.
