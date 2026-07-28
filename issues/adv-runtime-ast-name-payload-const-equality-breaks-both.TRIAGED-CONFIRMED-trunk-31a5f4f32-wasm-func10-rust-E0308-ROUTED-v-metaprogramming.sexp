; FINDING (breaker, 2026-07-28): an Ast.Name whose String payload is RUNTIME-SELECTED
; ((if (= mode 1) "defx" "defy")), spliced into a REBUILT Ast.List that is then compared
; against a CONST-FOLDED read result, breaks BOTH backends — DIFFERENTLY:
;   wasm: invalid component, func[10] fails to validate (module written, every run traps)
;   rust: artifact does not build — E0308 mismatched types at rustc
;
; Isolation:
;   ok  (Ast.Name (if ...)) alone, matched + payload measured (m1: 4/4)
;   ok  the same rebuild+compare with a CONST Name payload, mode split at the OUTER call (m3: 1/0)
;   FAIL only the composition: runtime-selected payload INSIDE the rebuilt list INSIDE the
;        (= rebuilt (read "...")) comparison — the const-fold of `read` meets a PARTIALLY-RUNTIME
;        Ast operand and the equality's lowering splits: part const Ast, part runtime value.
;
; Smell: the = lowering (or the Ast reify) commits to a CONST-Ast representation for one operand
; and a runtime rep for the other, then emits a comparison across mismatched reps — wasm emits
; an ill-typed function, rust emits ill-typed source (E0308). A rep-boundary bug, not a value bug.
; Related family: my earlier wasm-decline-vs-rust-compute cases were VERDICT splits; this one
; BUILD-FAILS both, so nothing runs — cannot ship as a todo pin (rust FAILS not declines).
;
; GRADED REPRO (expected 1 at mode 1 — the correct codemod answer; both backends break today):
(case "a runtime-selected Name payload inside a rebuilt Ast compares against a read result"
  (input  (do
        (def (main (: mode Int64))
          (match (read "(defn add 1)")
            ((Ast.List parts)
              (match parts
                ((list (Ast.Name _kw) rest .. more)
                  (if (= (Ast.List (List.prepend (List.prepend more rest)
                                                 (Ast.Name (if (= mode 1) "defx" "defy"))))
                         (read "(defx add 1)"))
                      1 0))
                (_ -2)))
            (_ -3)))
        (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 2 Int64)) (output (: 0 Int64)))
