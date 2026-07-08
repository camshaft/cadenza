# A fold matching a tuple of its recursive results with constructor patterns is declined

*2026-07-08*

**What happened.** Adversarial probing of the structural-editing (rep→rep) seam found that the natural
constant-fold pass — recursively fold an expression's two children, then match the TUPLE of the two
recursive results with constructor patterns to combine constant leaves — is declined. The body

    (match (tuple (fold a) (fold b))
      ((tuple (E.Lit x) (E.Lit y)) (E.Lit (+ x y)))
      ((tuple fa fb)               (E.Add (tuple fa fb))))

inside a recursive `fold : E → E` declines "constructor pattern against unresolved scrutinee (e.g.
quote/AST)" (or, with the two folds `let`-bound first, "match scrutinee is not compile-time-resolvable").
It is a VALID program: the SAME fold written with separate nested single-scrutinee matches — `(match
(fold a) ((E.Lit x) (match (fold b) …)) …)` — compiles and yields the correct result (12 for `(Add (Lit
3) (Add (Lit 4) (Lit 5)))`). So this is an honest decline of a not-yet-realized codegen path, not a
miscompile and not a false rejection of the only formulation.

**Why it matters.** This is the archetypal optimizer idiom a self-hosted compiler is written in — fold
the children, then pattern-match the folded pair to fire a local rewrite rule. The existing corpus case
"a transformation maps a syntax tree to a syntax tree and preserves meaning" (20-structural-editing.sexp)
had to WORK AROUND exactly this: its `simp` binds `x`/`y` with `let` and probes them with a
single-scrutinee `is-lit` helper, and its doc notes "a constructor pattern binds its payload rather than
matching a nested literal directly." The workaround exists precisely because the natural tuple-match form
is declined. So the limitation is real and already shapes how compiler passes must be written.

**Root cause (likely) — a recursive self-call's result shape is unresolved at the pattern site within
its own body.** The general capability exists: a tuple of two RUNTIME sums from NON-recursive producers
matches with constructor patterns fine, and a SINGLE recursive-self-call result matches fine. Only the
combination — a TUPLE whose elements are recursive self-calls of the enclosing function, matched with
CONSTRUCTOR patterns — is not realized. The compiler resolves a scrutinee's shape to check constructor
patterns (the `resolve`/beta-reduce path from `[[ask65-payload-through-return-resolve-not-inference]]`),
but a recursive self-call inside the function's own body cannot be resolved that way (the recursion
doesn't bottom out at compile time), so the tuple element's shape is "unresolved" and the constructor
pattern check declines. The realization is to lower a constructor pattern against a genuinely-runtime
(unresolvable) sum element by emitting the runtime tag dispatch, rather than requiring the element's shape
to be statically resolvable — the same runtime sum-match the single-scrutinee and non-recursive-tuple
cases already emit, extended to a tuple element that is a recursive self-call.

**The lesson.** A capability realized for the pieces (runtime sum match; tuple of non-recursive-producer
sums; single recursive result) is not yet realized for their composition (tuple of recursive-self-call
sums with constructor patterns) — a composition gap on the self-hosting-critical fold idiom. The tell:
the corpus already contains a hand-written workaround (`is-lit` + `let`) for the exact shape, which is a
strong signal the natural form should be a pinned realization target.

**Corpus case added.** `spec/semantics/20-structural-editing.sexp` §"a bottom-up fold matches a tuple of
its recursive results with constructor patterns" — the natural constant-fold, expecting output 12 (the
value the working single-scrutinee equivalent produces). The seed currently DECLINES, so the case
classifies `todo` (an honest decline, gate stays GREEN); it will PASS when the tuple-of-recursive-self-
calls constructor-pattern lowering lands. A generation that does not yet resolve a recursive self-call's
shape at the pattern site declines rather than miscompiling.
