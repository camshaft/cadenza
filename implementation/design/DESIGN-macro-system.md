# DESIGN — Macro system: macros are plain functions over the AST

> **Status:** Phase-1 design — operator sanity-check DONE 2026-09-01, all four §7 forks decided
> (see "Operator decisions" below). Subsystem: `rcdzc` (spec touch to
> `spec/capabilities/metaprogramming.md` with v-spec-oracle; caller-env-eval effect with v-effects).
> Owner: `v-metaprogramming`. Increment 1 (`Ast.gensym`) landed #7274.
>
> **Operator decisions (2026-09-01, supersede the §7 recommendations where they differ):**
> - **(a) Unevaluated param → the QUOTE approach, NO new sigil.** Reuse the existing `quote` surface
>   to mark a call-by-AST parameter; do not add a `!ast`/`#ast` marker (§2 rewritten).
> - **(b) Caller-env eval → IN v1, modeled as an EFFECT (operator overrode the defer).** A macro (an
>   ordinary fn) performs an effect that evaluates an arbitrary AST in the *caller's* environment and
>   returns the value, threaded + handled like any Cadenza effect — no `defmacro`. Cross-lane with
>   **v-effects** (§3 rewritten). This is the powerful core, not deferred.
> - **(c) Hygiene → operator OPEN to PRESERVING it; spec-amendment ON HOLD.** Produce a
>   hygiene-preservation proposal with **v-spec-oracle** addressing the "track scopes of all name
>   nodes" worry; preserve if feasible+simple, else fall back to no-auto-hygiene + amend (§6 rewritten).
> - **(d) Naming → `Ast.gensym`** (namespaced, rebindable, no global). LANDED #7274.
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
`quote` marker (§2), (b) a **post-resolve** expander that reifies the marked arguments to `Ast` and
splices the macro's result at the call site, iterating to a fixpoint (§4), (c) `Ast.gensym` (§5),
and (d) the general `Eval` effect for caller-environment evaluation (§3). Hygiene is **preserved**
(fork c) via per-splice provenance+rename, pending operator ratification of a faithful-mirror :142
wording tweak (§6).

## 2. Capability 1 — unevaluated (call-by-AST) parameters

**DECIDED (fork a): the QUOTE approach, no new sigil.** Mark the parameter on the **callee's**
signature by reusing the existing `quote` surface in binder position — e.g.
`(def (unless (quote cond) (quote body)) …)` means "this parameter receives its argument quoted (as
an `Ast`), not evaluated". No `!ast`/`#ast` marker is added (operator: "I don't want more sigils than
we already have"). Default (unmarked) parameters stay strict call-by-value. Exact binder spelling
(`(quote cond)` vs a `quote`-typed annotation) is settled in the increment, but it is the `quote`
mechanism, not a new sigil.

**Mechanism.** At a call `(unless c b)` where `unless`'s parameters are unevaluated: before the
argument is ever resolved/typed/lowered as a value, **reify the argument node to its `Ast`** (reuse
`quote::reify_quotes`, the exact machinery `(quote c)` uses) and pass that `Ast` constant as the
parameter. The callee then receives `cond : Ast` = the reflected syntax of `c`. This is a
**front-end** concern — it must happen after name resolution knows the callee's signature but
**before** infer/lower see the argument as a value (§4). It does **not** touch `apply_lambda` or the
runtime call path (which correctly assume value args).

## 3. Capability 2 — caller-environment evaluation, as an EFFECT (the core)

**DECIDED (fork b): in v1, modeled as an EFFECT — operator overrode the "defer / splice-only"
recommendation.** The operator wants a macro (an ordinary fn) to **evaluate an arbitrary AST in the
*caller's* environment and get back a value**, and to model that as an ordinary Cadenza **effect**
rather than a `defmacro`: "the function just has access to the env of the caller and can eval
arbitrary ASTs and get back an answer … it's basically an effect, so it would get threaded through
and you could wrap it just like any other effect." This is the powerful core of the model.

