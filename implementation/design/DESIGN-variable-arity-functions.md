# DESIGN — variable-arity (varargs) functions: `(.. v)` rest params → list or tuple

> **Status:** DESIGN — DRAFT FOR OPERATOR REVIEW (do NOT auto-merge; the operator merges when
> ready). Subsystem: `rcdzc` (seed compiler), with coordinated touches to `cadenza-syntax` (ML
> surface), the `Type` reflection module (shared with `v-metaprogramming`), and `spec/`.
> **Owner:** `v-varargs` (this vertical), from an operator spark relayed by the concierge.
>
> **Operator intent (verbatim, seq 219):** *"Can we get a vertical to spec out, design and implement
> variable arity functions? I'm thinking we use the `(.. v)` syntax for the rest args. And then you
> would either get a list of arguments of the same type or a tuple depending on how you annotated it.
> For the tuple the function would be able to do compile time type checking and branch on what types
> were passed or assert a value is of a certain type. We'd need to be able to get the size of a tuple
> if we can't already. I think we'd also need a way to try getting a value with a given type. So you
> would have something like `ty: Type -> a -> Option b`. And that would return Some V if the provided
> value is the provided type. I'm not sure what to best call this so the agent can propose that in the
> doc. But we should probably put it on the Type module."*

---

## 0. TL;DR — the shape of the feature

A function may declare its **last** parameter as a **rest parameter** `(.. v)`, collecting all
trailing arguments into a single bound value. The collection's shape is chosen **by the rest
parameter's type annotation**:

| rest param | binds `v` to | trailing args | function is | primary use |
|---|---|---|---|---|
| `(.. (: v (List T)))` | `List<T>` | all same type `T` | ONE runtime fn | fold/iterate a homogeneous run |
| `(.. (: v (Tuple)))` / unannotated | `Tuple(T_k … T_n)` (per call-site) | heterogeneous | monomorphized per call | compile-time type-branching |

```cadenza
;; homogeneous → List  (single runtime function)
def sum(.. (: xs (List Int))) = List.fold(xs, 0, add)
sum(1, 2, 3)            ;; xs = [1, 2, 3] : List Int  → 6

;; heterogeneous → Tuple  (monomorphized at each call-site)
def describe(.. xs) =
  let n = Tuple.size(xs) in
  ...branch on the types actually passed...
describe(1, "hi", true) ;; xs = (1, "hi", true) : Tuple(Int, Str, Bool)
```

