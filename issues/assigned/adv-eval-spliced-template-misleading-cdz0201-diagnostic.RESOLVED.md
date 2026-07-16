# SOUND gap, MISLEADING diagnostic (breaker): eval of an Ast-spliced template rejects with a user-type-error-looking CDZ0201

Probed the new ast-lift splicing (seed + ML both closed the splice gap): splicing itself is SOLID — param-bound, let-bound, and inline (quote …) operands all graft correctly (match consumers verify structure; structural equality with the direct quote holds). The remaining decline is EVAL of a template containing an Ast splice: eval's desugar reconstructs source statically and can't see through a runtime-spliced subtree, so (eval (quasiquote (+ (unquote (quote (* 2 3))) 1))) rejects — arguably sound (eval is the optional/limited surface). BUT the diagnostic is misleading: CDZ0201 'a Ast and an Int64 are different types (this operation is not defined across that kind boundary)' at the eval site, which reads like a user type error rather than 'eval cannot reconstruct a runtime-spliced template'. Two routes: (a) diagnostics vertical gives it a named message + fix hint (bind the template, match on it instead), or (b) v-metaprogramming teaches the eval desugar to interpret the spliced node (the trees are provably equal to evaluable ones). Not filing as a miscompile — no wrong value; the equality probe proves the spliced tree is the right tree.

VERIFIED live on trunk 535f90aa7: (eval (quasiquote (+ (unquote (quote (* 2 3))) 1))) -> CDZ0201 "a Ast and an Int64 are different types" at the eval site.

## RULING 2026-07-16 (v-metaprogramming, owns quote/quasiquote/eval)
DELIBERATE SOUND LIMIT — do NOT extend eval; the v-diagnostics message rephrase is the WHOLE answer.
Mechanism: eval's desugar (eval_ast::reconstruct) reconstructs SOURCE statically. A VALUE-splice
reconstructs fine ((eval `(+ ,n 1)) n=6 -> 7 PASSES). It ONLY declines when the unquote operand is
itself an Ast value ((unquote (quote (* 2 3)))) -> reconstructed source has an Ast in arithmetic
position -> CDZ0201 (a correct type error). Evaluating that needs the desugar to RECURSIVELY INTERPRET
a runtime Ast = a runtime AST interpreter, which metaprogramming.md marks OPTIONAL (seed ships
compile-time-FOLD eval only). So sound-declines, not a bug. v-metaprogramming pinning the boundary
(value-splice works / Ast-value-splice declines) in 12-metaprogramming. Only remaining action:
v-diagnostics' named-message rephrase.
