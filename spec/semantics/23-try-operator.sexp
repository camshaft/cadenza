; The `?` / `try` fallible short-circuit operator — witnesses DESIGN-try-operator-rcdzc.md. `(try e)` is
; the canonical s-expr form of the ML postfix `e?`: on the success variant it UNWRAPS the payload
; (`Some a` / `Ok a` → `a`), and on the failure variant it SHORT-CIRCUITS the enclosing fallible boundary
; (the enclosing function's `Result`/`Option` result type), making the boundary's value the failure
; itself. It is NOT a monad — it desugars onto the effects system's within-function abortive lowering (a
; synthesized `Mir::Block` + `Mir::Break`), so it adds no user-visible effect and nothing to the effect
; row. See README.md for the case vocabulary.
;
; STAGE STATUS. T0a (landed): `(try e)` is carried first-class through resolve/infer — its type is the
; operand's success payload, an operand that is not a fallible sum is CDZ0203, and a wrong-arity `(try …)`
; is CDZ0201. The BOUNDARY check (a `?` with no enclosing `Result`/`Option` function → CDZ0230) and the
; function-boundary DESUGAR (a value executing through wasmtime, both the happy and the short-circuit
; path) are the next slices; until the desugar lands a well-formed `(try e)` DECLINES (scored *todo* by
; the gate, never a miscompile). The executing cases below are the ones that will matter most once T1
; lands — a value must come out the far side, since `?` is control.

; ── T0a: rejections (these PASS today) ──────────────────────────────────────────────────────────────

(case "a `?` on a non-fallible operand is a type error"
  (doc    "`(try 5)` — a `?` on a plain `Int64` has nothing to unwrap. The operand of `?` must be a
           fallible `Result`/`Option`; anything else is the ordinary type mismatch CDZ0203, anchored at
           the operand. Pins the operand-shape half of the `?` type rule
           (DESIGN-try-operator-rcdzc.md §5): `?` unwraps a fallible sum, so a non-sum operand has no
           well-typed result and is rejected rather than run.")
  (input  (do (def (main) (try 5)) (export main)))
  (error  CDZ0203))

(case "a `?` on a String operand is a type error"
  (doc    "`(try \"hi\")` — the String companion of the `(try 5)` case: a `?` on any definite
           non-fallible type is CDZ0203. Pins that the operand-shape check is by the sum's VARIANT set
           (Some/None, Ok/Err), not by the operand happening to be a number.")
  (input  (do (def (main) (try "hi")) (export main)))
  (error  CDZ0203))

(case "`try` takes exactly one operand — a zero-operand form is malformed"
  (doc    "`(try)` has nothing to unwrap — `try` is a fixed one-operand form (like `not`/`quote`), so a
           zero-operand `(try)` is malformed, CDZ0201. Pins the arity check of `resolve_try`.")
  (input  (do (def (main) (try)) (export main)))
  (error  CDZ0201))

(case "`try` takes exactly one operand — a two-operand form is malformed"
  (doc    "`(try (Ok 1) (Ok 2))` supplies a surplus operand; `try` takes EXACTLY one (the fallible
           expression it unwraps), so it is malformed, CDZ0201, with the surplus-delete fix path shared
           with `quote`. Pins the upper arity bound of `resolve_try`.")
  (input  (do (def (main) (try (Ok 1) (Ok 2))) (export main)))
  (error  CDZ0201))

; ── T1 target: executing cases (DECLINE → *todo* until the boundary desugar lands) ──────────────────
; These are the operator's actual ask — the nested-`match`-collapse shapes, run through wasmtime. They
; are recorded here now so the gate pins the intended VALUE; the current generation declines them (the
; desugar is the next slice), which the differential gate scores as *todo*, not disagreement. When T1
; lands they flip from todo to pass with no corpus edit.

(case "`?` on the success variant unwraps the payload (Option, happy path)"
  (doc    "`parse-pair`-shaped Option chain collapsed with `?`: both `?`s see a `Some`, so the boundary
           falls through to the body's `Some`. `(Int64.checked-add 20 22)` = `(Some 42)`, so `x` = 42;
           `(Int64.checked-add 40 2)` = `(Some 42)`, so `y` = 42; the function returns `(Some (+ x y))` =
           `(Some 84)`. Witnesses DESIGN-try-operator-rcdzc.md §3.2 (the Option desugar) + §4 v1 (the
           enclosing-function boundary) on the happy path.")
  (input  (do
            (def (main)
              (let ((x (try (Int64.checked-add 20 22))))
                (let ((y (try (Int64.checked-add 40 2))))
                  (Some (+ x y)))))
            (export main)))
  (output (: (Some 84) (Option Int64))))

(case "`?` on the failure variant short-circuits the boundary (Option, None path)"
  (doc    "The short-circuit companion: the FIRST `?` sees a `None` (an overflowing checked-add), so it
           BREAKS the enclosing boundary — the function's value becomes `None` and the second `?` and the
           body never run. `(Int64.checked-add Int64.max-value 1)` overflows → `None`, so `(try …)` bails
           and `main` = `(None unit)`. Witnesses the abortive path of §3.2/§4: `?` exits the lexically
           enclosing function, contributing nothing to the effect row.")
  (input  (do
            (def (main)
              (let ((x (try (Int64.checked-add Int64.max 1))))
                (let ((y (try (Int64.checked-add 40 2))))
                  (Some (+ x y)))))
            (export main)))
  (output (: (None unit) (Option Int64))))
