# Design — irrefutable binding patterns for `param` / `let` (rcdzc)

**Author:** design pass (compiler). **Audience:** the implementer picking up rcdzc task #153, + future me.
**Status:** proposal / handoff — **nothing landed**. Revised after a 3-lens adversarial review (see §0).
This is the plan for letting a *binding position* (a function parameter, a `let` binder, a `fn` parameter,
a `do`-block `def`) hold an **irrefutable pattern** — `(def (f (tuple a b)) …)`, `(let (((tuple a b) v)) …)`
— instead of only a bare name. It is written ahead of implementation, in the spirit of
`DESIGN-effects-rcdzc.md` and the ask-81 closures handoff: it states the target, the accept/decline
boundary, the pass-by-pass edits with line anchors, and the subtleties an implementer must get right.

The through-line: **a binding position that holds an irrefutable pattern is exactly a single-arm
destructuring match — which rcdzc already compiles.** So this increment adds **zero IR nodes** and
**zero infer/select/fold code for the tuple case**; it is a `resolve`-time desugar onto machinery that
is already proven green.

---

## 0. Review corrections (READ FIRST — supersede any stale phrasing below)

An adversarial review (verification lens + design/edge lens + spec-fidelity lens) confirmed the two
load-bearing claims — **"tuple binding pattern = single-arm tuple match with zero downstream change"**
(traced end-to-end through infer/lower/fold/select) and the **β-reduction/α-rename + shadowing**
subtleties (§9) — as **sound**. It also found defects that a literal implementer would hit. They are
fixed inline in the sections below; this block is the load-bearing summary.

- **C1 — spec-first is a PREREQUISITE, not optional.** No normative sentence or corpus case sanctions a
  pattern in a *binding* position today — the spec only defines patterns in `match`-arm ("pattern")
  position and sub-pattern ("binder") position (core-semantics.md §*A Tuple Is Deconstructible By Pattern
  Matching* :165 says "in pattern position"). The constitution makes the corpus the source of truth
  (compiler is a projection of the spec, not its source). **So the FIRST step is: add the normative
  capability sentence to `core-semantics.md` + author the corpus witnesses — THEN the resolve desugar.**
  This doc specs the mechanism; it does not license building ahead of the corpus. Added as step 0 in §10.
