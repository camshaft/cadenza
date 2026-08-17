# Postmark, let chain as perform argument (2026-08-17)

- `pst1.sexp` — LET expression in a perform's ARGUMENT position (sweep:
  17 if-args, 2 match-args, ZERO let-args at perform sites). The body-side
  mirror of bcn1's let-in-resume-arg: stamp #2's argument let-binds the
  first ANSWER's tens digit ((/ a 10)) doubled-plus-one; stamp #3's chains
  TWO lets folding the second answer's low digit with the audit's remainder
  before a mod-7 clamp. Each let chain is scoped inside the argument
  expression only, and its bindings depend on earlier dispatch answers, so
  the continuation must carry them across the suspend correctly. 4/5 rows
  diverge across n%3 seeds. PASS x3 at e46e64712.
