# An unquote that cannot evaluate falls back to quoting, swallowing a scope error

*2026-07-08*

**What happened.** Adversarial probing of quasiquote found that an unquote whose expression cannot
be const-evaluated silently falls back to QUOTING the expression as inert AST, rather than raising
the error the failed evaluation implies. `` `(a ,(+ b 1)) `` with `b` unbound produces `(Ast.List
(list (Ast.Name "a") (Ast.List (list (Ast.Name "+") (Ast.Name "b") (Ast.Int 1)))))` — the unquote
`,(+ b 1)` did not evaluate `(+ b 1)` (it can't, `b` is unbound); instead it quoted the expression.
With `b` bound, `` `(a ,(+ b 1)) `` correctly evaluates to `(a 6)`; the bare `(+ b 1)` correctly
rejects CDZ0101. Only inside the unquote does the unbound name get swallowed.

**Why it is a break.** metaprogramming.md #Quasiquote Constructs AST With Selective Evaluation: a
subexpression `,<expr>` "MUST evaluate `<expr>` normally and insert its result." Evaluation of
`(+ b 1)` requires resolving `b`, which is unbound — core-semantics.md #Binding Is Lexical makes a
reference to a name with no enclosing binding an unconditional compile-time error (CDZ0101). So the
unquote must be rejected, not quieted into a valid AST value. Falling back to quoting turns the
selective-EVALUATION unquote into a second quote and converts a scope error into a successful value
— a false accept of an ill-formed program.

**Root cause — the unquote's eval fallback quotes on any non-const result.** In the seed
(`codegen.rs::quote_node`), an active unquote evaluates its inner expression and, on the catch-all
arm, falls back to quoting it:

    match self.eval_const(inner, env) {
        Ok(Some(CVal::Ast(n))) => return Some(n),
        Ok(Some(v))           => return cval_to_node(&v),
        _                     => return self.quote_node(inner, env, 0),   // fallback: quote it
    }

The `_` arm fires for BOTH "not a compile-time constant" (a legitimate reason to defer to a runtime
embed) AND "ill-formed / unbound" (an error). Conflating them means an unbound name in an unquote is
quoted instead of rejected. The fallback should distinguish: a runtime-but-well-scoped expression
may embed at runtime, but an expression that fails to *resolve* (unbound name) is the unbound-name
error — the unquote must not swallow it by quoting.

**The lesson.** A fallback keyed on "eval_const didn't return a value" collapses two very different
cases — "this is fine but not constant" and "this is broken" — into one path, and picks the wrong
behavior (quote it) for the broken one. When a construct's semantics is "evaluate X," a failure to
evaluate X because X is ill-formed is X's error, not a cue to reinterpret X under different
semantics (quote). The tell: the identical expression `(+ b 1)` rejects bare but is silently quoted
under `,` — the unquote changed the expression's meaning from evaluate-it to quote-it precisely when
evaluation would have surfaced the error.

**Corpus case added.** `spec/semantics/12-metaprogramming.sexp` §"an unquote of an expression with
an unbound name is rejected, not quoted" — `` `(a ,(+ b 1)) `` (b unbound) MUST reject CDZ0101, the
unbound-in-unquote companion of the unquote-evaluates cases. Native seed; the behavior gate catches
it (expected reject CDZ0101, observed a running component whose unquote was quoted).
