; ADVERSARIAL FINDING (breaker, 2026-07-15) — 🟠 FALSE REJECTION (valid program rejected CDZ0101):
; a module MUTUAL-RECURSION cycle in which one member is PRIVATE (an `(export …)` clause names the
; other) rejects "unbound name", though the privacy landing (0c008299) explicitly keeps a private
; member MUTUALLY VISIBLE to its siblings ("the filter touches only the export record, not
; resolve::module_sibling_binds"). The cycle is the missed face: one-directional forward/backward
; references to a private sibling DO resolve; only a private member participating in a CYCLE loses
; resolution.
;
; REPRODUCER (rejects CDZ0101; expected: runs, (. m even) 4 = 1):
;   (do (module m
;         (export even)
;         (def (even (: n Int64)) (if (= n 0) 1 (odd (- n 1))))
;         (def (odd  (: n Int64)) (if (= n 0) 0 (even (- n 1)))))
;       ((. m even) 4))
;
; ISOLATION (trunk@48d9d976, fresh build; each hand-verified):
;   mutual pair, NO export clause                          → 1     [PASS — export-everything default]
;   mutual pair, BOTH names exported                       → 1     [PASS]
;   mutual pair, `even` exported, `odd` private            → 🟠 CDZ0101  (this finding)
;   mutual pair, private `odd` defined FIRST               → 🟠 CDZ0101  (definition order irrelevant)
;   NON-mutual forward ref to a private sibling            → 42    [PASS — one-directional is fine]
;   NON-mutual private helper defined after its caller     → 42    [PASS]
;   → the discriminator is exactly PRIVATE ∧ IN-A-CYCLE.
;
; ROOT CAUSE (hypothesis): the mutual-recursion path resolves a cycle member through the module's
; EXPORT RECORD (or a scope built from it) rather than through `module_sibling_binds` — the sibling
; scan covers straight-line/forward refs, but the cycle-participant lookup (likely the recursive-def
; pre-bind that ties the knot for mutually-recursive groups) consults the filtered record, so a
; private cycle member is "unbound" at its co-member's call site. Fix locus: make the mutual-group
; pre-bind use the member set, not the export set.
;
; SEVERITY: 🟠 FALSE REJECTION, not a miscompile — but it makes the privacy feature unusable for the
; MAIN idiom it exists for (hide the helpers of a recursive implementation; parser/walker helper pairs
; are almost always mutually recursive). Grades todo under the gate (rejection where output expected),
; so it will not show as a red gate — the graded case below records the intended semantics.

(case "a private module member participates in mutual recursion with an exported sibling"
  (doc    "`(module m (export even) (def (even n) … (odd …)) (def (odd n) … (even …)))` — `even` is
           exported, `odd` is private (absent from the export clause). Visibility is explicit for
           IMPORTERS only: a private member stays mutually visible to its siblings (modules-and-
           namespaces.md §Visibility Is Explicit; the 0c008299 landing pins sibling visibility via
           module_sibling_binds). So the cycle even↔odd must resolve and `((. m even) 4)` = 1 (4 is
           even). Instead the cycle member rejects CDZ0101 'unbound name': the mutual-group knot-tying
           resolves through the FILTERED export record, so the private co-member is invisible exactly
           when it participates in a cycle — one-directional references to a private sibling resolve
           fine (both orders), and the same cycle passes with no export clause or with both names
           exported. Hiding the private half of a recursive helper pair is the privacy feature's
           canonical use; the cycle path must consult the member set, not the export set. Expected: 1.")
  (input  (do
            (module m
              (export even)
              (def (even (: n Int64)) (if (= n 0) 1 (odd (- n 1))))
              (def (odd (: n Int64)) (if (= n 0) 0 (even (- n 1)))))
            ((. m even) 4)))
  (output (: 1 Int64)))

;; RESOLVED 2026-07-15 (trunk@3ba79db6b): private module member's body now sees siblings (mutual recursion) — gate PASS
