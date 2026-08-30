# Design — irrefutable binding patterns for `param` / `let` (rcdzc)

**Author:** design pass (compiler), then a currency rewrite against the shipped query resolver.
**Audience:** the implementer of rcdzc task #153, + future me.
**Status:** Increment A LANDED (mostly). The `let` case is complete (resolution + validation + spec +
corpus + tests, landed on `spec`). The `def` parameter case is complete via a load-time rewrite (§5.2,
approach b). REMAINING: the `fn`-lambda parameter case (declines today — the rewrite covers top-level
`def`s, not inline lambdas) and a literal-ATOM parameter `(def (f 0) …)` (still silently accepted — a
pre-existing malformed-param gap, out of this feature's scope). Increment B (records) is deferred; the
**single-variant-sum** half of Increment B is now **ACTIVATED** (operator-greenlit 2026-08-30) — see
§8.1. Owners: v-compiler-primitives leads, joint with v-inference (resolve-side).

This is the plan for letting a *binding position* (a function parameter, a `let` binder, a `fn` parameter,
a `do`-block `def`) hold an **irrefutable pattern** — `(def (f (tuple a b)) …)`, `(let (((tuple a b) v)) …)`
— instead of only a bare name.

The through-line, unchanged from the original design and now confirmed by a spike: **a binding position
that holds an irrefutable pattern is exactly a single-arm destructuring match — which rcdzc already
resolves.** So this adds **zero IR nodes**. But — and this is the correction the shipped architecture
forces — the realization is NOT a "resolve-time desugar into an `Hir::Match`". rcdzc has no `Hir`/`Mir`,
no `Scope::Bind`, no `let_chain`. It is a **demand-driven query resolver**: a reference resolves by
walking *up* the AST to the nearest binder, and a tuple-pattern binder resolves to a `SumPayload` reading
the element out of the bound value. The whole feature is: teach that upward walk to look inside a
tuple-pattern binding position, and add a validation hook so an ill-formed binding pattern faults instead
of silently miscompiling.

---

## 0. Currency rewrite — READ FIRST (supersedes the original architecture framing)

The original doc (written against an earlier `rcdzc` shape) described the mechanism as a `resolve`-time
desugar of `(let (((tuple a b) v)) body)` into `Hir::Match { scrutinee, arms: [(Tuple([a,b]), body)] }`,
with edits at `resolve.rs` line anchors into `let_chain` / `collect_binders` / `resolve_arm` / a
`bind_irrefutable` continuation helper / `Scope::Bind` frames. **None of that machinery exists in the
current compiler.** The rewrite:

- **A1 — the resolver is DEMAND-DRIVEN, not scope-building.** There is no top-down scope chain and no
  desugar step. `resolve::resolve_name` (`resolve.rs:301`) resolves a bare name by calling `binder_in`
  (`resolve.rs:471`) on each enclosing form, ascending until one binds the name. A `let` body reference
  ascends into the `let` (Case 1) and looks the name up in the bindings-list; a parameter reference
  ascends into the `(fn …)`/`(def …)` (Cases 3/4) and resolves to a `Param` formal. There is nothing to
  "desugar into a match" — the binder lookup itself *is* where a tuple pattern is taught.
- **A2 — the tuple-match machinery to reuse is `find_binder_in_tuple` + `SumPayload`, already shipped for
  MATCH ARMS.** A `(match V ((tuple a b) …))` arm binder resolves through `match_arm_variant_binds`
  (`resolve.rs:807`) → `find_binder_in_tuple` (`resolve.rs:951`), which descends the pattern and returns
  `(scrutinee, path=[Elem(i)…], heads)`; the reference becomes `Resolved::SumPayload { scrutinee, steps,
  heads }` (`resolve.rs:557`). `SumPayload` is inferred (`infer.rs:156`: walk the scrutinee type down the
  `Elem`/`Payload` path), lowered (`lower.rs:156`: fold to the stored element, else emit a runtime
  `Core::SumPayload` read), and evaluated already. **A binding-position tuple pattern reuses this
  verbatim** — the only difference is the "scrutinee" is a `let` value occurrence (or a parameter formal),
  not a match scrutinee.
- **A3 — CDZ0102 linearity ALREADY LANDED for match arms + parameters.** `Code::NonLinearBinder =>
  "CDZ0102"` exists (`diag.rs:96`); `check_pattern_linear` / `collect_pattern_binders` (`lower.rs:1323`)
  is the recursive, non-deduping walk the original doc's §5.5 called for, wired into match-arm lowering
  (`lower.rs:1265`) and into duplicate-parameter checking. So §5.5's "CDZ0102 is the one new code" is
  DONE for its existing sites; binding patterns need only to *reach* the same check (and the two gated
  corpus cases can ungate — §0.2).
- **A4 — refutable → CDZ0210, shape-incompatible → CDZ0201, contradicting annot → CDZ0203.** These codes
  all exist (`diag.rs`: `NonExhaustive => CDZ0210`, `Malformed => CDZ0201`, `TypeMismatch => CDZ0203`).
  The original §4's code-choice reasoning stands unchanged (a non-total binding IS the non-exhaustive
  single-arm match the concept produces → CDZ0210; a can-never-match shape → CDZ0201). What changes is
  *where* they are emitted: not from a desugared `Hir::Match`, but from a **validation hook** on the
  `Resolved::Let` / parameter forms (§5, new).
