# DESIGN — Macro system: macros are plain functions over the AST

> **Status:** Phase-1 design (draft for operator sanity-check). Subsystem: `rcdzc` (with a spec
> touch to `spec/capabilities/metaprogramming.md`, coordinated with v-spec-oracle). Owner:
> `v-metaprogramming`. Operator greenlit 2026-09-01.
>
> **Operator intent (verbatim):** "it's just a function that takes AST arguments and returns an
> AST. One interesting thing would be to be able to evaluate an AST in the callers scope instead of
> in the function that's being called. If we have that and have the ability to specify an argument
> as taking an unevaluated argument instead of an eager value I think that's all you need, right? We
> probably want a gensym as well. I think the way we have the AST it's going to be difficult to
> provide hygiene by default. But I really like macros just being plain functions that use
> metaprogramming facilities and the caller doesn't care."

## 1. Model

A macro is **an ordinary function** — no `defmacro`, no macro-definition form, no special dispatch.
It takes AST arguments and returns an AST; the caller writes an ordinary call and does not know or
care that the callee is a macro. This is already the spec's stance ("A macro MUST be an ordinary
compile-time function over the abstract syntax tree", metaprogramming.md:86) and the one-tier
evaluator (`eval::apply_lambda`, eval.rs:1057) already runs it. Two existing load-time rewrites —
`eval` (`desugar_eval`, eval_ast.rs:79) and tagged templates (`tagged_template::expand`,
tagged_template.rs:52) — already do "rewrite a call form → ordinary application → splice the result
in place → let the one-tier fold reach a fixpoint before typecheck". **The macro system is that
same expansion generalized to ordinary calls whose callee marks parameters as unevaluated.**

So we do **not** add an evaluator or a lazy/thunk representation. We add: (a) a per-parameter
"unevaluated" marker, (b) a load-time/post-resolve expander that reifies the marked arguments to
`Ast` and splices the macro's result at the call site, (c) a `gensym` primitive. Caller-scope
evaluation falls out of (b). Hygiene-by-default is explicitly dropped (§6).

## 2. Capability 1 — unevaluated (call-by-AST) parameters

**Surface (proposed).** Mark the parameter, not the call site (the caller "doesn't care"). Two
candidates, operator to pick in §7:
- **A.** a parameter annotation of type `Ast` combined with an *unevaluated* marker, e.g.
  `(def (unless (: cond !ast) (: body !ast)) …)` — a `!ast` / `#ast` marker on the binder means
  "bind the argument's reified AST, do not evaluate it".
- **B.** a dedicated binder form `(def (unless (quote cond) (quote body)) …)` reusing the `quote`
  keyword in binder position to mean "this parameter receives its argument quoted".

Either way the marker lives on the **callee's** signature. Default (unmarked) parameters stay
strict call-by-value.

**Mechanism.** At a call `(unless c b)` where `unless`'s parameters are unevaluated: before the
argument is ever resolved/typed/lowered as a value, **reify the argument node to its `Ast`** (reuse
`quote::reify_quotes`, the exact machinery `(quote c)` uses) and pass that `Ast` constant as the
parameter. The callee then receives `cond : Ast` = the reflected syntax of `c`. This is a
**front-end** concern — it must happen after name resolution knows the callee's signature but
**before** infer/lower see the argument as a value (§4). It does **not** touch `apply_lambda` or the
runtime call path (which correctly assume value args).

## 3. Capability 2 — caller-scope evaluation (mostly free)

The operator wants an AST to be able to evaluate **in the caller's scope**, not the callee's. The
key realization: **the splice-at-call-site expansion model gives this for the macro's *output*
automatically.** The macro's returned `Ast` is spliced into the call-site node (as
`tagged_template::expand` does, tagged_template.rs:82-90), so every name in it resolves against the
**caller's** lexical scope. An unevaluated argument (the caller's own syntax) embedded in the
returned AST likewise resolves at the call site = the caller's scope. No `eval-in-env` primitive is
needed for the common macro shape (destructure arg ASTs, splice a new AST that mentions them).

**Open fork (§7).** A macro that wants to evaluate an arg AST **to a value _during_ its own body**
(compute with it at expansion time, not just re-emit it) would evaluate it in the *callee's* scope —
wrong for names free in the caller. Two options: (i) declare this out of scope for v1 (the splice
model covers the stated use cases), or (ii) add an explicit `eval-at-caller` affordance. Current
`eval` (`desugar_eval`) resolves in the eval-site's own scope (eval_ast.rs:363-366) and has no
caller-env injection, so (ii) is genuinely new work. **Recommend (i) for v1**, revisit if a concrete
need appears.

