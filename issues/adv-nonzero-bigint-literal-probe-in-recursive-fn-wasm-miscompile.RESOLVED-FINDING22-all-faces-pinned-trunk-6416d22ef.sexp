; FINDING (breaker, 2026-07-24): a NONZERO BigInt-payload literal pattern probe inside a
; RECURSIVE (or mutually recursive) function miscompiles on wasm. Two observable faces:
;   FACE A (silent wrong value): on the built-in Ast (Ast.Int payload = BigInt), the literal
;     arm NEVER MATCHES — the match falls through to the catch-all, so a peephole rule like
;     `(* ,x 1) -> x` silently returns its input unchanged. VALID module, wrong behavior.
;   FACE B (invalid module): the minimal plain user sum `(type W (Mk BigInt))` with the same
;     shape emits an INVALID wasm component: "failed to compile: wasm[0]::function[2]".
; RUST: the Ast/quote form declines explicitly ("a non-scalar literal-payload probe is not
; rendered by the Rust backend" — an honest not-yet); the plain-sum form (Face B) COMPILES on
; rust — rust verdict on compute not yet compared (wasm never runs).
;
; MATRIX (all wasm, minimal repros; 40 = matched, -1 = fell through):
;   ✗ (type W (Mk BigInt)); recursive walk; arm (Mk 1); (walk 1 (Mk 1))     → INVALID MODULE (Face B)
;   ✗ recursive simp, arm `(* ,x 1); input (quote (* y 1))                  → -1 (no match)
;   ✗ same with literal 2 / literal 5 (pattern and input agree)             → -1
;   ✗ arm `(+ ,x 1); input (+ y 1)  (plus head, nonzero literal)            → -1
;   ✗ hand-BUILT input (Ast.List (list (Ast.Name "*") (Ast.Name "y") (Ast.Int 1))) → -1 (quote not required)
;   ✗ hand-WRITTEN pattern (Ast.List (list (Ast.Name "*") x (Ast.Int 1)))   → -1 (quote-pattern not required)
;   ✗ recursion via a WRAPPER (mutual: simp -> go -> simp)                  → -1
;   ✓ literal 0: arm `(* ,x 0) / `(+ ,x 0) recursive                        → matches (b4/b18 pass;
;       the LANDED root-only peephole pin + my nested-quote chain pins all use 0 — why the corpus
;       never caught the nonzero row)
;   ✓ NON-recursive: every nonzero form matches fine (landed pins + controls)
;   ✓ Int64 payload (type W (Mk Int64)), recursive, (Mk 1)                  → 40
;   ✓ String payload (Mk "a"), recursive                                    → 40
;   ✓ BINDER instead of literal + explicit = compare                        → works (workaround)
;   ✗ pattern 1 vs input 0 / pattern 5 vs input 0 (recursive)               → -1 (so the literal is
;       not simply zeroed; the probe comparison itself is broken under recursive specialization)
;
; So: BigInt-ONLY (Int64/String payloads fine), NONZERO-only (0 fine), RECURSIVE-fn-only
; (non-recursive fine), quote-machinery-independent. Likely the recursive-fn specialization
; materializes the BigInt literal for the probe differently (const-pool/heap handle not
; emitted or referenced wrongly in the recursive context), with the plain-sum case tripping
; validation (Face B) and the multi-variant Ast case surviving as a never-true compare (Face A).
;
; IMPACT: any recursive Ast rewriter with a nonzero integer literal in a quote pattern —
; the EXACT shape of a real peephole pass ((* x 1) -> x, (+ x 1) unfold, etc.) — silently
; no-ops on wasm. The structural-editing corpus doc (20-structural-editing.sexp peephole case)
; already notes recursive simp is "a shape the rust backend does not yet fold" — on wasm it
; COMPILES and returns WRONG VALUES, which is strictly worse.
;
; Repro below = Face A minimal (expect 40; wasm gives -1). Face B one-liner in the matrix above.

(case "a nonzero BigInt literal probe in a RECURSIVE fn matches its own constructor (FACE-A repro)"
  (input (do
        (def (simp node)
          (match node
            (`(* ,x 1) (simp x))
            (other     other)))
        (def (main)
          (match (simp (quote (* y 1)))
            ((Ast.Name _n) 40)
            (_ -1)))
        (export main)))
  (output (: 40 Int64)))