- **A5 — the arity-0 / annotation / β-reduction subtleties (original §6, §9) survive with new anchors.**
  `Hir::Annot` is `Resolved::Annot` (`infer.rs:664`); the `(: pat T)` peel happens where the binding is
  read; the empty-tuple `Ty::Tuple([]) ≠ Ty::Unit` quirk is still real (§8). The α-rename/β-reduce
  discussion is moot — there is no β-reducing fold of an `Hir::Lambda`; a `fn` parameter resolves to a
  `Param` formal and infers a fresh var, so the "capture" concern does not arise the same way.

### 0.1 Spike results (VERIFIED — the `let` happy path works today)

Implemented in this worktree (`resolve.rs`, `last_binder_named`): a bindings-list lookup, when a pair's
LHS is a tuple pattern, descends it with `find_binder_in_tuple` and returns a `SumPayload` rooted at the
pair's value occurrence. Cases 1 (body) and 2 (later initializer) both route through it. Confirmed by
running the compiled component:

| program | result |
|---|---|
| `(let (((tuple a b) (tuple 3 4))) (+ a b))` | **7** ✓ |
| `(let (((tuple a (tuple b c)) (tuple 1 (tuple 2 3)))) (+ a (+ b c)))` | **6** ✓ (nested) |
| `(let (((tuple a b) (tuple 3 4)) (c (+ a b))) c)` | **7** ✓ (a later binding sees the pattern's binders — `let*`) |
| `(def (f p) (let (((tuple a b) p)) (+ a b)))` applied to `(tuple 10 20)` | **30** ✓ (RUNTIME scrutinee, not a literal) |

So the core mechanism is proven: **zero new IR, reuse `SumPayload`, works compile-time and runtime.**

### 0.2 Spike results (GAPS — an ill-formed binding pattern currently MISCOMPILES)

The same spike exposed that binding-position patterns get **no validation** — they silently do the wrong
thing where a match arm would fault. These are the work items §5 must close:

| program | got today | want |
|---|---|---|
| `(let (((Some x) (Some 5))) x)` — refutable | **CDZ0101 unbound `x`** | CDZ0210 (non-total) |
| `(let (((tuple a b c) (tuple 1 2))) a)` — wrong arity | **compiled, ran to a value** | CDZ0201 (shape) |
| `(let (((tuple x x) (tuple 1 2))) x)` — non-linear | **compiled, ran to a value** | CDZ0102 (linearity) |
| `(def (fst (tuple a b)) a)` — tuple PARAM | (not attempted — §5.2) | 7 |

The `(Some x)` case resolves to CDZ0101 because `find_binder_in_tuple` only descends `tuple` heads, so a
`Some` head binds nothing and `x` is unbound — a confusing message, not the intended coverage code. The
wrong-arity and non-linear cases are silent miscompiles: the lazy binder lookup finds `a`/`x` at their
`Elem` paths and never checks that the pattern is well-formed against the value's type or that it is
linear. **A binding-position pattern needs the same up-front validation a match arm gets** — see §5.

---

## 1. TL;DR — the win, the mechanism, the pick

**The win.** Today every parameter and `let` binder is a bare name. To take a tuple apart you must bind it
whole and project each element by hand:

```lisp
; today — the self-host decoder's actual shape (implementation/compiler/cdzc/15-decode.cdz:139)
(def (decode-node bytes i)
  (let ((r (decode bytes i)))          ; r : (tuple <Ast> <next-offset>)
    (match r ((tuple ast pos) ast))))  ; one-arm match just to name the two halves
```

Binding patterns let the same code read:

```lisp
(def (decode-node bytes i)
  (let (((tuple ast pos) (decode bytes i)))   ; name both halves at the binding
    ast))
```

and, at the parameter itself: `(def (fst (tuple a b)) a)` — `fst` still takes ONE argument (a pair) and
names its parts.

**The mechanism (corrected — see §0).** rcdzc resolves a reference by walking up to its binder; a
tuple-pattern match-arm binder already resolves to a `SumPayload` reading the element out of the
scrutinee. A binding-position tuple pattern is the same thing rooted at a `let` value / a parameter
formal. So:

- `(let (((tuple a b) v)) body)`: a body reference to `a` ascends into the `let`, finds the binding whose
  LHS is `(tuple a b)`, descends it with `find_binder_in_tuple`, and resolves to `SumPayload { scrutinee:
  v, steps: [Elem(0)] }`. (SPIKED — §0.1.)
- `(def (f (tuple a b)) body)`: a body reference to `a` ascends into the `def`, finds that the parameter
  is a tuple pattern, and resolves to `SumPayload` reading `Elem(0)` of the parameter formal. **Arity is
  preserved** — `f` still takes one argument. (NOT landed — §5.2.)

**The pick (see §7).** Ship **Increment A**: `name`, `_`, and **tuple** patterns (nested to any depth) in
every binding position, plus the validation that makes a refutable / wrong-arity / non-linear binding
fault instead of miscompiling. Architect but **defer Increment B**: record patterns and
single-variant-sum patterns in binding position (these need net-new *pattern* support — `find_binder_in_*`
only knows tuples and multi-variant sums today). Support **optional annotations** `(: <pat> <Type>)` on a
binder as a thin `Annot` wrapper (§6). Increment A declines every B case honestly, never miscompiles.

---

## 2. Target surface syntax

All four binding positions accept the same pattern grammar:

```lisp
; 1. FUNCTION PARAMETER — arity preserved; the parameter is a pair, named apart
(def (fst (tuple a b)) a)                 ; (fst (tuple 7 8)) => 7
(def (add-pair (tuple a b)) (+ a b))      ; (add-pair (tuple 3 4)) => 7

; 2. LET binder                                                    [SPIKED — works]
(let (((tuple a b) (mk-pair)))  (+ a b))
(let (((tuple a (tuple b c)) v)) (+ a (+ b c)))   ; nested, any depth

; 3. FN (lambda) parameter — rides the same param mechanism as (def …) params
((fn ((tuple a b)) (+ a b)) (tuple 3 4))  ; => 7

; 4. DO-block declaration (a value-def whose LHS is a pattern) — nice-to-have, §5.4
(do (def (tuple a b) (mk-pair)) (+ a b))

; wildcard / name are the degenerate patterns (already work; now uniform)
(let ((_ (side-effect)))  42)             ; discard, explicitly (works today)
(def (f x) x)                             ; a name is the trivial irrefutable pattern

; OPTIONAL ANNOTATION on any binder (§6)
(def (f (: x Int64)) x)
(let (((: (tuple a b) (Tuple Int64 Int64)) v)) a)
```

**What is NOT a binding pattern** (refutable → rejected in binding position, §4): `(Some x)`, `(Ok v)`,
`0`, `true`, `"lit"`, and a length-constrained list pattern `(list a b)` / `(list x .. rest)`. These may
only appear in a `match` arm where a sibling arm covers the other cases. (The one *irrefutable* list
pattern — a zero-leading rest binder `(list .. rest)` — is out of scope with all list patterns, §8. There
is no `(cons …)` form; the spec's list surface is `(list x .. rest)`.)

---

## 3. Why "irrefutable", and the accept set

`core-semantics.md` §*Patterns Compose* (line 125: "A pattern MUST admit any pattern in each of its binder
positions … matched recursively to any depth") defines pattern composition and linearity. A binding
position (`let`, a parameter) has **no alternative arm** — if the pattern failed there is nowhere to go —
so a binding pattern must be **irrefutable**: it matches *every* value of its type.

| pattern | irrefutable? | binding position |
|---|---|---|
| name `x` | yes (binds anything) | ✅ Increment A (works today) |
| wildcard `_` | yes (matches anything) | ✅ Increment A (works today) |
| `(tuple p₁ … pₙ)`, n≥1, each `pᵢ` irrefutable | yes — a tuple has ONE shape | ✅ Increment A (`let` spiked; param owed) |
| `(tuple)` / `()` (arity 0) | yes in principle | ⚠ EXCLUDED — rides `Ty::Tuple([]) ≠ Ty::Unit` (§8) |
| `(record (k₁ p₁) …)`, each `pᵢ` irrefutable | yes — a record has ONE shape | ⏳ Increment B (decline) |
| single-variant user sum `(V x)` | yes — one variant | ⏳ Increment B (**decline**, not reject) |
| `(Some x)` / `(Ok v)` / any multi-variant ctor | **no** — the other variant exists | ❌ reject `CDZ0210` (§4) |
| a literal `0` / `true` / `"s"` | **no** — matches one value | ❌ reject `CDZ0210` (§4) |
| a length-constrained list pattern `(list x .. r)` | **no** — depends on length | ❌ reject `CDZ0210` (§4) |
| a zero-leading rest binder `(list .. r)` | yes — matches every list | ⏳ out of scope (all list patterns, §8) |

Increment A ships the top three. They compose recursively (a tuple element MAY itself be a name, `_`, or a
tuple pattern, to any depth) and that recursion is **already handled** by `find_binder_in_tuple`
(`resolve.rs:951`, recurses into nested tuple elements) and by `SumPayload` infer/lower (which walk an
arbitrary `Elem`/`Payload` path).

**Two DIFFERENT non-accept outcomes** (keep them apart): a pattern that is irrefutable-in-principle but
not-yet-supported (record, single-variant sum, any list pattern) **declines** (Increment B / later —
reject-don't-miscompile); a pattern that is genuinely **refutable** (a multi-variant ctor, a literal, a
length-constrained list pattern) is an **ill-formed** binding and is **rejected `CDZ0210`** (§4). A
decline says "a later phase handles it," a reject says "no total match exists here."

---

## 4. Refutable-in-binding-position is a rejection — code `CDZ0210`, not `CDZ0201`

A refutable pattern where the language guarantees a total match is an **ill-formed program**, not a
not-yet-supported construct. Reject it with a coded diagnostic naming the offending constructor/literal.

**Which code?** The concept is "a binding pattern IS a single-arm match", so read the corpus for what a
single-arm match that fails to cover its type yields:

- `(match (Some 5) ((Some x) x))` — a Some-only arm, no None → **`CDZ0210`** (non-exhaustive)
  (`02-binding-and-control.sexp` §"a sum match missing a variant is non-exhaustive…").
- `(match true (true 1))` / `(match 5 (5 1))` — a single literal arm → **`CDZ0210`**.

So a **non-total** binding pattern (a multi-variant ctor, a literal, a length-constrained list pattern) is
**`CDZ0210`**. Reserve **`CDZ0201`** for a **shape-INCOMPATIBLE** pattern — one that can *never* match: a
wrong-arity tuple `(tuple a b c)` vs a 2-tuple, or a kind mismatch (a tuple pattern vs a sum value). That
distinction is exactly what the corpus draws (a wrong-arity tuple match arm = CDZ0201; the Some-only arm =
CDZ0210).

```lisp
(let (((Some x) o)) x)     ; CDZ0210 — non-total: None is uncovered, no alternative arm
(def (f 0) …)              ; CDZ0210 — a literal covers one value of its type
(let ((true v)) …)         ; CDZ0210
(let (((tuple a b c) p)) …); CDZ0201 — SHAPE-incompatible if p is a 2-tuple (can never match)
```

**⚠ Unlike the original doc, exhaustiveness does NOT fall out for free here.** The original claimed the
desugared `Hir::Match`'s exhaustiveness pass would emit CDZ0210 automatically. There is no such desugar —
the binder is resolved lazily and the pattern is never handed to `lower_match`'s exhaustiveness/arity
checks. §0.2 confirms this: a refutable `(Some x)` binding gives CDZ0101 (unbound), and a wrong-arity
tuple binding silently compiles. **So the validation is NOT free — §5 must run it explicitly.**

---

## 5. Pass-by-pass changes (against the SHIPPED architecture)

The feature is two parts: **(5.1) binder resolution** (route a tuple-pattern binder to a `SumPayload`) and
**(5.3) validation** (fault a refutable / wrong-arity / non-linear binding). Resolution for the `let` case
is spiked; the param case and all of validation are owed.

### 5.1 `let` binder resolution — `resolve.rs` (SPIKED, §0.1)

`last_binder_named` (`resolve.rs:1003`) is the bindings-list lookup that both let cases (body / later
initializer) call. Changed to return `Option<Resolved>` (was `Option<StructId>`): a bare-name LHS returns
`Resolved::Ref { value }` (the hot path, unchanged); a `is_tuple_pattern` LHS descends with
`find_binder_in_tuple` and returns `Resolved::SumPayload { scrutinee: value_occ, steps, heads }`. Callers
in `binder_in` Case 1 (`resolve.rs:486`) and Case 2 (`resolve.rs:534`) drop their `.map(|v| Ref{value:v})`
wrapper. This is the whole of `let` resolution — proven end-to-end (§0.1). **A binding's initializer does
not see its own binders**: Case 2 passes `stop_before = Some(from)`, so the value's own references resolve
in the outer scope (preserved by the existing window logic).

### 5.2 `def` parameter — a load-time rewrite (LANDED, approach b)

A parameter is a **formal**, not a value occurrence: a bare parameter reference resolves to `Resolved::Param
{ binder }` (`resolve.rs:150`), substituted by the argument at β-reduction. Rooting a `SumPayload` directly
at a param formal (approach a) would need a new β-reduction arm (β-reduce handles `Ref`/`Param`, not a
`SumPayload` over a substituted param) and coordinated edits across `is_param_occurrence` /
`build_scope_binders` / `resolved_of`. **Approach b — a load-time rewrite — is what LANDED**, because it
reduces the parameter case to the already-proven destructuring-`let` case with zero new resolver seams:

```
(def (f (tuple a b)) BODY)   →   (def (f p$0) (let (((tuple a b) p$0)) BODY))
```

`binding_params::lower` (`binding_params.rs`, called from `Db::load` right after `accum::introduce`)
rewrites each `def` whose parameter is a destructuring pattern (a compound list that is not a `(: name T)`
annotation): the parameter slot becomes a fresh whole-value name `p$k`, and the body is wrapped in one
`(let (((<pattern>) p$k)) …)` per pattern parameter (nested, outermost = the first parameter). It reuses
the ORIGINAL pattern occurrence as the `let` LHS and the original body, so a body reference to `a`/`b`
ascends into the synthesized `let` and resolves to the `SumPayload` element read the §5.1 path already
handles; `p$k` binds the whole argument. This mirrors `accum::introduce`'s load-time synthesis exactly
(fresh AST resolves through the ordinary scope walk — no re-resolution corruption), and **arity is
preserved** — `f` still takes one argument per pattern parameter.

A refutable / non-linear / ill-shaped parameter pattern is NOT classified here — the synthesized `let`
carries it to `check_binding_pattern` (§5.3), which faults it with the binding-position code (CDZ0210 /
CDZ0102 / CDZ0201) through the one validation path the `let` case owns. So a `(Some x)` parameter is
CDZ0210, a `(tuple x x)` parameter is CDZ0102 — the same codes the equivalent `let` binder gets. (Broadening
the rewrite to ALL compound params, not just tuples, is what turns the confusing CDZ0101-unbound a
non-rewritten `(Some x)` param would give its binder into the honest coverage code.)

**Verified** (unit tests + corpus): single / nested / multiple / mixed-name tuple parameters, a runtime
argument, a heap-carrying tuple, and a higher-order `(def (app f (tuple a b)) (f a b))` all compile and
run; refutable/non-linear parameters reject.

**NOT yet covered — the `fn`-lambda parameter.** `(fn ((tuple a b)) …)` is an INLINE lambda, not a
top-level `def` in `defs`, so `binding_params::lower` does not reach it — a `fn`-tuple-param currently
DECLINES (CDZ0101 on its binders). Extending the rewrite to descend `fn` forms in the AST (wrap the lambda
body in the same `let`, replace the param) is the follow-up. Also out of scope: a literal-ATOM parameter
`(def (f 0) …)` is still silently accepted (a bare atom is not a destructuring pattern, so the rewrite
skips it — a pre-existing malformed-param gap, unrelated to destructuring).

### 5.3 Validation hook — the NEW work (OWED)

§0.2 shows an ill-formed binding pattern miscompiles. A binding-position pattern must be validated exactly
as a match arm is. The machinery all exists; it must be *reached* from the binding forms:

1. **Linearity (CDZ0102).** `check_pattern_linear` (`lower.rs:1323`) is the recursive, non-deduping walk
   already used for match arms. Call it on each binding/parameter pattern.
2. **Refutability (CDZ0210).** Classify the pattern head: `tuple`/name/`_` → OK; a literal or a
   multi-variant ctor → CDZ0210; a single-variant ctor / record / list → **decline** (Increment B). The
   classifier MUST consult the prelude to tell a single-variant sum (decline) from a multi-variant one
   (reject) and a bare ctor name (`None`) from a binder — mirror `head_ctor` / the sum-variant lookup that
   `find_binder_in_pattern` and lowering already use. Do NOT scan head strings (memory rule: no keys
   outside the prelude).
3. **Shape / arity (CDZ0201).** A wrong-arity tuple pattern against a known tuple type, or a tuple pattern
   against a non-tuple value. `pattern_constraints` (`lower.rs:1268`) already computes tuple-arity
   constraints for match arms and faults a mismatch; the binding path should invoke the same check against
   the bound value's `type_of`.

**Where the hook lives.** The natural site is `collect_node`'s `Resolved::Let` arm (`infer.rs:1691`) for
`let`, and the def/fn parameter fault collector (`param_annotation_faults`, `infer.rs:324`, is the existing
per-parameter fault site) for parameters. For each binding/parameter whose LHS is a pattern (not a bare
name / `_`), run linearity + refutability + shape and push the coded `Reject`. (`collect_node` is where
`Resolved::Match` already pushes its arms-agree faults — the parallel is exact.)

**⚠ Do this as part of Increment A, not after.** The spike shows the resolution half alone is a
*miscompile generator* — it happily resolves `a` inside `(tuple a b c)` against a 2-tuple. Resolution and
validation must land together, or the feature regresses the gate.

### 5.4 `do`-block value-def with a pattern LHS (nice-to-have)

`do_local_binds` / `do_def_binds` (`resolve.rs:604`/`1111`) resolve a do-local `(def …)` by name. Extend
to a pattern LHS routing through the same tuple-pattern binder logic as a `let`. Low value (the corpus
uses `let` for destructuring); ship only if free, else a `(def (tuple a b) v)` in a `do` declines cleanly.

### 5.5 Linearity across the whole pattern — ALREADY DONE (was §5.5's ask)

The original §5.5 asked for a recursive, non-deduping CDZ0102 walk. It **landed** as `check_pattern_linear`
(`lower.rs:1323`) for match arms + duplicate parameters. Binding patterns need only *call* it (§5.3 item
1). The two anticipatory corpus cases (`05-compound-types.sexp` :3386, :3406) verify the recursive/nested
behavior and now pass the current binary — they should **ungate** (§0.2 → §10). The binding-position
CDZ0102 case (`(let (((tuple x x) v)) …)`) is what pins §5.3's wiring and must be authored.

---

## 6. Optional annotations on a binder — `(: <pat> <Type>)`

`type-system.md` §*Annotations Constrain, Never Contradict*: an annotation participates in inference as an
extra constraint (CDZ0203 on contradiction). `(: e T)` is already `Resolved::Annot` (`infer.rs:664`), and
a parameter annotation `(: name T)` already type-checks via `param_annot_ty` (`infer.rs:306`) /
`param_annotation_faults` (`infer.rs:324`). So an annotated binder is a **thin wrapper**: when reading a
binding position, peel a `(: <pat> T)` LHS to `<pat>`, and thread the annotation as a constraint on the
bound value / formal.

- **annotated let:** `(let (((: x Int64) v)) body)` — `x` binds `v` with `v`'s type unified to `Int64`.
- **annotated destructuring:** `(: (tuple a b) (Tuple Int64 Int64))` — the annotation constrains the whole
  tuple *before* the pattern takes it apart.
- **annotated parameter:** `(def (f (: x Int64)) body)` — already works for a bare-name param via
  `param_annot_ty`; a destructuring `(: (tuple a b) T)` param constrains the formal then destructures.

Include the **monomorphic** `(: <pat> <ConcreteType>)` wrapper in A. Leave **type-valued / generic**
annotated params (`(def (id (: x T) (: T Type)) x)`, #150) to a **distinct bidirectional-checking mode** —
`type-system.md` says a type-valued-param position is checked, *not solved by unification*, so #150 must
NOT reuse the `Annot`→unify path. Keep the two mechanisms separate.

---

## 7. Accept / decline / reject boundary (Increment A)

**Accept** (resolves onto the `SumPayload` path):
- `name` / `_` in any binding position (works today; now uniform).
- `(tuple p₁ … pₙ)` nested to any depth, each `pᵢ` a name/`_`/tuple — in `let` (spiked), `def` param, `fn`
  param, (and `do`-`def` if §5.4 shipped).
- an optional monomorphic annotation `(: <pat> <ConcreteType>)` on any of the above.

**Reject** (ill-formed — coded, never a silent accept; §5.3 enforces):
- a **refutable** pattern — multi-variant ctor `(Some x)`/`(Ok v)`, a literal, a length-constrained list
  pattern → **CDZ0210**.
- a **shape-incompatible** pattern — a wrong-arity tuple `(tuple a b c)` vs a 2-tuple, or a kind mismatch →
  **CDZ0201**.
- a **non-linear** pattern (a binder repeated, flat or nested) → **CDZ0102** (`check_pattern_linear`).
- an annotation that contradicts the value/param type → **CDZ0203**.

**Decline** (irrefutable-in-principle but not-yet-supported — reject-don't-miscompile; NEVER a
CDZ0201/0210 reject):
- a **record** binding pattern `(record (k p) …)` → Increment B (§8).
- a **single-variant-sum** binding pattern `(W x)` → Increment B (distinguished from a refutable
  multi-variant ctor by the sum's variant count).
- any **list** binding pattern (even the irrefutable zero-leading rest binder) → out of scope with all
  list patterns (`(needs list-patterns)`).

---

## 8. Increment B — records + single-variant sums in binding position (DEFERRED)

Records and one-variant sums are irrefutable *in principle*, but `find_binder_in_*` and the `SumPayload`
path do **not** know them: `find_binder_in_tuple` handles only `tuple` heads; `find_binder_in_pattern`
handles multi-payload variant ctors. So they are genuinely new pattern kinds, not just new binding sugar.

**Record patterns.** A record is a name-sorted positional `arr` (like a tuple), so the runtime binding is
the same element read. B adds a record-pattern descent (bind each field at its name-sorted slot, mirroring
the record literal's sort) and pins the record's exact field set (a projection on an unsolved var
declines — "one field cannot pin the whole field set" — so it must be a *pattern* that pins the full field
set at once, exactly as the tuple pattern pins arity). A bounded, self-contained extension.

**Single-variant user sums.** `(type Wrap (W Int64))` makes `(W x)` irrefutable. In A the refutability
classifier **declines** it (variant count == 1); B makes it *accept* by descending the sole variant's
payload with no discriminant guard. Rare in the corpus; lowest priority.

**List patterns** are a separate feature (`(needs list-patterns)`), out of scope. The classifier
**declines** all list patterns (rather than rejecting) so the irrefutable zero-leading rest binder is not
mis-rejected when list patterns land.

**Empty tuple `(tuple)` / `()`.** Excluded from A. rcdzc resolves `()` to `Ty::Unit` but `(tuple)` to
`Ty::Tuple([])`, treated as distinct by `unify`, so a `(tuple)` binder would mis-reject against a
unit-typed value. A pre-existing `Ty` quirk this feature would merely ride; a wildcard `_` is the correct
way to bind-and-ignore a unit until `Unit`/`Tuple([])` identity is reconciled.

### 8.1 ACTIVATED (2026-08-30, operator-greenlit): single-variant-sum binders — the const-forwarding fix

**Why now.** v-code-cleanliness's single-variant-sum `match`→`let` sweep hit a spurious `CDZ0201` on
`implementation/iterators/src/adapter.cdz`: rewriting `fold(it, acc, g) = match it with | Iter.Mk(s, step)
=> drive(step, s, acc, g)` to `let Iter.Mk(s, step) = it in drive(step, s, acc, g)` (where `drive` has a
`const step` param) declines "an argument to a `const` parameter must be compile-time-known — it depends
on runtime data" (call_lower.rs:397-region / the `type_specialize` const-arg gate). The operator greenlit
the real fix (not a workaround, not status-quo). It is **exactly this deferred increment**, not a new arc.

**Root cause (confirmed, resolve-side).** A `let` variant-destructure binder is NOT descended by
`find_binder_in_tuple` (tuple heads only), so `step` falls to the generic bare-name path →
`Resolved::Ref { value: <it-occ> }`. At the inner `drive(step, …)` call, `arg_captures_runtime_binding`
(`call_lower.rs:1478`) flags a `Resolved::Ref`/`Param` binder whose value is `is_within`-OUTSIDE the arg
and is a param/local → so `step` is judged a runtime capture → `type_specialize` (`call_lower.rs:1709`)
declines the `const` arg. A **`match`-arm** variant binder resolves to a `SumPayload { scrutinee, steps,
heads }` (Case 6 `match_arm_variant_binds`), which is NEITHER `Ref` nor `Param` → NOT flagged → the const
arg is accepted and `drive` specializes. **The entire delta is `resolved_of(step)`** — a `let` binder
vs a match-arm binder. (The sibling TUPLE let-destructure already works because §5.1 resolves a tuple
binder to a `SumPayload`; adapter's `let (x, s2) = p` at drive:93 compiles. This increment gives the
single-variant SUM binder the identical treatment.) A lowering-phase `let`→`match` desugar does NOT fix
it (verified: the spec-demand keys on the resolve-phase binder classification, fixed pre-lower).

**The fix.** Extend the §5.1 binder resolution to a single-variant-sum LHS: when `last_binder_named`'s
LHS is a variant-ctor pattern `((. Sum V) a b…)` (or name-head `(V a b…)`) whose owning sum has exactly
ONE variant (`variant_owner_decl` + `type_decl_by_occ().variants.len() == 1`), descend the sole variant's
payload (a `find_binder_in_variant`, the binding-position twin of `match_arm_variant_binds`) and return
`Resolved::SumPayload { scrutinee: value_occ, steps, heads }` — NO discriminant guard (single variant is
irrefutable). A multi-variant ctor stays REFUTABLE → the §5.3 classifier faults it CDZ0210 (a `let`
binding cannot assume one of several variants). This is resolve.rs, mirroring the tuple path and the
match-arm path — zero new IR, the design's through-line ("a binding position holding an irrefutable
pattern IS a single-arm destructuring match rcdzc already resolves").

**Not the rejected option (b).** This is NOT a syntactic `let`→`match` rewrite (the workaround the operator
distinguished from the real fix); it is the principled resolve-side `SumPayload` binder-resolution the
design already ships for tuples (§5.1), extended to the sole variant. The const-forwarding "backward"
verdict is carried for free once `step` resolves as a compile-time-traceable projection of its scrutinee.

**Scope / ownership split** (operator: v-compiler-primitives leads; joint with v-inference):
- **v-inference (resolve-side):** `find_binder_in_variant` + the `last_binder_named` single-variant case
  (resolve.rs); the §5.3 classifier arm intercepting a ctor-head binding position (multi-variant → CDZ0210,
  single-variant → accept, replacing the current CDZ0101-unbound / decline); co-verify `type_specialize`
  still DECLINES a genuinely runtime-data const arg (no over-specialization — a `fold` over a runtime
  iterator whose `step` is not compile-time-known must still decline, exactly as the match form does).
- **v-compiler-primitives (lead):** this design; the const-arg-gate interaction (confirm the `SumPayload`
  binder is not flagged by `arg_captures_runtime_binding` and the gate accepts); the corpus + rcdzc-test
  witnesses (the adapter `fold` unblock + a minimal single-variant-sum-destructure-let-forwards-to-const
  case + the runtime-`it` still-declines negative case); driving the increment green-per-commit; fleet coord.

**Witnesses / done-criteria.** (1) adapter.cdz `fold` as `let Iter.Mk(s, step) = it in …` compiles (unblocks
the v-code-cleanliness sweep). (2) a rust/corpus witness: a single-variant-sum destructure-let forwarding a
field to a `const` param compiles and runs. (3) a NEGATIVE witness: the same over a genuinely-runtime
scrutinee still declines (parity with the match form — v-inference's over-specialization guard). (4) a
multi-variant destructure-let still rejects CDZ0210 (refutability preserved). Store-dependent gating waits
for the cachix build-hold RESUME; resolve/lower work + native `cargo test -p rcdzc` proceed under the hold.

- **Arity is preserved for a destructuring parameter.** `(def (fst (tuple a b)) a)` is a **one-argument**
  function whose argument is a pair. The parameter occupies one slot; the destructure is in how references
  read it. Do NOT expand a tuple param into N wasm params (breaks the export/call signature).
- **A binding's initializer does not see its own binders.** Case 2 passes `stop_before = Some(from)`, so
  the value resolves in the outer scope. `(let (((tuple a b) a)) …)` must resolve the RHS `a` outside the
  pattern (an unbound-name / outer-binding, not the pattern's `a`). (Spiked: `let*` scoping — a later
  binding seeing an earlier pattern's binder — works, §0.1.)
- **Resolution WITHOUT validation is a miscompile generator (§0.2).** The lazy binder lookup resolves `a`
  inside `(tuple a b c)` against a 2-tuple with no complaint. Land §5.1/5.2 (resolution) and §5.3
  (validation) TOGETHER, or the gate regresses.
- **`SumPayload` over a `Param` scrutinee is the param case's crux (§5.2).** Verify it infers when the
  param type is solved from the call site — the tuple pattern must pin the tuple arity the way a match
  tuple pattern does.
- **Refutable → CDZ0210 (coverage), not CDZ0201 (§4).** A non-total binding pattern is the
  non-exhaustive-single-arm-match the concept produces. Reserve CDZ0201 for a shape-incompatible pattern.
  Do NOT emit a generic "unsupported" decline for a refutable pattern.
- **The `(Some x)` binding today gives CDZ0101, not a clean decline (§0.2).** Because `find_binder_in_tuple`
  only descends `tuple` heads, a `Some` head binds nothing and `x` reads as unbound. The §5.3 classifier
  must intercept a ctor-head binding position BEFORE the binder lookup concludes "unbound", and emit
  CDZ0210 (multi-variant) / decline (single-variant).
- **Wildcard-as-discard is a real drop.** `(let ((_ e)) body)` binds `e` to a throwaway and drops it —
  works today; keep it so `e`'s own faults (an unbound name inside it, CDZ0101) still surface.
- **Linearity is recursive AND non-deduping** — already realized in `check_pattern_linear` (`lower.rs:1323`);
  binding patterns must *call* it, not reinvent it.

---

## 10. Spec-first prerequisite, corpus payoff & what it unblocks

**⚠ Step 0 — SPEC LEADS (still owed).** `core-semantics.md` §*Patterns Compose* (line 125) sanctions
patterns in binder positions *of a pattern*, and §137 sanctions list-element binder positions — but no
normative sentence yet says a `let` binder / parameter / `fn` param position MAY hold an irrefutable
pattern, nor that a refutable one is CDZ0210. The constitution makes the executable corpus the source of
truth. So the FIRST unit of work is:
1. add the normative capability sentence(s) to `core-semantics.md` (a new §"A Binding Position Accepts An
   Irrefutable Pattern", defining that a `let` binder / parameter / `fn` param MAY hold an irrefutable
   pattern binding a value of the matched type, that a refutable one is `CDZ0210` and a shape-incompatible
   one `CDZ0201`); and
2. author the corpus witnesses below.
The spike (§0.1) proves the mechanism ahead of the spec, but the *landed* feature must implement a
specified capability. (Ordering note: the spec sentence + corpus cases may land in the same increment as
the code, but they must exist — a green run of a case the spec doesn't sanction is not conformance.)

**New `02-binding-and-control.sexp` cases to author** (the mechanism makes the accepts pass):
- accept: a destructuring `let` `(let (((tuple a b) v)) …)`; a destructuring parameter `(def (fst (tuple a
  b)) a)`; a nested tuple binder `(let (((tuple a (tuple b c)) v)) …)`; a wildcard discard `(let ((_ e)) …)`;
  a multi-binding destructuring `let` where a later binding uses an earlier pattern's binder (the decoder
  idiom — pins `let*` scoping, spiked green);
- an **annotated** binder — accept `(def (f (: x Int64)) x)`, and **reject** a *contradicting* annotation
  `(let (((: (tuple a b) (Tuple Int64 Bool)) v)) …)` → `CDZ0203`;
- **reject** a refutable binding pattern — `(let (((Some x) o)) x)` → **`CDZ0210`**; a literal binder
  `(def (f 0) …)` → `CDZ0210`;
- **reject** a shape-incompatible binder — a wrong-arity tuple against a known 2-tuple → `CDZ0201`;
- a binding-position **CDZ0102** case — `(let (((tuple x x) v)) …)` → `CDZ0102` (pins §5.3's linearity
  wiring; the two existing linearity cases are `match` arms).

**Two existing gated cases ungate NOW (independent of the rest — a quick landing).** The flat + nested
linearity cases (`05-compound-types.sexp` :3386, :3406) still carry `(needs linear-patterns)` but the
current binary already produces `CDZ0102` for both (verified). Remove the gate + the stale "the seed does
not yet enforce" doc comments so they run as passing rejections.

- **The self-host decoder.** `implementation/compiler/cdzc/15-decode.cdz` threads `(tuple <Ast> <offset>)`
  through every `decode-*`, each paying a bind-then-`match` tax. Binding patterns cut a match per decode
  step. On the self-host critical path — which is why #153 sits where it does.

---

## 11. Scope decisions (recommendations)

0. **Quick win first — ungate the two linearity cases (§10).** Independent of everything else; verifies
   currency and closes two anticipatory-pinned cases.
1. **Spec sentence + corpus witnesses (§10 step 0)** for the binding-pattern capability.
2. **Increment A, landed as sub-increments, gate-green each:** (a) `let` resolution (spiked) + validation
   hook → land; (b) `def`/`fn` param resolution + validation → land; (c) monomorphic annotations → land.
   **Resolution + validation ship together** (§5.3/§9) — resolution alone miscompiles.
3. **Defer B** (records, single-variant sums); it *declines* (never rejects) in A.
4. **`do`-`def` pattern LHS (§5.4)** only if free.
5. **Diagnostic codes:** refutable → CDZ0210; shape-incompatible → CDZ0201; non-linear → CDZ0102 (existing);
   contradicting annotation → CDZ0203.

## 12. Ladder placement & related

Task **#153** (params/let as optionally-annotated patterns). A `resolve`-side lift that reuses the
`SumPayload` tuple-binder path (`resolve.rs:557`, `find_binder_in_tuple` `resolve.rs:951`) shared with
match arms, plus a validation hook mirroring `check_pattern_linear` (`lower.rs:1323`) and
`pattern_constraints` (`lower.rs:1268`). Its linearity code (`CDZ0102`) already landed; ungating its two
corpus cases is a free adjacent win.

Related: `spec/capabilities/core-semantics.md` §*Patterns Compose* (:125) / the tuple-pattern cases in
`spec/semantics/02-binding-and-control.sexp`; `05-compound-types.sexp` (the two gated linearity cases,
:3386 / :3406); the ask-81 closures handoff (the reuse-an-existing-tier pattern); the learning
`2026-07-08-pattern-linearity-must-be-pinned-across-sub-patterns-not-only-flat.md` (the recursive-check
tripwire, now satisfied); `DESIGN-effects-rcdzc.md` (house style).