- **C2 — refutable binding pattern → `CDZ0210`, NOT `CDZ0201`.** By this doc's own "it IS a single-arm
  match" thesis, `(let (((Some x) o)) x)` = `(match o ((Some x) x))` = a *non-exhaustive* single-arm match,
  which the corpus already codes **`CDZ0210`** (`02-binding-and-control.sexp` §"a sum match missing a
  variant is non-exhaustive…", the Some-only match). `CDZ0201` is correct ONLY for a *shape-incompatible*
  pattern (a wrong-arity tuple, a kind mismatch) that can NEVER match. Split the two (§4, §5.2, §7).
- **C3 — the classifier MUST consult the prelude (variant count), not scan head strings.** As first
  drafted, `check_irrefutable` rejected *every* constructor head with `CDZ0201` — which (a) wrongly
  hard-rejects a single-variant-sum binder that §3/§7/§8 say should *decline* (Increment B), and (b)
  routed an annotated binder `(: x T)` (a `List` head `":"`) into the ctor-reject, contradicting §6's
  accept. Fixed in §5.2: peel `:` first; resolve a ctor head against the prelude exactly as
  `collect_binders` (`resolve.rs:1073`) does; single-variant → decline, multi-variant/literal/list → the
  non-total reject (`CDZ0210`), and a bare nullary ctor (`None`) is a ctor, not a binder.
- **C4 — the linearity recipe as first written was DEAD.** `collect_binders` (`resolve.rs:1075`) already
  dedupes (`if !out.contains`), so a "set-insert over its output" can NEVER observe a repeat → `CDZ0102`
  would never fire and the ungated corpus cases would FAIL. Fixed in §5.5: use a NON-deduping collector
  (or detect the collision inside the walk).
- **C5 — `bind_irrefutable`'s `&Node` body can't carry a multi-binding `let`.** The continuation of a
  `let` chain is `(&[Node] rest, &Node body)`, not one node, so "reuse `resolve_arm` unchanged" is false
  for the common multi-binding case. Fixed in §5.1: the helper takes a resolve-*continuation* (a
  `FnOnce(&mut Self, &Scope) -> Hir`) or a pre-resolved `Hir` for the arm body, not a raw `&Node`.
- **C6 — accuracy nits.** `(cons h t)` is invented syntax → the spec's list-pattern surface is
  `(list x .. rest)`; a *zero-leading rest binder* `(list .. rest)` IS irrefutable (so the blanket "list
  patterns are refutable" rationale is right-outcome-wrong-reason — corrected in §3/§8). The empty-tuple
  binder `(tuple)`/`()` rides an unresolved `Ty::Tuple([]) ≠ Ty::Unit` quirk in `unify` (`ty.rs`) — §3
  now excludes arity-0 tuple patterns pending that reconciliation. Line anchors into
  `05-compound-types.sexp` are the two `CDZ0102` cases at **:2561** (flat) and **:2581** (nested).

---

## 1. TL;DR — the win, the insight, the pick

**The win.** Today every parameter and `let` binder is a bare name. To take a tuple apart you must bind
it whole and project each element by hand:

```lisp
; today — the self-host decoder's actual shape (implementation/compiler/cdzc/15-decode.cdz:139)
(def (decode-node bytes i)
  (let ((r (decode bytes i)))          ; r : (tuple <Ast> <next-offset>)
    (match r ((tuple ast pos) ast))))  ; one-arm match just to name the two halves
```

Every tree-walking pass over a `(tuple <node> <offset>)` accumulator — the whole CBOR decoder, every
`decode-*`, the parser's position threading — pays this bind-then-rematch tax. Binding patterns let the
same code read:

```lisp
(def (decode-node bytes i)
  (let (((tuple ast pos) (decode bytes i)))   ; name both halves at the binding
    ast))
```

and, at the parameter itself:

```lisp
(def (fst (tuple a b)) a)     ; f still takes ONE argument — a pair — and names its parts
```

**The insight.** rcdzc **already** compiles a single-arm tuple-scrutinee match:
`(match t ((tuple a b) (+ a b)))` works today (`tests.rs:248`, `infer.rs:671`, `select.rs:561`). That
path binds a tuple pattern's elements against a scrutinee handle by `arr-get`, with **exact-arity**
inference and no discriminant. **An irrefutable binding pattern is that match, one arm, generated by
`resolve` instead of written by hand.** So:

- `(let (((tuple a b) v)) body)` desugars to `(let ((g v)) (match g ((tuple a b) body)))` — or, since the
  match path takes any scrutinee expression, directly to `(match v ((tuple a b) body))`.
- `(def (f (tuple a b)) body)` gives the parameter its own fresh anonymous local `g` (**arity is
  preserved** — `f` still takes one argument) and rewrites the body to `(match (Local g) ((tuple a b) body))`.

**The pick (see §7).** Ship **Increment A** now: `name`, `_`, and **tuple** patterns (nested to any
depth) in every binding position, plus the linearity check (`CDZ0102`) they finally make reachable — a
pure `resolve` desugar, **no infer/lower/fold/select/serialize change**. Architect but **defer Increment
B**: record patterns and single-variant-sum patterns in binding position (these need net-new *pattern*
support in `infer_pattern`/`emit_match`, since rcdzc's match path only knows tuples and multi-variant
sums today). Support **optional annotations** `(: <pat> <Type>)` on a binder as a thin `Annot` wrapper
(§6). Increment A declines every B case honestly (reject-don't-miscompile), never miscompiles.

---

## 2. Target surface syntax

All four binding positions accept the same pattern grammar. Grounded in the corpus vocabulary
(`spec/semantics/`, `core-semantics.md`):

```lisp
; 1. FUNCTION PARAMETER — arity preserved; the parameter is a pair, named apart
(def (fst (tuple a b)) a)                 ; (fst (tuple 7 8)) => 7
(def (add-pair (tuple a b)) (+ a b))      ; (add-pair (tuple 3 4)) => 7

; 2. LET binder
(let (((tuple a b) (mk-pair)))  (+ a b))
(let (((tuple a (tuple b c)) v)) (+ a (+ b c)))   ; nested, any depth

; 3. FN (lambda) parameter — rides the same desugar as (def …) params
((fn ((tuple a b)) (+ a b)) (tuple 3 4))  ; => 7

; 4. DO-block declaration (a value-def whose LHS is a pattern) — nice-to-have, §5.4
(do (def (tuple a b) (mk-pair)) (+ a b))

; wildcard / name are just the degenerate patterns (already work; now uniform)
(let ((_ (side-effect)))  42)             ; discard, explicitly
(def (f x) x)                             ; a name is the trivial irrefutable pattern

; OPTIONAL ANNOTATION on any binder (§6)
(def (f (: x Int64)) x)
(let (((: (tuple a b) (Tuple Int64 Int64)) v)) a)
```

**What is NOT a binding pattern** (refutable → rejected in binding position, §4):
`(Some x)`, `(Ok v)`, `0`, `true`, `"lit"`, and a length-constrained list-element pattern
`(list a b)` / `(list x .. rest)`. These may only appear in a `match` arm, where a sibling arm covers the
other cases. (The one *irrefutable* list pattern — a zero-leading rest binder `(list .. rest)`, which
matches every list — is not excluded on refutability grounds; it is simply out of scope for this
increment along with all list patterns, gated `(needs list-patterns)`. See §8. Note the spec's
list-pattern surface is `(list x .. rest)` — there is no `(cons …)` form.)

---

## 3. Why "irrefutable", and the accept set

core-semantics.md §*Bindings Introduced By A Pattern* / §*Patterns Compose* / §*A Tuple Is Deconstructible
By Pattern Matching* define patterns and their linearity. A binding position (`let`, a parameter) has **no
alternative arm** — if the pattern failed to match there is nowhere to go. So a binding pattern must be
**irrefutable**: it matches *every* value of its type. The accept set is exactly the patterns that cannot
fail:

| pattern | irrefutable? | binding position |
|---|---|---|
| name `x` | yes (binds anything) | ✅ Increment A |
| wildcard `_` | yes (matches anything) | ✅ Increment A |
| `(tuple p₁ … pₙ)`, n≥1, each `pᵢ` irrefutable | yes — a tuple has ONE shape | ✅ Increment A |
| `(tuple)` / `()` (arity 0) | yes in principle | ⚠ EXCLUDED — rides `Ty::Tuple([]) ≠ Ty::Unit` (§9 F5) |
| `(record (k₁ p₁) …)`, each `pᵢ` irrefutable | yes — a record has ONE shape | ⏳ Increment B (decline) |
| single-variant user sum `(V x)` | yes — one variant | ⏳ Increment B (**decline**, not reject) |
| `(Some x)` / `(Ok v)` / any multi-variant ctor | **no** — the other variant exists | ❌ reject `CDZ0210` (§4) |
| a literal `0` / `true` / `"s"` | **no** — matches one value | ❌ reject `CDZ0210` (§4) |
| a length-constrained list pattern `(list x .. r)` | **no** — depends on length | ❌ reject `CDZ0210` (§4) |
| a zero-leading rest binder `(list .. r)` | yes — matches every list | ⏳ out of scope (all list patterns, §8) |

Increment A ships the top three (name, `_`, tuple of arity ≥ 1). They compose recursively — a tuple
element MAY itself be a name, `_`, or a tuple pattern, to any depth (`core-semantics.md` §*Patterns
Compose*) — and that recursion is **already handled** by `infer_pattern` (`infer.rs:793`, recurses tuple
subs) and `bind_payload` (`select.rs:677`, recurses the `Mir::Tuple` arm via nested `arr-get`).

**Note the two DIFFERENT non-accept outcomes** (the review's central correction, §0 C2/C3): a pattern that
is irrefutable-in-principle but not-yet-supported (record, single-variant sum, any list pattern)
**declines** (Increment B / later — reject-don't-miscompile); a pattern that is genuinely **refutable**
(a multi-variant ctor, a literal, a length-constrained list pattern) is an **ill-formed** binding and is
**rejected `CDZ0210`** (§4). Do not collapse these — a decline says "a later phase handles it," a reject
says "no total match exists here." The classifier (§5.2) must tell them apart by consulting the prelude,
not by scanning head strings.

---

## 4. Refutable-in-binding-position is a rejection — and the code is `CDZ0210`, not `CDZ0201`

A refutable pattern where the language guarantees a total match is an **ill-formed program**, not a
not-yet-supported construct. Reject it with a coded diagnostic + a message naming the offending
constructor/literal — do **not** silently accept, and do **not** emit a generic "unsupported" decline
(which would wrongly read as "a later phase will handle it").

**Which code?** This doc's own thesis is "a binding pattern IS a single-arm match" (§1, §5.3). Take that
seriously and read the corpus for what a single-arm match that fails to cover its type yields — the corpus
is unambiguous:

- `(match (Some 5) ((Some x) x))` — a Some-only arm, no None → **`CDZ0210`** (non-exhaustive)
  (`02-binding-and-control.sexp` §"a sum match missing a variant is non-exhaustive even when the scrutinee
  is the covered one").
- `(match true (true 1))` / `(match 5 (5 1))` — a single literal arm → **`CDZ0210`**
  (`02-binding-and-control.sexp` §"a bool match on a constant scrutinee is non-exhaustive…" / §"an int
  match on a constant scrutinee is non-exhaustive…").

So a **non-total** binding pattern (a multi-variant ctor, a literal, a length-constrained list pattern)
is **`CDZ0210`** — the same code the desugared single-arm match would emit anyway. Reserve **`CDZ0201`**
for a **shape-INCOMPATIBLE** pattern — one that can *never* match, regardless of coverage: a wrong-arity
tuple `(tuple a b c)` vs a 2-tuple, or a kind mismatch (a tuple pattern vs a sum value). That distinction
is exactly what the corpus draws (`02-binding-and-control.sexp` §"a tuple pattern of the wrong arity is a
type error" = `CDZ0201`; the Some-only match = `CDZ0210`).

```lisp
(let (((Some x) o)) x)     ; CDZ0210 — non-total: None is uncovered, no alternative arm
(def (f 0) …)              ; CDZ0210 — a literal covers one value of its type
(let ((true v)) …)         ; CDZ0210
(let (((tuple a b c) p)) …); CDZ0201 — SHAPE-incompatible if p is a 2-tuple (can never match)
```

Rationale: the split is decidable at compile time (§3 table, resolved via the §5.2 classifier). A
non-total binding is a *coverage* defect → `CDZ0210`; a shape-incompatible one is a *type/shape* defect →
`CDZ0201`. A generation that cannot yet classify may decline; once the classifier exists, a refutable
binding pattern is a reject with the coverage code.

**Bonus — the existing exhaustiveness check gives `CDZ0210` for free.** Because the desugar already emits
a single-arm `Hir::Match`, `infer`'s exhaustiveness pass (`infer.rs:712`, CDZ0210 over the sum's variant
set) would reject a refutable ctor binding *even without* the §5.2 classifier. The classifier's only jobs
are (a) a better message and a resolve-time (pre-infer) rejection, and (b) telling a *decline* (record /
single-variant sum) apart from a *reject* — which the raw exhaustiveness check cannot do. Keep it minimal.

---

## 5. Pass-by-pass changes

**The headline: the only file that changes for Increment A is `resolve.rs`.** Everything downstream
(infer/lower/fold/select/serialize) is untouched — the desugar targets the already-green single-arm
tuple-match path. `diag.rs` gains one new code (`CDZ0102`) for linearity (§5.5).

### 5.1 The desugar (the core), in `resolve.rs`

There is already an anonymous-local generator (`fresh_local`, `resolve.rs:670`), a scope-frame chain
(`Scope::Bind`, `resolve.rs:634`), a binder collector (`collect_binders`, `resolve.rs:1067`), and — the
keystone — `resolve_arm` (`resolve.rs:1047`), which collects a pattern's binders, allocates a fresh local
per binder, and resolves the pattern **and** an under-scope body into a `(pat-Hir, body-Hir)` pair. That
is *exactly* the arm a binding pattern desugars to. Reuse it.

Add one helper. **⚠ Two review corrections are baked in (§0 C3, C5):** (1) the body is a resolve
*continuation*, not a raw `&Node` — because a `let`'s "rest" is `(&[Node] bindings, &Node body)`, which no
single `&Node` denotes (see the `let` routing below); (2) an annotation `(: <pat> T)` is peeled *before*
classification, or it would fall into the `List` arm and be misclassified as a refutable constructor.

```
/// Resolve a BINDING-POSITION pattern + the continuation it scopes into a single-arm irrefutable match.
/// `scrutinee` is the resolved value being bound (a `let` value, or `Local(g)` for a parameter).
/// `cont(self, scope)` resolves whatever the binders scope over (the rest of a let-chain + body, or a
///   function body) UNDER the passed scope — so a multi-binding let threads its remaining bindings here.
/// Returns the continuation's Hir, wrapped so the pattern's binders are in scope in it.
fn bind_irrefutable(&mut self, pat_node: &Node, scrutinee: Hir,
                    cont: impl FnOnce(&mut Self, &Scope) -> Hir, scope: &Scope) -> Result<Hir, Reject> {
    match pat_node {
        // ── (: <pat> T) — peel the annotation, wrap the SCRUTINEE in Hir::Annot, recurse on <pat> (§6).
        Node::List(items) if items.first().and_then(name_of) == Some(":") && items.len() == 3 => {
            let ty = self.expr(&items[2], scope);                       // the type expression
            let annotated = Hir::Annot(Box::new(scrutinee), Box::new(ty));
            return self.bind_irrefutable(&items[1], annotated, cont, scope);
        }
        // ── bare name / wildcard: NOT a destructuring — bind directly (no match), so the trivial cases
        //    stay byte-identical to today's let/param handling. (`_` = a throwaway drop, reuse do_seq idiom.)
        Node::Name(n) if n == "_" => { /* fresh throwaway local, bind scrutinee, cont under `scope` */ }
        Node::Name(n)             => { /* fresh local id, Scope::Bind {name:n}, cont under the new frame */ }
        // ── a destructuring pattern → a single-arm match. The pattern's binders scope over `cont`.
        Node::List(_) => {
            self.check_irrefutable(pat_node)?;      // §5.2 — decline (Increment B) OR reject CDZ0210
            self.check_linear(pat_node)?;           // §5.5 — reject a repeated binder (CDZ0102)
            // Collect binders, alloc a fresh local each, resolve the pattern AND the continuation under
            // them (this is `resolve_arm`'s body, generalized from `&Node` to a continuation — see C5).
            let binders = self.pattern_binders(pat_node);
            let ids: Vec<u32> = binders.iter().map(|_| self.fresh_local()).collect();
            let named: Vec<(&str,u32)> = binders.iter().copied().zip(ids).collect();
            let pat  = resolve_with_params(self, &named, pat_node, scope);
            let body = /* run `cont` under the `named` frames chained onto `scope` */ ;
            Ok(Hir::Match { scrutinee: Box::new(scrutinee), arms: vec![(pat, body)] })
        }
    }
}
```

`resolve_arm` (`resolve.rs:1047`) is the *shape* to copy — it collects binders, allocs a fresh local each,
and resolves pattern+body under them — but it hard-codes a `&Node` body, so `bind_irrefutable`
**generalizes** it to a continuation rather than *calling* it. (A one-binding `let` or a function body,
whose body IS a single node, can still delegate to `resolve_arm` directly; the continuation form is
required only for the multi-binding-`let` case.)

Then route the four binding positions through it:

- **`let`** (`let_chain`, `resolve.rs:1188`). Today the binding name is read by `name_of(&kv[0])` and a
  non-name declines (`resolve.rs:1196`). Change: the LHS `kv[0]` may be any pattern. Resolve the value
  `kv[1]` first (in the *outer* scope — a binding's initializer does not see its own binders), then call
  `bind_irrefutable(kv[0], value, |r, inner| r.let_chain(rest, body, inner), scope)` — **the continuation
  is the recursive `let_chain` over the remaining bindings** (this is the C5 fix: the "rest of the let" is
  bindings-plus-body, which is why a raw `&Node` body cannot express it). `let*` sequencing
  (`02-binding-and-control.sexp` §"a later let binding sees an earlier one") is preserved: each binding's
  binders enter scope for the bindings that *follow* because the continuation resolves `rest` under the
  binders' frame. (Verified against the review's multi-binding example — scoping flows correctly; the only
  failure mode is an implementer who passes the *final* body node and silently drops the intervening
  bindings, which the continuation shape prevents.)

- **function parameter** (`resolve.rs:315`). Today `def.params` are collected as names (`collect_def`,
  `resolve.rs:586`) and a non-name declines (`resolve.rs:590`). Change: a parameter may be a pattern.
  **Arity is unchanged** — a destructuring parameter still occupies one argument slot. Give each parameter
  its own fresh local `g` (as today), but if the parameter is a pattern, wrap the body:
  `body = bind_irrefutable(param_pat, Hir::Local(g), original_body, scope)`, nesting one wrap per
  destructuring parameter (outermost = first parameter). A plain-name parameter keeps today's zero-cost
  `Scope::Bind` path.

- **`fn` lambda parameter** (`lambda`, `resolve.rs:1221`). Same rewrite as a `def` parameter: a
  destructuring `fn` param binds an anonymous local and wraps the lambda body in the match. `Hir::Lambda`'s
  `params` stay the anonymous locals — the lambda's arity is unchanged, so β-reduction (ask-81) and the
  spine-collapse still see a fixed arity. (A lambda applied in place then β-reduces the match's scrutinee
  to a concrete `Mir::Tuple`; see §8 "the fold reduces a constructed scrutinee".)

- **`do`-block value-def** (`do_seq` + `is_value_def`, `resolve.rs:1134`/`1248`) — §5.4, a nice-to-have.

### 5.2 Classifying refutability (`resolve.rs`, new scan — MUST consult the prelude)

**⚠ This is the section the review most faulted (§0 C3).** The classifier CANNOT be a pure head-string
scan, because it must (a) tell a bare *constructor* name apart from a *binder* name — `None` is a ctor,
`x` is a binder — and (b) tell a *single-variant* sum (irrefutable → **decline**, Increment B) apart from
a *multi-variant* one (refutable → **reject `CDZ0210`**). Both facts live in the prelude / `SumDef`, which
`collect_binders` (`resolve.rs:1073`) already consults — so this function must consult it the SAME way, or
the two drift (the review found the first draft classified `None` as an irrefutable binder and hard-rejected
every ctor with `CDZ0201`).

```
/// Classify a binding-position pattern. Ok(()) = irrefutable, Increment A (name / `_` / tuple-of-those).
///   Reject::decline    = irrefutable-in-principle but a LATER increment (record; single-variant sum; any
///                        list pattern) — reject-don't-miscompile, flips to accept when B lands.
///   Reject::coded(0210) = genuinely REFUTABLE (multi-variant ctor / literal / length-constrained list) —
///                        no total match exists; an ill-formed binding (§4).
///   Reject::coded(0201) = shape-INCOMPATIBLE (a wrong-arity tuple) — caught downstream by infer's exact-
///                        arity unify (§5.3); the classifier need not pre-empt it, but MAY for a message.
fn check_irrefutable(&self, pat: &Node, scope: &Scope) -> Result<(), Reject> {
    match pat {
        // A bare name: a binder UNLESS it resolves to a ctor (mirror collect_binders, resolve.rs:1073).
        Node::Name(n) if n == "_" => Ok(()),
        Node::Name(n) => match self.prelude.get(n.as_str()) {
            Some(Hir::Ctor { def, .. }) => self.classify_ctor(def),   // bare nullary ctor (`None`) — not a binder
            _ => Ok(()),                                             // a genuine binder
        },
        Node::Int(_) | Node::Bool(_) | Node::Str(_) =>
            Err(Reject::coded(Code::NonExhaustive,                    // a literal covers ONE value → CDZ0210
                "a literal pattern is refutable — it cannot appear in a binding position")),
        Node::List(items) => match items.first().and_then(name_of) {
            // an annotation is peeled by bind_irrefutable BEFORE this runs, but be defensive:
            Some(":") if items.len() == 3 => self.check_irrefutable(&items[1], scope),
            Some("tuple") => items[1..].iter().try_for_each(|p| self.check_irrefutable(p, scope)),
            Some("record") => Err(Reject::decline("a record binding pattern is a later increment (B)")),
            Some("list")  => Err(Reject::decline("a list binding pattern is a later increment")),
            // a ctor head: bare `(Some x)` (head resolves to a Ctor) or qualified `((. T V) x)`.
            _ => match self.resolve_pattern_head_ctor(&items[0], scope) {
                Some(def) => self.classify_ctor(def),                 // decline if 1 variant, else CDZ0210
                None      => Err(Reject::coded(Code::TypeError,        // not a ctor at all → shape error 0201
                    "a binding pattern head is not a tuple, record, or constructor")),
            },
        },
    }
}
/// A sum ctor in binding position: single-variant → irrefutable-but-later (DECLINE); else REFUTABLE (0210).
fn classify_ctor(&self, def: &SumRef) -> Result<(), Reject> {
    if def.variants().len() == 1 { Err(Reject::decline("a single-variant-sum binding pattern is Increment B")) }
    else { Err(Reject::coded(Code::NonExhaustive,
        "a multi-variant constructor pattern is refutable — the other variants are uncovered")) }
}
```

`resolve_pattern_head_ctor` is the same head resolution `collect_binders`/`resolve_arm` already perform (a
bare name → `prelude.get`, a `(. T V)` → the qualified-ctor lookup that resolves to `Hir::Ctor`); factor
it out so `collect_binders`, the classifier, and `resolve_arm` share ONE notion of "is this head a ctor,
and of which sum" — the review's "keep them parallel or they drift" finding. `Code::NonExhaustive` is the
existing `CDZ0210` (`diag.rs`); the classifier adds no new code for the refutable case (only `CDZ0102` for
linearity, §5.5, is new).

### 5.3 `infer.rs` / `lower.rs` / `fold.rs` / `select.rs` — NO CHANGE (tuple case)

This is the payoff of the desugar. Trace `(def (fst (tuple a b)) a)` end-to-end:

1. **resolve** → `HirFunc { arity: 1, body: Match { scrutinee: Local(g), arms: [(Tuple([Local a, Local b]), Local a)] } }`.
2. **infer** (`infer_match`, `infer.rs:656`). The parameter's signature var is `v₀` (`infer.rs:39`). The
   match infers the scrutinee `Local(g) : v₀`, then `infer_pattern(Tuple([a,b]), v₀)` (`infer.rs:793`)
   unifies **`v₀ = Tuple([v₁, v₂])` at the pattern's exact arity** and binds `a:v₁`, `b:v₂`. By the
   tuple-scrutinee guard at `infer.rs:677` the substitution now shows `Ty::Tuple(_)`, single arm, tuple
   pattern → accepted. Body `Local a : v₁`; `fst : Fn([Tuple(v₁,v₂)], v₁)`. **Exact arity, no infer edit.**
   The `TODO fix this!` under-constrained-arity hazard at `infer.rs:468` (tuple *projection* on a var pads
   to *minimum* arity) is **avoided** — the desugar uses a tuple *pattern* (exact) not a projection (min).
3. **lower** → `Mir::Match` (`lower.rs:98`), pattern tree preserved.
4. **fold** (`fold.rs:656`) — folds the scrutinee and arm body; a `Local(g)` scrutinee is not const, so the
   match survives to select unchanged. ✓
5. **select** (`emit_match`, guard `select.rs:565`) — the `Ty::Tuple` single-arm path: the scrutinee handle
   IS the tuple `arr`; `bind_payload` (`select.rs:656`) `arr-get`s each element into the binder's slot. ✓

The only reason this works with zero downstream change is that **the tuple single-arm match is already a
shipped, tested feature** (`tests.rs:248`; corpus §"resolving a name in a shadowing environment" and the
whole decode idiom lean on it). The desugar just *emits* it.

**One inherited (not new) limitation the review flagged:** a *never-called, polymorphic* destructuring def
`(def (fst (tuple a b)) a)` with no call site declines at `finalize` ("unsolved type variable") because
rcdzc has no let/def generalization — but this is IDENTICAL to today's `(def (id x) x)` uncalled, not a
gap this feature introduces. The moment it is called (`(fst (tuple 7 8))`) the element vars solve and it
grounds. The "zero downstream change" claim is about *code*, and it holds.

### 5.4 `do`-block value-def with a pattern LHS (nice-to-have)

`is_value_def` (`resolve.rs:1248`) currently requires `items[1]` to be a name. Extend it to return the raw
LHS node; `do_seq` (`resolve.rs:1146`) then routes a pattern LHS through `bind_irrefutable` exactly like a
`let`. Low value (the corpus uses `let` for destructuring, `do`-`def` for names/functions), so ship it
only if free; otherwise a `(def (tuple a b) v)` in a `do` declines cleanly.

### 5.5 Linearity — the new diagnostic `CDZ0102`

core-semantics.md §*Bindings Introduced By A Pattern* / §*Patterns Compose*: "a pattern MUST bind each
name at most once … a name appearing in more than one sub-pattern is the same `CDZ0102` error as one
appearing twice in a flat pattern." rcdzc has **no `CDZ0102`** today (`diag.rs` enum stops at `CDZ0305`)
and no linearity check anywhere. Binding patterns are the first place it becomes reachable, and the corpus
already pins both facets, gated `(needs linear-patterns)`:

- flat: `(match (tuple 1 2) ((tuple x x) x) …)` → CDZ0102 (`05-compound-types.sexp`, the `(case …)` at
  **:2561**, `(error CDZ0102)` at :2569)
- nested: `(match (tuple 1 (tuple 2 3)) ((tuple x (tuple x y)) x) …)` → CDZ0102 (`05-compound-types.sexp`,
  `(case …)` at **:2581**; the learning
  `2026-07-08-pattern-linearity-must-be-pinned-across-sub-patterns-not-only-flat.md` is explicitly the
  tripwire: a shallow-only check FAILs the nested case). These are the ONLY two `CDZ0102` cases in the
  whole corpus, and BOTH are `match` arms.

**Do this right the first time — check across the WHOLE pattern, recursively.** Add
`Code::PatternNonLinear => "CDZ0102"` to `diag.rs`. Then — **⚠ the review caught a dead-code trap here
(§0 C4)** — you CANNOT reuse `collect_binders` (`resolve.rs:1067`) for the duplicate detection: it already
dedupes (`resolve.rs:1075`, `if !out.contains(&n) { out.push(n) }`), so its output never holds a repeat
and a "set-insert over its result" would NEVER fire `CDZ0102` (the ungated corpus cases would silently
FAIL, not pass). Instead, either (a) add a NON-deduping sibling collector that pushes every binder
occurrence, then scan for a duplicate; or (b) detect the collision *inside* the recursive walk — thread a
`HashSet<&str> seen` and return `CDZ0102` the first time an insert finds the name already present. Both
are recursive to any depth (the same traversal `collect_binders` does); the recursion is what the nested
corpus case demands.

Wire the check into `bind_irrefutable` (§5.1) **and** `resolve_arm` (`resolve.rs:1047`) so ordinary
`match` arms get it too. ⚠ Note: the two *existing* corpus cases are `match` arms, so they are closed by
the `resolve_arm` wiring ALONE — the `bind_irrefutable` wiring closes nothing currently pinned. So §10 MUST
add a binding-position `CDZ0102` case (`(let (((tuple x x) v)) …)`) or the binding-position half of the
wiring ships untested.

⚠ The linearity check is **anticipatory-corpus-pinned** (see the learning): implement the recursive,
non-deduping version — not the "immediate binders of one node" version, and not a set-insert over the
deduping `collect_binders` — or the nested corpus case FAILs.

---

## 6. Optional annotations on a binder — `(: <pat> <Type>)`

type-system.md §*Annotations Constrain, Never Contradict*: an annotation participates in inference as an
extra constraint (CDZ0203 on contradiction), and `(: e T)` is already `Hir::Annot` (`resolve.rs:826`,
`infer.rs:578`). So an annotated binder is a **thin wrapper** — but **⚠ the peel MUST happen in
`bind_irrefutable`'s dispatch (§5.1), NOT be assumed** (the review's §0 C3: as first drafted, `(: x T)`
fell into the classifier's `List` arm and was rejected as a refutable ctor). The `:`-arm added to
`bind_irrefutable` (§5.1) wraps the *scrutinee* in `Hir::Annot` and recurses on the inner pattern, so all
three cases below are ONE code path:

- **annotated value/let:** `(let (((: x Int64) v)) body)` → the `:`-arm wraps → `bind_irrefutable(x,
  Annot(v, Int64), cont)` → `x` binds the annotated value. The `Annot` unifies `v`'s type with `Int64`
  (CDZ0203 on mismatch).
- **annotated destructuring:** `(: (tuple a b) (Tuple Int64 Int64))` → wrap → `bind_irrefutable((tuple a
  b), Annot(value, (Tuple …)), cont)` — the annotation constrains the whole tuple *before* the pattern
  takes it apart.
- **annotated parameter:** `(def (f (: x Int64)) body)` — the parameter has no value node, so the
  scrutinee is the param's own local: the `:`-arm wraps `Local(g)` → `bind_irrefutable(x, Annot(Local(g),
  Int64), cont)`, and `x` binds it. **Coherence note (the review's "why both g and x?"):** `g` is the ABI
  parameter slot (arity stays 1), `x` is the binder the body references — they are distinct `fresh_local`
  ids, so there is NO double-bind. It is one redundant runtime copy (`x = Annot(Local g)` is a bare
  `Local`, so it does NOT β-reduce away — `is_const`/`is_transient` reject a `Local`), leaving a live but
  harmless `Mir::Let`. Acceptable (correct, one extra local); if the copy ever matters, annotate the
  parameter's *signature var* directly instead (a small infer-side hook), but that is not needed for
  correctness. It needs no infer change today (the `Annot` arm at `infer.rs:578` already does the unify).

**Scope note — type-valued / generic params: a NEW mode, NOT a reuse of this path.** type-system.md §*A
Position That Binds A Type-Valued Parameter … MUST Be A Bidirectional-Checking Boundary* and §*Generics
Are Type-Valued Parameters*: an annotated **type-valued** parameter (`(def (id (: x T) (: T Type)) x)`) is
where generics plug in (#150). **⚠ The review (§0, F3) caught a trap in the earlier phrasing:** the spec
says a type-valued-parameter position is a boundary at which a type is "synthesized by monomorphization …
or checked against an explicit annotation, **rather than solved by unification**." The monomorphic path
here IS `Annot`→unify — which is exactly what the spec forbids for the *type-valued* case. So #150 must
NOT "inherit" this unify path; it plugs in a **distinct bidirectional-checking mode**. The monomorphic
handling is valid *only because* its annotation is a concrete type over the term core (a value param, not
a type param). Keep the two mechanisms separate; do not phrase this as "widen the `Annot`→unify path."

---

## 7. Accept / decline / reject boundary (Increment A)

**Accept** (desugars onto the green tuple-match path):
- `name` / `_` in any binding position (unchanged behavior; now uniform).
- `(tuple p₁ … pₙ)` nested to any depth, each `pᵢ` a name/`_`/tuple — in `let`, `def` param, `fn` param,
  (and `do`-`def` if §5.4 shipped).
- an optional monomorphic annotation `(: <pat> <ConcreteType>)` on any of the above.

**Reject** (ill-formed — coded, never a silent accept):
- a **non-total (refutable)** pattern — a *multi-variant* constructor `(Some x)`/`(Ok v)`, a literal, a
  length-constrained list pattern → **CDZ0210** (non-exhaustive; §4 — this is the code the desugared
  single-arm match itself emits, NOT CDZ0201).
- a **shape-incompatible** pattern — a wrong-arity tuple `(tuple a b c)` vs a 2-tuple, or a pattern-kind
  mismatch → **CDZ0201** (falls out of the exact-arity unify at `infer_pattern`; `02-…sexp` §"a tuple
  pattern of the wrong arity is a type error").
- a **non-linear** pattern (a binder repeated, flat or nested) → **CDZ0102** (§5.5, the one NEW code).
- an annotation that contradicts the value/param type → **CDZ0203** (existing, §6).

**Decline** (irrefutable-in-principle but not-yet-supported — reject-don't-miscompile, flips to accept
when B lands; NEVER a CDZ0201/0210 reject, §0 C3):
- a **record** binding pattern `(record (k p) …)` → Increment B (net-new *pattern* support; §8).
- a **single-variant-sum** binding pattern `(W x)` → Increment B (rare; §8) — `check_irrefutable`
  distinguishes it from a refutable multi-variant ctor by the sum's variant count (§5.2 `classify_ctor`).
- any **list** binding pattern (even the irrefutable zero-leading rest binder) → out of scope with all
  list patterns (`(needs list-patterns)`).

---

## 8. Increment B — records + single-variant sums in binding position (DEFERRED)

Records and one-variant sums are irrefutable *in principle*, but rcdzc's match path does **not** know them
as patterns: `infer_pattern` (`infer.rs:746`) has arms for `Wildcard`/`Local`/`Int`/`Bool`/ctor-`Apply`/
`Tuple` only, and the single-arm-destructure guard (`infer.rs:677`) tests `Ty::Tuple(_)` only. So they are
genuinely new pattern kinds, not just new binding sugar — hence B, not A.

**Record patterns.** A record is represented identically to a tuple — a name-sorted positional `arr`
(`ty.rs:318`, `lower.rs:121` sorts fields) — so the runtime binding is the same `arr-get` walk. B adds:
1. `infer_pattern`: a `Hir::Record` pattern arm — unify `expected` with `Ty::Record(fields)` at the
   pattern's **exact field set** (each field a fresh var), infer each sub-pattern. (The precise-field-set
   constraint is why this must be a *pattern*, not the record *projection* desugar: a bare
   `Hir::RecordProj` on an unsolved var **declines** — "one field cannot pin the whole field set",
   `infer.rs:434` — so `(def (f (record (a x))) …)` on an unannotated param could not solve its type via
   projections. A record pattern pins the full field set at once, exactly as the tuple pattern pins arity.)
2. the single-arm guard (`infer.rs:677`): also admit `Ty::Record`.
3. `bind_payload`/`emit_match` (`select.rs:656`): a record pattern binds each field by `arr-get` at its
   **name-sorted slot** — the same sort `lower.rs:121` applies to a record literal, so pattern and value
   agree on slot order.

This is a bounded, self-contained extension of the tuple path (a few dozen lines across three files) and a
clean second increment. **A stopgap** — desugar a record binding pattern to `RecordProj`s *only when the
record type is already known* (an annotated param, or a `let` whose value is a record literal / a typed
expression) and decline on an unsolved var — is available if a record-destructuring case blocks self-host
before B lands; but the pattern approach is the real fix (it types unannotated params).

**Single-variant user sums.** `(type Wrap (W Int64))` makes `(W x)` irrefutable — its sole variant covers
the type. In Increment A, `check_irrefutable`'s `classify_ctor` (§5.2) already **declines** it (variant
count == 1 → `Reject::decline`, NOT a `CDZ0210` reject — the review's §0 C3 correction: rejecting it would
wrongly reject a valid program a later increment intends to accept). B then makes it *accept* by extending
`infer_pattern`/`emit_match` with a single-variant-sum pattern arm (the sum's one disc is unconditional, so
it is a payload bind with no discriminant guard — structurally the tuple path over the payload). Rare in
the corpus; lowest priority.

**List patterns are a separate feature entirely** — gated `(needs list-patterns)` /
`(needs list-pattern-runtime-tail)`, out of scope here. Note the refutability nuance the review flagged
(§0 C6): a *length-constrained* list pattern `(list x .. rest)` IS refutable (fails on the empty list), but
the zero-leading rest binder `(list .. rest)` matches *every* list and is irrefutable. Neither is in scope
regardless — but the classifier (§5.2) **declines** all list patterns rather than *rejecting* them, so the
irrefutable one is not mis-rejected when list patterns land.

**Empty tuple `(tuple)` / `()`.** Excluded from Increment A (§3 table). rcdzc resolves `()` to `Ty::Unit`
but `(tuple)` to `Ty::Tuple([])`, and `unify` (`ty.rs`) treats these as **distinct** — so
core-semantics.md's "the empty tuple IS the unit value" is not honored at the type layer, and a `(tuple)`
binder mis-rejects against a unit-typed value. This is a pre-existing rcdzc `Ty` quirk this feature would
merely ride; the binding-pattern increment excludes arity-0 tuple patterns until Unit/Tuple([]) identity is
reconciled (a separate `ty.rs`/spec item). A wildcard `_` is the correct way to bind-and-ignore a unit.

---

## 9. Subtleties an implementer must get right

- **Arity is preserved for a destructuring parameter.** `(def (fst (tuple a b)) a)` is a **one-argument**
  function whose argument is a pair — NOT a two-argument function. The parameter binds one anonymous local;
  the destructure happens in the body. Do not expand a tuple param into N wasm params, or `fst`'s
  export/call signature (`ir.rs:449`, ABI = the signature) and every call site break. (This is also why the
  `fn`-param case keeps `Hir::Lambda.params` = the anonymous locals: β-reduction and spine-collapse rely on
  a fixed arity — ask-81.)
- **A binding's initializer does not see its own binders.** Resolve the `let` value in the *outer* scope
  before introducing the pattern's binders (`let*` order is preserved because binders scope over *following*
  bindings + the body, via the continuation, §5.1 C5 fix). Getting this backwards would let
  `(let (((tuple a b) a)) …)` resolve the RHS `a` to the binder — an unbound-name bug. **Verified sound by
  the review** for the shadowing case too: `(def (f x v) (let (((tuple x y) v)) x))` — the value `v`
  resolves in the outer scope (cannot see the pattern's `x`), the pattern binders `x,y` get fresh ids via
  `fresh_local`, and the body `x` resolves nearest-binding-first (`Scope::Bind`, `resolve.rs:641`) to the
  pattern's `x`, not the param — correct shadowing, no fresh-id collision.
- **The continuation, not `resolve_arm` verbatim (§5.1 C5).** `resolve_arm` (`resolve.rs:1047`) is the
  right *shape* but hard-codes a single `&Node` body, which cannot express the "rest of a multi-binding
  `let`" (that is bindings-plus-body). `bind_irrefutable` generalizes the body to a resolve continuation.
  An implementer who copies `resolve_arm` literally and passes only the final body node **silently drops
  the intervening bindings** — the one real footgun here; the continuation shape prevents it.
- **The fold reduces a constructed scrutinee, so an in-place destructure of a literal still works.**
  `(let (((tuple a b) (tuple 3 4))) (+ a b))` desugars to `(match (tuple 3 4) ((tuple a b) (+ a b)))`; the
  fold keeps the `Mir::Tuple` scrutinee (`fold.rs:548`) and the select tuple path binds its elements — 7,
  compile-time or runtime alike. No const-propagation of tuple elements is required (nice, not needed).
- **fn-param β-reduction + α-rename — verified sound by the review.** `((fn ((tuple a b)) (+ a b)) (tuple
  3 4))` desugars to `Apply(Lambda{[g], Match{scrut: Local(g), …}}, [Tuple(3,4)])`. The fold's β-reduce
  α-renames the body's inner binders FIRST (`alpha_rename` recurses into `Match` arms) — so `a,b` get fresh
  ids above every module id — THEN substitutes `g := Tuple(3,4)` into the scrutinee; the tuple match then
  survives fold and select's tuple path destructures it. No capture. (A destructuring `fn` param that
  *escapes* to a non-inlinable HOF declines as a runtime closure exactly as a plain one does — ask-81 — no
  new miscompile.)
- **Linearity must be recursive AND non-deduping (§5.5, C4)** — a shallow "immediate binders" check passes
  the flat corpus case but FAILs the nested one; and a set-insert over the *deduping* `collect_binders`
  never fires at all. Use a recursive, non-deduping walk.
- **Refutable → CDZ0210 (coverage), not CDZ0201 (§4, C2).** A non-total binding pattern is the
  non-exhaustive-single-arm-match the desugar produces, so its code is `CDZ0210` — emit the coded rejection
  with the constructor/literal named. Reserve `CDZ0201` for a shape-incompatible pattern. Do NOT emit a
  generic "unsupported" decline for a refutable pattern (that reads as "a later phase handles it").
- **Wildcard-as-discard is a real drop.** `(let ((_ e)) body)` binds `e` to a throwaway local and drops it
  — reuse the `do_seq` discard idiom (`resolve.rs:1168`) so a discarded value still type-checks (an unbound
  name inside `e` is still CDZ0101). Do not elide `e` entirely.
- **New corpus cases must land un-gated for the accept path.** The tuple binding-pattern cases are a new
  capability with no `(needs …)` gate blocking them (the machinery ships in A) — add them as passing cases
  (after the normative spec sentence lands, §10 step 0). The record/single-variant cases land gated
  `(needs record-patterns)` / `(needs …)` so they skip until B. The linearity cases already exist gated
  `(needs linear-patterns)`; A ungates them (or the gate is retired) once CDZ0102 lands.

---

## 10. Spec-first prerequisite, corpus payoff & what it unblocks

**⚠ Step 0 — SPEC LEADS (the review's highest-priority finding, §0 C1).** No normative sentence or corpus
case sanctions a pattern in a *binding* position today — the spec defines patterns only in `match`-arm
("pattern") and sub-pattern ("binder") position, and the canonical "take a tuple from a parameter" idiom
in the corpus is *projection* (`(def (fst t) (tuple.0 t))`, `05-compound-types.sexp`) or explicit
bind-then-match. The constitution makes the executable corpus the source of truth (the compiler is a
projection of the spec, not its source). So the FIRST unit of work is **not** the desugar — it is:
1. add the normative capability sentence(s) to `core-semantics.md` (e.g. under a new §"A Binding Position
   Accepts An Irrefutable Pattern" or an extension of §*A Tuple Is Deconstructible By Pattern Matching*),
   defining that a `let` binder / parameter / `fn` param MAY hold an irrefutable pattern, desugaring to a
   single-arm match, and that a refutable one is `CDZ0210`; and
2. author the corpus witnesses below.
Only then does the resolve desugar (§5) implement a now-specified capability rather than invent surface.

**New `02-binding-and-control.sexp` cases to author** (the desugar makes these pass in Increment A):
- accept: a destructuring `let` `(let (((tuple a b) v)) …)`; a destructuring parameter `(def (fst (tuple a
  b)) a)`; a nested tuple binder `(let (((tuple a (tuple b c)) v)) …)`; a wildcard discard `(let ((_ e)) …)`;
  a multi-binding destructuring `let` where a later binding uses an earlier pattern's binder (the decoder
  idiom, pins the C5 continuation-scoping);
- an **annotated** binder — accept `(def (f (: x Int64)) x)`, and **reject** a *contradicting* annotation
  `(let (((: (tuple a b) (Tuple Int64 Bool)) v)) …)` → `CDZ0203` (the review's F4 omission);
- **reject** a refutable binding pattern — `(let (((Some x) o)) x)` → **`CDZ0210`** (the §4 rule needs a
  witness); a literal binder `(def (f 0) …)` → `CDZ0210`;
- **reject** a shape-incompatible binder — a wrong-arity tuple against a known 2-tuple → `CDZ0201`.

**New binding-position `CDZ0102` case** (the review's F4/§5.5 gap): `(let (((tuple x x) v)) …)` →
`CDZ0102`, gated `(needs linear-patterns)`. The two *existing* linearity cases are `match` arms, so they
exercise the `resolve_arm` wiring ONLY — this case is what pins the `bind_irrefutable` half.

**Two existing gated cases ungate:** the flat + nested linearity cases (`05-compound-types.sexp`, the
`(case …)` at **:2561** and **:2581**) once `CDZ0102` lands (§5.5).

- **The self-host decoder.** `implementation/compiler/cdzc/15-decode.cdz` threads `(tuple <Ast> <offset>)`
  through every `decode-*`; each currently pays the bind-then-`match` tax (a one-arm `(match … ((tuple ast
  pos) …))`, e.g. `:139`). Binding patterns cut a match per decode step and read the way the spec's own
  examples are written.
- **Expected corpus delta:** small-to-moderate (ergonomic, not a new value domain), but **on the self-host
  critical path** — the decoder/parser idiom — which is why #153 sits where it does.

---

## 11. Scope decisions (recommendations)

0. **Spec first (§10 step 0).** Land the normative `core-semantics.md` sentence + corpus witnesses BEFORE
   the desugar. Non-negotiable per the constitution (corpus = source of truth); the review flagged building
   ahead of the spec as the top issue.
1. **Increment split.** Ship **A** (name/`_`/tuple binding patterns + `CDZ0102` linearity) — a pure
   `resolve` desugar + one diag code, zero downstream risk. **Defer B** (records, single-variant sums) as a
   bounded follow-on that extends the match path itself. B *declines* (never rejects) in A.
2. **Annotations.** Include the **monomorphic** `(: <pat> <ConcreteType>)` wrapper in A (thin, reuses
   `Annot`; peeled in `bind_irrefutable`, §5.1). Leave **type-valued/generic** annotated params to #150's
   bidirectional seam as a **distinct checking mode** — do NOT phrase it as "widen the `Annot`→unify path"
   (the spec forbids solving a type-valued-param position by unification; §6 / §0 F3).
3. **`do`-`def` pattern LHS (§5.4).** Ship only if free; else decline cleanly.
4. **Linearity blast radius.** Wire the recursive, non-deduping `CDZ0102` check into BOTH
   `bind_irrefutable` and `resolve_arm`, gated on a full-corpus green (target: 0 regressions from 360; the
   two existing `match`-arm linearity cases + the new binding-position case move from skip→pass, nothing
   else should move).
5. **Diagnostic codes.** Refutable (non-total) binding → **CDZ0210** (not CDZ0201); shape-incompatible →
   CDZ0201; non-linear → CDZ0102 (new); contradicting annotation → CDZ0203. See §4.

## 12. Ladder placement & related

Task **#153** (params/let as optionally-annotated patterns), NEXT after **#152** (L2+ int widths) per
[[index-compiler-rewrite]]. It is a `resolve`-only lift that reuses the tuple single-arm match
(`tests.rs:248`) and the `Annot` node (first-class-types L1, #150). Its linearity code (`CDZ0102`) is
independent and closes two anticipatory-pinned corpus cases. It precedes effects (#148) and does not touch
`Lir`/`serialize`/`Layout`/`heap`.

Related: `spec/capabilities/core-semantics.md` §§*Bindings Introduced By A Pattern* / *Patterns Compose* /
*A Tuple Is Deconstructible By Pattern Matching*; `spec/semantics/02-binding-and-control.sexp` (the witness
this extends) + `05-compound-types.sexp` (linearity cases); the ask-81 closures handoff (the desugar-onto-
an-existing-reduction-tier pattern this mirrors); the learning
`2026-07-08-pattern-linearity-must-be-pinned-across-sub-patterns-not-only-flat.md` (the recursive-check
tripwire); `DESIGN-effects-rcdzc.md` (house style).