## 4. The expansion phase

A new load-time expander, sibling to `tagged_template::expand`, but keyed on **binding**: when a
call head resolves to a function with unevaluated parameters, rewrite the call so the marked
arguments are reified to `Ast` and the others stay values, then let `eval::apply_lambda` β-reduce and
splice the result in place, expanding to a fixpoint before typecheck. Unlike tagged templates (which
always package chunks+holes as lists with no callee-signature lookup), the macro expander needs the
**callee's parameter markers**, so it runs **after name resolution** (to know the callee) and
**before type checking** — matching the spec's "Macro expansion MUST run as a distinct phase that
precedes type checking … expanding to a fixpoint" (metaprogramming.md:145-148). Phase placement
(extend resolve, or a dedicated post-resolve pass) is settled in increment 1.

## 5. Capability 3 — gensym

No general fresh-name facility exists (only `unify::Fresh` for *type* variables, and one ad-hoc
`{spelling} $capture{n}` unreadable-name mint in the eval hygiene pass, eval_ast.rs:222). Add a
`gensym` compile-time primitive returning a fresh `Ast.Name` guaranteed collision-free — reuse the
"embed a character the reader can never produce" trick (a space) so a generated name cannot clash
with any source or other gensym. Small, self-contained; folds like the other `Ast.*` intrinsics
(ast_reflect.rs). Surface: `(gensym)` or `(Ast.gensym)` → `Ast`, pure.

## 6. Capability 4 — no hygiene by default (⚠ SPEC CHANGE — v-spec-oracle handshake)

The operator accepts **no hygiene by default**: a macro that introduces a binder may capture / be
captured; authors get hygiene manually via `gensym`. **This contradicts the current spec**, which
mandates hygiene: metaprogramming.md:136-142 "A name a macro introduces MUST NOT capture …",
"Hygiene MUST be realized by tracking the set of scopes an identifier carries". (Note the scope-set
mechanism is spec-only — **not implemented** today; the sole hygiene code is the targeted
alpha-rename in `eval`'s reconstruction, eval_ast.rs:181-234.) So the plain-function macro model
requires **amending the "Macros Are Hygienic" section** to state that macros are non-hygienic by
default and hygiene is opt-in via `gensym`. This is a faithful-mirror **spec change that needs
v-spec-oracle sign-off** before the corpus locks it in. Flagged as the top operator/spec fork (§7).

## 7. Open questions / forks needing an operator or oracle call

1. **Unevaluated-param surface syntax** — §2 option A (`!ast`/`#ast` marker) vs B (`quote` in binder
   position). *Recommend A* (explicit, unambiguous, reads as a type-ish annotation).
2. **Caller-scope eval scope** — §3: v1 = splice-model only (recommend) vs also an `eval-at-caller`
   value primitive.
3. **Hygiene spec amendment** — §6: relax the hygiene MUST to "non-hygienic by default, manual via
   gensym" (needs v-spec-oracle handshake). **Operator: confirm the spec should be amended, not the
   model.**
4. **Naming** — `gensym` vs `Ast.gensym` vs `Ast.fresh`; keep consistent with the `Ast`/`Type`
   reflection-module convention.

## 8. Increments (corpus-pinned, direct-to-main, one PR each)

1. **`gensym`** — the standalone primitive + corpus (fresh, collision-free, distinct per call). No
   dependence on the rest; proves the fresh-name substrate hygiene will lean on.
2. **Unevaluated parameters + the expander** — the surface marker + the post-resolve reify-and-splice
   expander; a first macro (`unless`/`swap`) as a plain function, corpus-pinned to its expansion.
3. **Caller-scope resolution corpus lock** — pin that a macro's output resolves in the caller's scope
   (a macro referencing a caller-local name; a capture case demonstrating the *no-hygiene* default).
4. **Spec section + hygiene amendment** — land the metaprogramming.md macro/hygiene wording via the
   v-spec-oracle handshake; pin the manual-hygiene-via-gensym pattern.

Each increment gates: corpus (`nix build .#checks…corpus-…`), ml_surface round-trip, nativize,
clippy `-D warnings`, pinned fmt. No runtime/hash change expected (front-end expansion + folds).
