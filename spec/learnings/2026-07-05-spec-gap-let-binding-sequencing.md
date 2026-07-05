# Spec gap: whether `let` binds sequentially or in parallel is unspecified

*2026-07-05*

**What happened.** An adversarial-corpus `/loop` run probed multi-binding `let` — does a later binding
see an earlier one in the same `let`? `(let ((x 1) (y (+ x 1))) y)` evaluates to 2 on the seed (a
sequential `let*`, where `y`'s initializer sees `x`), and `(let ((x 1) (x (+ x 10))) x)` evaluates to 11
(the second `x` sees the first). The run could not record either as the oracle: the specification fixes
`let` only *syntactically* — `options/code-shape/` lists `(let ((<name> <expr>)…) <body>)` as "a lexical
binding form" — and states no requirement on whether the bindings in one `let` are evaluated
sequentially (each initializer sees the earlier names, i.e. `let*`) or in parallel (every initializer is
evaluated in the enclosing scope, so a reference to a sibling binding is unbound). Recording either
outcome would have invented a design decision the specification has not made, so the case was dropped.

**Why.** `core-semantics.md` §Binding covers lexical resolution, shadowing (deferred to the corpus), and
closure capture, but treats `let` as a single binding form without addressing the *multi-binding* case.
The two readings diverge observably: under parallel semantics `(let ((x 1) (y (+ x 1))) y)` is an
unbound-name rejection (CDZ0101); under sequential semantics it is 2. A behavioral property with two
incompatible observable outcomes and no requirement selecting between them is an under-specification, not
a corpus omission — the corpus cannot record an oracle the specification has not fixed.

**The requirement it drove.** *Deferred to a clarity pass* (this entry is the hand-off; the requirement
edit is intentionally left to a follow-up agent, per the operator's request to document gaps for a
clarity-focused agent rather than resolve them inline). The resolution should add one RFC-2119 sentence
to `core-semantics.md` §Binding fixing the multi-binding `let` semantics — the conventional choice is
sequential (`let*`, each initializer sees the bindings to its left), which matches the seed and the
common functional-language default, but the direction is the operator's to set. Once fixed, it is
witnessed by cases in `spec/semantics/02-binding-and-control.sexp`: `(let ((x 1) (y (+ x 1))) y)` → 2 and
`(let ((x 1) (x (+ x 10))) x)` → 11 under sequential, or the corresponding CDZ0101 rejections under
parallel. Until then the corpus records neither, and the seed's sequential behavior is an unspecified
implementation choice.
