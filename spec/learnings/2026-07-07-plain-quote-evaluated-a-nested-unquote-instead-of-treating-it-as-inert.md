# Plain quote evaluated a nested unquote instead of treating it as inert

*2026-07-07*

**What happened.** Adversarial probing of the metaprogramming surface found a `quote` that
is not inert. `(quote (g ,x))` with `x` bound to `99` produced
`(Ast.List (list (Ast.Name "g") (Ast.Int 99)))` — byte-identical to the quasiquote
`` `(g ,x) ``. Plain `quote` was *evaluating* the `,x` and embedding its value, so `quote`
and `quasiquote` were the same function on this input. A companion case exposed a second
leak on the same code path: `(quote (unquote 1 2))` returned `(Ast.Int 1)` — the multi-operand
`unquote` was evaluated *and* silently truncated to its first operand, skipping the arity
check that the quasiquote path enforces (`(quasiquote (unquote 1 2))` correctly rejects
CDZ0201).

**Why it is a break.** metaprogramming.md #Quote Produces An AST Value: `(quote <expr>)` MUST
represent the structure of `<expr>` *without evaluating `<expr>` itself*. #Quasiquote
Constructs AST With Selective Evaluation: "Unquote and unquote-splicing outside a quasiquote
context MUST be a syntax error." A plain `(quote …)` body is inert data, not a
selective-evaluation template — so a nested `,x` is exactly the "unquote outside quasiquote"
the corpus already pins for the bare `,x` case (rejected CDZ0401). One layer of `quote` deep,
the rule is unchanged: reject (or preserve inert), never evaluate. Evaluating it makes `quote`
observably non-inert, contradicting #Quote Produces An AST Value, and the arity leak drops a
malformed form's second operand instead of rejecting it.

**Root cause — the active-unquote guard fired at quote's own level.** In the seed
(`codegen.rs::quote_node`), `quote` calls in at `level=0` and `quasiquote` at `level=1`; an
`unquote` decrements the level and is "active" (evaluated) when it would bring an *enclosing
quasiquote* to level 0. The active-unquote branch was guarded `if level <= 1`, which is true
at level 0 (plain quote) as well as level 1 (quasiquote). So an unquote under plain quote was
treated as active. The `unquote-splicing` sibling in the same function was guarded
`level == 1` — which is why `,@x` stayed inert under plain quote while `,x` leaked. That
asymmetry between the two guards in one function was the tell: the correct guard for the
active branch is `level == 1`, matching the splicing sibling. (At level 0 the form is not a
quasiquote hole; per §"unquote outside quasiquote" it is a CDZ0401 rejection, and a multi-
operand unquote is CDZ0201 as in the quasiquote path.)

**The lesson.** A construct with a nesting-level counter has an off-by-one hazard at the
*ground* level. `quote` and `quasiquote` share one recursive walker distinguished only by the
starting level, so a guard written `level <= 1` (natural when you're thinking "the innermost
active case") silently swept in the plain-`quote` ground level too, collapsing the two forms.
The gate did not catch it because the corpus pinned the *bare* `,x` reject and every
*positive* quasiquote embedding, but never an unquote nested inside a **plain** quote — the
one input that distinguishes an inert `quote` from an active `quasiquote`. When two forms
share a parameterized walker, the adversarial case is the one where the parameter is at its
boundary value: quote-with-a-nested-unquote is to quasiquote what the empty list is to a fold.

**Corpus case added.** `spec/semantics/12-metaprogramming.sexp` §"an unquote nested inside a
plain quote is a syntax error, not an active unquote" — `(quote (g ,x))` MUST reject CDZ0401,
alongside a note pinning the `(quote (unquote 1 2))` arity companion (CDZ0201). Native seed
only; the Cadenza compiler declines `quote` entirely, so the differential gate would not have
surfaced it — the behavior gate does.