Three supporting primitives (§4/§5), two of them brand new:
- **`Tuple.size : Tuple -> Int`** — compile-time tuple arity (NEW; no size prim exists today).
- **`Type.try-as : Type -> a -> Option b`** — compile-time "view this value at this type" (NEW; the
  operator's `ty -> a -> Option b`; name proposed §5, on the `Type` module).
- `Type.of` / `Type.eq` — already exist; the branching uses them plus the two new prims.

This design leans entirely on machinery that already exists: the `(.. operand)` marker shape
(v-ast-compound), tuple types/ctor/projection, the `PathStep::TupleRestFrom` gather precedent, and
the compile-time one-tier reducer that already folds `Type.of`/`Type.eq`/monomorphization.

---

## 1. Current state (verified this session — file anchors on the worktree base)

All paths under `implementation/seed/crates/`.

### 1.1 Functions & parameters are generic s-expressions — there is no "param" node
The AST (`cadenza-ast/src/ast.rs`) is a generic s-expr tree: `Struct::Atom(LeafId)` / `Struct::List(Vec)`
(ast.rs:215), identifiers are `Leaf::Name` (ast.rs:80). A function is just a list:
- `def add(a, b) = a + b` → `(def (add a b) (+ a b))` (`spec/syntax/ml/111-function-def-with-params/`)
- `fn(x, y) => x + y` → `(fn (x y) (+ x y))` (`spec/syntax/ml/188-lambda-multi-param/`)

rcdzc reads params structurally:
- `resolve_lambda` (`resolve.rs:6772`) — `(fn (<param>…) body)`; the param list is collected as
  `Rc<[StructId]>` (resolve.rs:6803).
- `def_as_resolved` (`resolve.rs:616`) — a def with params → `Resolved::Lambda { params, body }`.
- **`is_param_occurrence` (`resolve.rs:6696`)** — the definitive "what is a parameter" predicate: a
  param is a **bare `Name`** OR an annotated **`(: name T)`** binder (resolve.rs:6703-6717). *This is
  the exact predicate a `(.. v)` rest param extends.*
- `Def.params: Vec<StructId>` (`db.rs:243`).

Application is **compile-time β-reduction**, curried: `apply_lambda(_uncached)` (`eval.rs:1057`/`1117`)
zips params to args **positionally** (zip loops at eval.rs:1157, 1197): exact arity reduces the body,
extra args curry, fewer = partial application. **This positional zip is the primary seam varargs
changes.** Arity errors: `infer/application.rs:215-253` (`args.len() == N` arms,
`BUILTIN_WRONG_ARITY_DECLINE`).

### 1.2 `(.. v)` marker — reused verbatim, ALREADY the fleet-wide spread/rest shape
`..` is **not** a special compiler token — it is an ordinary `Leaf::Name("..")`, and `(.. v)` is a
`Struct::List([Name(".."), v])`. Recognition idiom: `db.ast.as_name(first) == Some("..")` (value side)
or `as_form(node, "..")` (`resolve.rs:4058`). This is the operator-mandated marker
(`DESIGN-collection-spread-construction.md §0a`: *"I want the `(.. v)` operator to be everywhere —
patterns and constructors"*). Today it is handled in **collection patterns** (list/map/tuple/set rest:
`resolve.rs:3519-3553`, 3034, 4058) and, in-flight from **v-ast-compound**, in **collection
construction** (value-position spread). **Function-parameter position is NEW and is THIS vertical's
territory** — see §7 for the coordination boundary.

### 1.3 Tuples — types, ctor, projection, and the gather precedent
- `Ty::Tuple(Rc<[Ty]>)` (`ty.rs:680`) — fixed-size positional product, structural equality.
- `Core::Tuple { elems: Rc<[StructId]> }` (`core.rs:399`); `Core::Proj { operand, index }`
  (`core.rs:407`) — projection; out-of-arity index is a **compile-time** reject, not a runtime trap.
- **`PathStep::TupleRestFrom(usize)` (`core.rs:176`)** — binds "the trailing sub-tuple from index k" as
  a *new* `Tuple(T_k … T_{n-1})` for a tuple-rest **pattern**. **This is the closest existing precedent
  to varargs gathering** — the tuple-rest varargs binding is the value-position dual of it.
- Tuple ops (`resolved.rs` `Prim`): `TupleNew` ("tuple-new"), `TupleCat`, `TupleSplitAt`, `TuplePop`.
  Prelude `tuple_module` (`prelude.rs:522-542`): `concat`/`split-at`/`remove`.
- **NO `tuple-size` / `tuple-len` prim exists** — arity is statically known from `Ty::Tuple(elems).len()`
  but nothing surfaces it to the language. §4 adds one.
- Tuple typing/checks: build in `infer/construct.rs:215`, projection type-check `infer/node.rs:592-641`
  (operand must be `Ty::Tuple`, index within arity → else CDZ0201), op result types
  `infer/node.rs:1773-1802`.

### 1.4 The `Type` module — compile-time reflection only, NO runtime type-test today
`type_module` (`prelude.rs:1926-1957`, registered `prelude.rs:215`) is a **namespace-only record**
(`Type` is not itself a type). Fields, each a `ctor_record` whose `(meta apply)` is a `Prim`:
- `of` → `type-of` (`Type.of e` → the type-VALUE of `e`'s inferred type; folds at `eval.rs:2893`),
- `eq` → `type-eq` (compile-time structural type equality → `Bool`),
- `ast` / `ast-generic` → `type-ast` (decl `Ast`; the `v-metaprogramming` reflection work).

Recognition is **structural** (`meta_apply_of(...) == Some(Prim::TypeOf)`, `eval.rs:3183-3213`), never
by the literal name `"Type"`, so a rebound `Type` name does not capture it. Type-values are ordinary
prelude records with a `(meta t)` channel (`prelude.rs:48-52`); a type-value's own type is `Ty::Type`.
**There is no runtime "is v of type T", no `try-as`, no downcast prim anywhere** — all reflection is
compile-time-folded. §5 adds the operator's `try-as`.

### 1.5 The one-tier compile-time reducer
Macro expansion, generic reduction, monomorphization, and constant folding are the **same** pure
mechanism (`spec/capabilities/metaprogramming.md:70-82`; reducer in `eval.rs`). `Type.of`/`Type.eq`
already fold on this tier. **Every new primitive here (`Tuple.size`, `Type.try-as`) and the
tuple-rest monomorphization are pure reductions on that same tier** — no new runtime capability, no
new effect.

---

## 2. Surface syntax

### 2.1 The rest parameter `(.. v)`
A rest parameter is a `(.. operand)` node in a `fn`/`def` parameter list, where `operand` is itself a
parameter binder — a **bare name** `(.. xs)` or an **annotated binder** `(.. (: xs T))`. It reuses the
exact `(.. operand)` arena shape §1.2, so the marker is consistent with value-position spread and
collection-pattern rest. ML surface and arena:

| | ML surface | s-expr / arena |
|---|---|---|
| bare rest | `def sum(..xs) = …` | `(def (sum (.. xs)) …)` |
| annotated list-rest | `def sum(..xs: List Int) = …` | `(def (sum (.. (: xs (List Int)))) …)` |
| annotated tuple-rest | `def f(..xs: Tuple) = …` | `(def (f (.. (: xs Tuple))) …)` |
| lambda rest | `fn(..args) => …` | `(fn ((.. args)) …)` |
| fixed + rest | `def g(a, b, ..rest) = …` | `(def (g a b (.. rest)) …)` |

**ML grammar:** the ML front-end already lexes `..` (`Kind::DotDot`, `lexer.rs:118-134`) and parses
`..x` in patterns/construction; §7 extends the **parameter-list** parser (`cadenza-syntax/src/parser.rs`)
to accept a trailing `..name` / `..name: T` as the last formal. (Both the s-expr front-end and the ML
front-end must produce the identical `(.. …)` arena node so rcdzc sees one shape.)

### 2.2 Placement rules (checked at resolve, diagnosed — §6)
1. **At most one** rest parameter per parameter list.
2. It must be **last** (no fixed params may follow) — mirrors collection-pattern rest.
3. It binds **zero or more** trailing arguments; the fixed params before it are still required
   positionally (partial application below the fixed count still curries as today).
4. The rest operand must be a valid binder (bare name or `(: name T)`) — anything else is a diagnostic.

Each violation is a dedicated compile-time diagnostic (§6), not a silent fallthrough to the bare-`..`
CDZ0201 reject.

---

## 3. Semantics — list-rest vs tuple-rest, chosen by annotation

The operator's rule: *"you would either get a list of arguments of the same type or a tuple depending
on how you annotated it."* Concretely:

### 3.1 List-rest (homogeneous) — `(.. (: v (List T)))`
- All trailing arguments must unify to `T`; `v : List<T>`.
- **ONE runtime function.** The trailing args are gathered into a runtime `List<T>`
  (`List.new(a_k … a_m)`), bound to `v`. The body is an ordinary function over a list; nothing is
  monomorphized. Zero trailing args → the empty list `[] : List<T>` (`T` must be inferable — from the
  annotation, or a diagnostic if fully unconstrained, §6).
- Typing: the function's type is `… -> List<T> -> R` with the rest folded into a single list parameter;
  a call `f(x, a, b, c)` unifies `a,b,c : T`.

### 3.2 Tuple-rest (heterogeneous) — `(.. (: v Tuple))` or unannotated (default, §3.3)
- The trailing arguments keep their individual types; at a call-site with trailing args `a_k … a_m`,
  `v : Tuple(T_k … T_m)` where each `T_i = type_of(a_i)`.
- **Monomorphized per call-site**, exactly like a generic. Because a tuple type is fixed-arity, a
  tuple-rest function has a *different* parameter type at each call; the body is reduced with `v` bound
  to the concrete `Core::Tuple([a_k … a_m])` (the value-position dual of `PathStep::TupleRestFrom`).
  This is the same compile-time β-reduction the reducer already does — each call specializes the body
  against the concrete tuple, so `Tuple.size(v)`, projections `v.i`, and `Type.try-as`/`Type.of` on
  elements all **fold to constants**. This is what makes "compile-time type checking and branch on
  what types were passed" work: after monomorphization the tuple is concrete, so every type test is
  decided at compile time (§5.3).
- Zero trailing args → the empty tuple `Tuple()` (arity 0); `Tuple.size` folds to `0`.

### 3.3 Default when unannotated — OPEN DECISION (proposed: tuple-rest)
`(.. xs)` with no annotation: **proposed default = tuple-rest (heterogeneous)**, because it is the more
general form (a homogeneous run is still expressible as a tuple, and list-rest is the *opt-in
homogeneity constraint* you request with `: List T`). Rationale: the interesting new capability the
operator described (compile-time type-branching, `Tuple.size`, `Type.try-as`) all live on the tuple
path; making that the default keeps the bare `(.. xs)` maximally expressive, and a caller who wants the
simpler single-runtime-function list just annotates `: List T`. **Alternative** considered: require an
annotation (no default) — rejected as less ergonomic. **This is the one surface-semantics choice worth
an operator ruling; a `Type.ast`-style default is pinned here and I will confirm via an `ask`.**

### 3.4 Binding mechanism (`apply_lambda`)
In `apply_lambda(_uncached)` (`eval.rs:1057`/`1117`), when the last param is a `(.. operand)` node:
1. Bind the fixed params `p_0 … p_{k-1}` positionally to `a_0 … a_{k-1}` (unchanged path).
2. Gather the remaining args `a_k … a_m`:
   - list-rest → synthesize `List.new(a_k … a_m)` (reuse the list-construction lowering) and bind `v`.
   - tuple-rest → synthesize `Core::Tuple([a_k … a_m])` (the `TupleRestFrom` value-dual) and bind `v`.
3. Reduce the body with the augmented env.
Arity: require `m ≥ k-1` (at least the fixed params). Below `k` fixed args still curries (partial
application) exactly as today — the rest param only engages once the fixed params are satisfied.
`infer/application.rs` arity checks (215-253) grow a "≥ fixed" rule for a varargs callee instead of the
exact `== N` equality.

---

## 4. `Tuple.size : Tuple -> Int` (NEW primitive)

The operator: *"We'd need to be able to get the size of a tuple if we can't already."* We can't — §1.3.

- **Surface:** a field `size` on the existing `tuple_module` (`prelude.rs:522`), `Tuple.size : Tuple -> Int`.
- **Prim:** new `Prim::TupleSize` (`resolved.rs:71` enum + `from_name` arm near the other tuple prims
  ~960), intrinsic name `tuple-size`.
- **Reduction:** a pure compile-time fold in `eval.rs` (the `match prim` at ~2769/3388/3473): the
  operand's type is `Ty::Tuple(elems)`; fold to the constant `Int` `elems.len()`. (A tuple's arity is
  always statically known, so this ALWAYS folds — it never survives to runtime; no `Core` node needed,
  so no backend arm — same story as a fully-const `Type.eq`.)
- **Typing (`infer`):** `Tuple -> Int`; operand must infer to a `Ty::Tuple` (else the existing
  not-a-tuple diagnostic, mirroring projection's `infer/node.rs:592-641`).
- **Gate:** corpus cases pinning `Tuple.size((1, "a", true))` ⇒ `3`, empty `Tuple.size(())` ⇒ `0`, and
  a tuple-rest varargs call whose body reads `Tuple.size(xs)`.

*Standalone & independently landable — the first increment (§8.1).*

---

## 5. `Type.try-as : Type -> a -> Option b` (NEW primitive — name proposed)

The operator: *"a way to try getting a value with a given type. So you would have something like
`ty: Type -> a -> Option b`. And that would return `Some v` iff the provided value is the provided
type … put it on the Type module. I'm not sure what to best call this so the agent can propose."*

### 5.1 Proposed name — `Type.try-as`
Ranked proposal (operator picks in review):
1. **`Type.try-as`** *(recommended)* — reads as "try to view this value **as** this type", the `try-`
   prefix signals the `Option` result (a fallible view), kebab-case matches the language. Usage reads
   naturally: `Type.try-as(Int, x)` → `Option Int`.
2. `Type.as` — shorter, but drops the fallibility signal (an `as` that returns `Option` is slightly
   surprising next to a total `Type.of`).
3. `Type.cast` — familiar, but "cast" connotes *conversion*; this performs no conversion, only a typed
   view, so `cast` mis-signals.

I will pin `Type.try-as` in the doc and send the concierge an `ask` with these three so the operator can
overrule in one line without blocking the build.

### 5.2 Signature & semantics
```
Type.try-as : (t : Type) -> a -> Option b     -- b is the type DENOTED by the value t
```
- `t` is a **first-class type value** (as produced by `Type.of`, a type literal, or a type name). The
  result element type `b` is the type `t` denotes — so `Type.try-as(Int, x) : Option Int`.
- **Semantics (compile-time fold):** let `Tv = type_of(x)`. If `Tv` **matches** `t` (structural
  equality via the same machinery as `Type.eq`; §5.4 covers whether this is `==` or `<:`), fold to
  `Some (x : b)`; otherwise fold to `None : Option b`.
- **Pure, one-tier.** Because `t` is compile-time-known and `x`'s type is statically inferred, this
  ALWAYS folds to a definite `Some`/`None` at compile time — it introduces **no runtime type tag and no
  runtime branch** (which is why it composes perfectly with tuple-rest monomorphization: after a
  tuple-rest call is specialized, each element's type is concrete, so `Type.try-as` on it is decided).
- Requires `t` to be a **concrete** type value; on a non-concrete `t` (unresolved type variable) it
  **declines with a diagnostic** (mirrors `Type.ast`'s non-concrete decline, `DESIGN-type-to-ast-reflection.md §3.4`).

### 5.3 This is the type-branching primitive
"branch on what types were passed" and "assert a value is of a certain type" both fall out:
```cadenza
def render(.. xs) =                 ;; tuple-rest, monomorphized per call
  Tuple.map(xs, fn(x) =>            ;; (illustrative; each element folds concretely)
    match Type.try-as(Int, x) with
    | Some(n) => Int.to-str(n)
    | None => match Type.try-as(Str, x) with
              | Some(s) => s
              | None => "?")
```
- **branch on types passed** = `match` on `Type.try-as` (or on `Type.eq(Type.of(x), T)`), folded per
  monomorphized call-site.
- **assert a value is of a certain type** = a thin derived helper `Type.assert-as(T, x)` (or a library
  `fn` using `try-as` + a compile-time error) that turns `None` into a compile-time diagnostic instead
  of an `Option`. Proposed as a **follow-up increment** built on `try-as` (§8.5), not a separate prim —
  keeps the primitive surface minimal.

### 5.4 OPEN DECISION — exact match: structural equality (`==`) vs subtype (`<:`)
Does `Type.try-as(t, x)` succeed only when `type_of(x)` **equals** `t`, or also when `type_of(x)` is a
**subtype** of `t` (e.g. a nominal newtype's inner, an open-sum widening)? **Proposed default: exact
structural equality** (reuse `Type.eq`), because it is the least surprising and matches the operator's
phrasing "iff the provided value **is** the provided type"; a subtype-aware variant can be added later
if wanted. Flagged for the operator + coordinated with `v-inference` (owns subtyping) and
`v-metaprogramming` (owns `Type.eq`).

### 5.5 Implementation seams (mirror `Type.of`/`Type.eq`/`Type.ast`)
| Concern | Anchor | Change |
|---|---|---|
| Prim | `resolved.rs:71` enum + `from_name` ~1028 (Type prims) | add `Prim::TryAsType`, intrinsic `type-try-as` |
| Prelude field | `type_module` (`prelude.rs:1926`) | register `try-as` via `ctor_record` with a `(meta t)` scheme `Type -> a -> Option b` |
| Reduction | `eval.rs:2893` (`Prim::TypeOf`), 3183-3213 (structural recognition) | add the `Prim::TryAsType` arm: decode `t` to `Ty`, compare to `type_of(x)`, fold `Some`/`None` |
| Typing | inference of `Type.*` fields | result `Option <denoted-by-t>`; non-concrete `t` → decline |
| Backend | none | never survives to runtime (always folds) → no `Core` node, no backend arm |

---

## 6. Diagnostics (each pins a corpus reject case)
Varargs must emit **actionable** diagnostics, not fall through to the generic bare-`..` CDZ0201:
1. **Rest param not last** — `def f(..xs, y) = …`: "a rest parameter `(.. v)` must be the last
   parameter; move `..xs` to the end or remove the trailing parameter(s)."
2. **Multiple rest params** — `def f(..xs, ..ys) = …`: "at most one rest parameter is allowed."
3. **Rest operand not a binder** — `(.. 3)` / `(.. (a b))`: "a rest parameter binds a name (optionally
   annotated `(.. (: v T))`); found `<node>`."
4. **List-rest element type mismatch** — `sum(1, "x")` for `(.. (: xs (List Int)))`: reuse the standard
   unify-fail diagnostic naming the offending arg and expected `T`.
5. **Under-arity** — fewer than the fixed-param count still curries (not an error); but an over-strict
   caller (e.g. named-arg contexts) gets the existing arity diagnostic adapted to "≥ N".
6. **`Type.try-as` on a non-concrete type** — §5.2 decline.
7. **`Tuple.size` on a non-tuple** — §4.
Each gets a dedicated code + a `spec/semantics` reject case (`(error CDZxxxx)`), following the
diagnostics vertical's bar (a concrete applyable fix, not just "what went wrong").

---

## 7. Peer coordination & the territory boundary (CRITICAL)

- **`v-ast-compound` — the `(.. v)` marker.** They own **value-position** spread (collection
  *construction* `#list(a (.. c))`). I own **parameter-position** `(.. v)` (fn/def formals). The two
  share ONLY the arena node shape `(.. operand)` — which is exactly the point (one consistent marker).
  They are disjoint syntactic contexts (a ctor child vs a param-list element), touched in different
  resolvers (`resolve_list`/`resolve_record` vs `resolve_lambda`/`is_param_occurrence`), so there is no
  file race on the recognition site. I will `note` them to confirm the shared recognition helper
  (`as_form(node, "..")`) stays a single shared idiom and neither of us forks it.
- **`v-metaprogramming` — the `Type` module.** They own `Type.ast`/`Type.ast-generic` and the
  structural-recognition path (`eval.rs:3183-3213`). My `Type.try-as` adds one field to `type_module` +
  one `Prim` arm in the SAME neighborhood. I will `note` them so the two prim additions don't collide
  and the module-field table stays coherent (co-author the `type_module` edit if timing overlaps).
- **`v-inference` — varargs typing + tuple branching + subtyping.** The list-rest unification, the
  tuple-rest monomorphized parameter type, the `≥ fixed` arity rule, and the §5.4 match-vs-subtype
  question all touch inference. Coordinate the typing rules (§3, §5.4) with them.
- **parser/syntax owner — ML param grammar (§2.1).** The `..name`/`..name: T` trailing-formal grammar
  addition in `cadenza-syntax/src/parser.rs`. Coordinate (may be the same person as v-ast-compound's
  surface work).
- **`v-spec-oracle` / spec owner.** A `spec/capabilities/` section for varargs + the `Type.try-as`/
  `Tuple.size` primitives; behavior lands as host-language-independent **corpus cases**, not Rust tests
  (fleet corpus directive).

---

## 8. Increments (top-to-bottom; each independently green, one MR each)

1. **`Tuple.size` primitive (§4).** New `Prim::TupleSize`, prelude field, compile-time fold, typing,
   corpus. Standalone & immediately useful; proves the "add-a-tuple-prim" spine. *(No dependency on the
   rest of the feature.)*
2. **`Type.try-as` primitive (§5).** New `Prim::TryAsType` on the `Type` module, fold, typing
   (`Option b`), non-concrete decline, corpus (a `Some`/`None`/`decline` triad). Coordinate name via
   `ask`. *(Also standalone — usable on any value, not just varargs.)*
3. **List-rest varargs (§3.1).** Extend `is_param_occurrence` + `resolve_lambda`/`def_as_resolved` to
   recognize a trailing `(.. (: v (List T)))`; the placement diagnostics §6.1-6.3; the `apply_lambda`
   list-gather §3.4; the `≥ fixed` arity rule; typing. Corpus: `sum(..)`, zero-arg empty-list, a
   fixed+rest mix, and the mismatch reject. *(Depends on nothing above; the simplest varargs shape,
   single runtime function.)*
4. **Tuple-rest varargs + monomorphization (§3.2/§3.3).** The unannotated / `: Tuple` rest, the
   per-call-site `Core::Tuple` gather (value-dual of `TupleRestFrom`), body specialization. Corpus:
   `describe(1, "hi", true)` reading `Tuple.size` + projecting elements; the compile-time type-branch
   via `Type.try-as` (§5.3). *(Depends on 1, 2, 3.)*
5. **`Type.assert-as` derived helper + polish (§5.3).** The assert-of-type ergonomic built on
   `try-as`; ML surface grammar (§2.1) if not already landed with 3; the guide note; the spec section.
6. **Spec lock + full corpus coverage + guide.** `spec/capabilities/` section, exhaustive edge-case
   corpus (empty rest, single trailing, interior fixed, nested varargs call, recursion), guide chapter.

The design PR (this doc) lands FIRST for operator review; increments 1-2 are unambiguous primitives
that can begin in parallel with the design review (they stand on their own regardless of the varargs
surface decisions), while 3-6 wait on the merged design (esp. the §3.3 default ruling).

---

## 9. The gate (what protects varargs across the fleet)
- **Corpus is authoritative** (`spec/semantics/`): a dedicated file (proposed
  `spec/semantics/NN-varargs.sexp`) plus additions to the tuple/type files. Every behavior above gets a
  case that FAILS if it regresses: list-rest fold, tuple-rest monomorphized branch, `Tuple.size`,
  `Type.try-as` `Some`/`None`/decline, every §6 reject (`(error CDZxxxx)`), and edge cases (empty rest,
  single, interior-fixed, recursion). Run `--target wasm` for cases that execute a value.
- **`rcdzc` unit tests** for the folds (`Tuple.size`, `Type.try-as`) and the `apply_lambda` gather, via
  `dev-gate` / `cargo test -p rcdzc --lib`.
- **New `Prim`s need their RUST-backend arm** per the standing rule — but both new prims ALWAYS fold at
  compile time (§4, §5.2) and never reach a backend, so the arm is the "unreachable/const-only" shape;
  confirm with the rust-backend owner that a fold-only prim needs no emit arm (as `Type.eq` today).

## 10. Open decisions (defaults pinned; operator/peer rulings sought via `ask`, non-blocking)
1. **Unannotated `(.. xs)` default** — §3.3, proposed **tuple-rest**. *(operator)*
2. **`Type.try-as` name** — §5.1, proposed **`Type.try-as`**. *(operator)*
3. **`try-as` match: `==` vs `<:`** — §5.4, proposed **exact structural equality**. *(operator + v-inference)*
4. **`Type.assert-as`** — derived helper vs its own prim — proposed **derived helper** (§5.3/§8.5).
5. **List-rest empty-list element-type inference** — when zero trailing args and `T` otherwise
   unconstrained: proposed **require the annotation to fix `T`** (else a diagnostic), not an unbound
   `List<?>`.

None of these block starting increments 1-2 (the standalone primitives). I proceed on the pinned
defaults and adjust if an `answer` overrules.