**Two paths compose.** (i) For output, splice-at-call-site still gives caller-scope resolution of the
macro's *returned* AST for free (as `tagged_template::expand` does, tagged_template.rs:82-90). (ii)
For *computing with* a caller expression during the macro body, a macro performs the **`in-caller` op
of the general `Eval` effect** — `Ast → Ast` — whose handler (provided ambiently at the call site by
the expansion context) evaluates the AST in the caller's environment and returns the reified value.
The macro's signature carries `{Eval}` in its row, so it is threaded and wrappable like any effect.

**Cross-lane with v-effects — SHAPE AGREED (2026-09-01).** The three open questions are resolved
(result type is `Ast`-reified, not value-polymorphic; ambient compiler-synthesized handler; plain row
entry). The canonical spec of the effect (v-effects' drop-in draft) follows.

### 3.x Caller-environment evaluation is the first op of a general `Eval` effect

Caller-environment evaluation is modeled as an ordinary effect, not a bespoke `defmacro` form. Per the
operator's refinement, the effect is a **general, EXTENSIBLE `Eval` effect** (the eval *capability*),
not a single-purpose one; its **first operation is `in-caller`** (evaluate an AST in the caller's
environment), with room to add further eval operations later:

    (effect Eval (op in-caller (-> Ast Ast)) ...future ops...)

`in-caller` takes the argument AST and returns an `Ast`: the evaluated value REIFIED back to an Ast
literal (via quote-reify). The result type is `Ast`, deliberately NOT value-polymorphic — a typed
effect row cannot carry a result type that depends on the argument value, and because a macro is
`Ast -> Ast` the reified-Ast result composes directly with the rest of the macro's AST manipulation.
(Returning a raw runtime value into the caller is a separate, future feature and is intentionally out
of scope for this op.)

Because it is a plain effect, `Eval` appears as an ordinary entry in a macro's effect row:

    a-macro : (quoted-arg: Ast) -{Eval}-> Ast

and threads/wraps exactly like any other effect: a function that calls the macro without discharging
the capability inherits `{Eval}` in its own row (standard row propagation), and a pure `Ast -> Ast`
macro that never evaluates in the caller simply carries no `Eval` — the capability is opt-in via the
row.

The capability is discharged by an AMBIENT, COMPILER-SYNTHESIZED handler that the expander wraps
around each macro application site — the `reduce_handle` analogue, but run at EXPANSION time rather
than at runtime:

    (handle Eval <caller-env>
      ((in-caller (a) _ <evaluate `a` in the caller's environment>))
      <macro-body>)

The arm evaluates the argument AST in the caller's environment (the one-tier evaluator seam,
`eval::apply_lambda`) and reifies the result to an Ast. Discharging at expansion time means `Eval` is
fully reduced away before the runtime backend ever sees it — it never becomes a `HostCall` or any
runtime effect, exactly as an in-program-handled effect folds away. The only distinction from a
user-written handler is WHO provides the arm (the compiler's expander) and WHEN it fires (expansion,
ambient at every macro call site) — the mechanism is the same nearest-enclosing effect discharge.
(Effect shape co-designed with v-effects, who co-owns increment 3; `Eval`-general naming per the
operator, 2026-09-01.)

**Explicit row + direct-caller semantics (operator directive, 2026-09-01).** Two refinements: (1) a
function acquires the capability **explicitly** — `{Eval}` is written in its signature/effect-row like
any Cadenza effect, NOT ambiently auto-acquired — so a **nested** function that also declares `{Eval}`
can perform it and it composes through the row normally. (2) The `caller` op targets the **direct /
immediate** caller's environment — the caller currently being evaluated — **not** a transitive parent
further up. These compose cleanly with the ambient discharge: the compiler-synthesized handler is wrapped
at **each** macro call site, so it provides **that** call's caller env; a nested `{Eval}` fn's
`in-caller` is therefore discharged by the handler at *its own* call site = its **direct** caller's env,
never the outermost. So "explicit row" governs acquisition (typed, composes to nested fns) and the
per-call-site ambient handler governs discharge (direct-caller env). The env captured by each synthesized
handler is the immediate call site's lexical environment. (Reconcile the acquisition/discharge split
with v-effects during increment 3.)

Reconnaissance settled the phase placement decisively: the expander **cannot** be a load-time
sibling of `tagged_template::expand`. Those load-time desugars (`reify_quotes` → `desugar_eval` →
`tagged_template::expand`, `db.rs:2565-2582`) run **before** the resolution indices exist
(`def_by_name`/`def_by_body`/`parent`/`scope_skip` are built at `db.rs:2658-2716`). Tagged templates
get away with a pure structural reshape because dispatch is *deferred* to ordinary resolution of the
rewritten call. A macro-call expander must instead **decide by the callee's signature** which
arguments to reify, so it needs resolution first → it runs as a **distinct post-resolve pass**
(after `db.rs:2716`), matching the spec's "Macro expansion MUST run as a distinct phase that precedes
type checking … expanding to a fixpoint" (metaprogramming.md:145-148).

**Param surface (increment 2a).** A `(quote x)` / `(quote (: x T))` binder in signature position marks
an unevaluated (call-by-AST) parameter. Recognize + normalize it at load exactly as
`strip_const_params` handles `(const …)` (`db.rs:2451-2457`): a `strip_quote_params` pass unwraps the
`(quote …)` wrapper **in place** to the plain binder (preserving the two-shape `name`/`(: name T)`
invariant every reader depends on) and records the binder occurrence in a new
`quote_params: FxHashSet<StructId>` side-set on `Db`. (Reuse `quote::binder_position_nodes`,
`quote.rs:364-383`, which already excludes a signature-position `(quote x)` from reification so a def
named `quote` still binds.)

**The pass (increment 2b), per call `(f a b)`:** (1) resolve the head via
`callee_def_index_for_infer` (`infer.rs:5948`) to a def; (2) if its params carry `quote_params`
markers, reify each marked argument to its `Ast` with `quote::reify` (`quote.rs:419` — expose
`pub(crate)`, or a `reflect_document`-style wrapper), leaving eager args unchanged; (3) β-reduce the
call with `eval::apply_lambda` (`eval.rs:1057`) to a result `Ast` occurrence; (4) reconstruct that
`Ast` back to source with `eval_ast::reconstruct` (`eval_ast.rs:356` — expose `pub(crate)`);
(5) splice at the call site via the overwrite-original-slot / blank-appended-root idiom
(`eval_ast.rs:155-163`, shared with `desugar_eval`/`tagged_template`/`reify_quotes`); (6) **iterate to
a fixpoint** — spliced output is new source that may contain further macro calls and whose names need
resolving, so the pass rebuilds the resolution indices and re-scans until no macro call remains, then
hands off to type-checking. Step 6's resolution-rebuild-per-round is the one genuinely new mechanism
(the load-time desugars are single-pass because their output needs no callee resolution); keep it
bounded by only re-resolving when a splice occurred.

## 5. Capability 3 — gensym

No general fresh-name facility exists (only `unify::Fresh` for *type* variables, and one ad-hoc
`{spelling} $capture{n}` unreadable-name mint in the eval hygiene pass, eval_ast.rs:222). Add a
`gensym` compile-time primitive returning a fresh `Ast.Name` guaranteed collision-free — reuse the
"embed a character the reader can never produce" trick (a space) so a generated name cannot clash
with any source or other gensym. Small, self-contained; folds like the other `Ast.*` intrinsics
(ast_reflect.rs). Surface: `(gensym)` or `(Ast.gensym)` → `Ast`, pure.

## 6. Capability 4 — hygiene (RATIFIED fork c: PRESERVE by default + explicit opt-out)

**Operator ratified (2026-09-01): preserve hygiene BY DEFAULT** ("let's try keeping hygiene in by
default. But it would be nice to have a way to opt out where you want as well"). So macros are
hygienic by default via per-splice provenance+rename, **plus** an explicit opt-out for a deliberately
capturing (unhygienic) macro.

**Mechanism (both halves).** We do **not** need a per-node scope-set. The existing `eval` hygiene
preserves hygiene via a **node-provenance boundary** (nodes `< original_len` are spliced caller
syntax; `≥ original_len` are macro-introduced) plus a **targeted alpha-rename**
(`rename_captured_binders`, eval_ast.rs:181-234). Generalize to macro output, **both directions**:
(direction 1, :138) rename macro-introduced binders (backed by `Ast.gensym`, #7274) so they cannot
capture caller names; (direction 2, :140) mark macro-introduced references by provenance so caller
binders cannot capture them (the reference-side dual — the one genuinely new piece; binder-side
already exists for `eval`). Tracks provenance **per splice**, not a scope-set per name node.

**Spec (ratified).** v-spec-oracle lands the faithful-mirror **:142 wording tweak** (mechanism →
provenance-general; :138/:140 stay verbatim) — no longer held. And a **:140 direction-2 corpus
witness** (an introduced reference NOT captured by a use-site binder) lands with v-spec-oracle once the
reference-side dual ships; it needs the macro-**function** shape (def-site ≠ use-site), and its exact
spelling depends on the realized expander (increment 2b) — co-finalized then.

**Opt-out (new requirement, realizes the spec's "unless explicit" clause).** :138/:140 are both "…
unless the macro explicitly requests it", so a deliberately-capturing macro is already spec-anticipated
— likely **no further spec change** beyond making the "unless explicit" mechanism concrete. Design the
opt-out **surface**: how a macro author marks an introduced name as intentionally resolved in the
caller's scope (candidates: an anti-hygiene / "inject unhygienically" marker on the introduced name, or
naming a captured identifier through the `Eval` effect's caller env). Co-design with v-spec-oracle;
surface to the operator if a real design choice, else note it. This is increment 4's second half.

## 7. Forks — ALL DECIDED (2026-09-01)

1. **Unevaluated-param surface** — DECIDED: the `quote` approach, no new sigil (§2).
2. **Caller-env eval** — DECIDED: in v1, as an EFFECT, cross-lane with v-effects (§3).
3. **Hygiene** — DECIDED: preserve if feasible; amendment on hold; v-spec-oracle proposal (§6).
4. **Naming** — DECIDED: `Ast.gensym` (§5, landed #7274).

## 8. Increments (corpus-pinned, direct-to-main, one PR each)

1. **`Ast.gensym`** — ✅ LANDED #7274 (fresh, collision-free, distinct per call site, stable per
   binding; the manual-hygiene substrate).
2. **Quote-based unevaluated parameters + the expander** — the `quote`-in-binder marker + the
   post-resolve reify-and-splice expander; a first macro (`unless`/`swap`) as a plain function,
   corpus-pinned to its expansion.
3. **Caller-env-eval `Eval` EFFECT** (with v-effects) — the general `Eval` effect with the `in-caller`
   op; **explicit `{Eval}` row acquisition** (composes to nested fns) + per-call-site ambient discharge
   giving **direct-caller** env semantics (§3); corpus-pin a macro that evaluates a caller expression
   in the (direct) caller's env. Blocked on increment 2 + the v-effects acquisition/discharge co-design.
4. **Hygiene — PRESERVE, both halves + opt-out** (with v-spec-oracle) — generalize provenance+rename to
   macro output: direction-1 binder-rename (exists) + the direction-2 **reference-side dual** (new);
   land v-spec-oracle's faithful-mirror :142 tweak + the :140 direction-2 corpus witness; then the
   explicit **opt-out** surface for a deliberately-capturing macro (§6). Blocked on increment 2 (operates
   on the expander's spliced output) + the reference-dual design.

Each increment gates: corpus (`nix build .#checks…corpus-…`), ml_surface round-trip, nativize,
clippy `-D warnings`, pinned fmt. No runtime/hash change expected (front-end expansion + folds).
Increment 2 is unblocked now; 3 and 4 land as their cross-lane co-designs (v-effects, v-spec-oracle)
resolve.
