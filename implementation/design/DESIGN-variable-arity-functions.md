# DESIGN — variable-arity (varargs) functions: `(.. v)` rest params → list or tuple

> **Status:** DESIGN — OPERATOR-REVIEWED, RULINGS INCORPORATED (PR #7462 review 2026-09-01; §10). Still
> DO NOT auto-merge — the operator merges when ready. Subsystem: `rcdzc` (seed compiler), with
> coordinated touches to `cadenza-syntax` (ML surface), the `Type` reflection module (shared with
> `v-metaprogramming`), and `spec/`.
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
- **`Type.try-as : a -> Option b`** — compile-time "view this value at the expected type" (NEW; the
  operator's `ty -> a -> Option b`, with the target type **inferred from usage** rather than passed —
  per his review ruling §5; on the `Type` module).
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
Arity: a varargs callee requires **at least as many arguments as it has fixed parameters** — with `k`
fixed params, at least `k` args; every argument beyond the `k`th is gathered into the rest. Fewer than
`k` args still curries (partial application) exactly as today — the rest param only engages once all
fixed params are satisfied (with zero trailing args the rest binds the empty list/tuple).
`infer/application.rs` arity checks (215-253) grow a "≥ fixed-count" rule for a varargs callee instead
of the exact `== N` equality.

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

## 5. `Type.try-as : a -> Option b` (NEW primitive — target type INFERRED from usage)

The operator: *"a way to try getting a value with a given type … return `Some v` iff the provided
value is the provided type … put it on the Type module."*

**Operator ruling (PR #7462 review):** the name is **`Type.try-as`** (confirmed); the match is
**strict** (same behavior as `Type.eq`); and — the key shape change — **the target type is NOT passed
as an explicit argument. It is INFERRED from usage.** The operator, verbatim: *"drop the type argument
altogether and have that be the default everywhere … to force a specific return value the caller can
always use type ascription"* and *"It should be inferred on usage I think. If it's not inferrable then
we should error and ask for explicit annotations."* So the signature is not `Type -> a -> Option b`; it
is `a -> Option b` with `b` fixed by the surrounding context.

### 5.1 Name — `Type.try-as` (RESOLVED)
Operator: *"Type.try-as looks good to me."* Reads as "try to view this value **as** the expected type",
the `try-` prefix signals the `Option` (fallible) result; kebab-case matches the language. It lives on
the `Type` module namespace even though it no longer takes a `Type` *value* — it is the type-directed
"typed peek" dual of the reflection fields.

### 5.2 Signature & semantics
```
Type.try-as : a -> Option b     -- b is inferred from the expected type at the use site
```
- **No `Type` argument.** The target type `b` is whatever `Option b` the result flows into (the
  expected type from inference); to *force* a target, the caller ascribes: `(Type.try-as x : Option Int)`.
- **Semantics (compile-time fold):** let `Tv = type_of(x)`. If `Tv` **structurally equals** `b` (the
  same equality machinery as `Type.eq` — strict, §5.4), fold to `Some (x : b)`; otherwise fold to
  `None : Option b`.
- **Target inferability is required — else a diagnostic.** `b` must be determined by the context
  (an expected `Option b` with `b` concrete, or an explicit ascription). If inference leaves `b` a free
  type variable, it is a **compile-time error** ("`Type.try-as` cannot infer the target type here; add
  a type ascription, e.g. `(Type.try-as x : Option T)`") — per the operator's "error and ask for
  explicit annotations". This replaces the earlier non-concrete-`Type`-value decline.
- **Pure, one-tier.** Because `b` is fixed by inference and `x`'s type is statically inferred, this
  ALWAYS folds to a definite `Some`/`None` at compile time — **no runtime type tag, no runtime branch**.
  This is why it composes perfectly with tuple-rest monomorphization: after a tuple-rest call is
  specialized, each element's type is concrete, so `Type.try-as` on it is decided at compile time.

### 5.3 This is the type-branching primitive
"branch on what types were passed" falls out directly (each arm ascribes the target it tests):
```cadenza
def render(.. xs) =                       ;; tuple-rest, monomorphized per call
  Tuple.fold(xs, "", fn(acc, x) =>        ;; (illustrative; each element folds concretely)
    match (Type.try-as x : Option Int) with
    | Some(n) => acc ++ Int.to-str(n)
    | None => match (Type.try-as x : Option Str) with
              | Some(s) => acc ++ s
              | None => acc ++ "?")
```
- **branch on types passed** = `match` on an ascribed `Type.try-as` (or on `Type.eq(Type.of(x), T)`),
  folded per monomorphized call-site.
- **assert a value is of a certain type** — **no dedicated helper.** Operator ruling: *"The caller can
  always use `Option.expect`. I don't think we need the helper."* So an assert is just
  `Option.expect((Type.try-as x : Option Int), "…")` — `try-as` + the existing `Option.expect`; no new
  `Type.assert-as` surface.

### 5.4 Match semantics — strict structural equality (RESOLVED)
Operator: *"Yes strict checks"* / *"it should be the same behavior as `Type.eq`."* So `Type.try-as`
succeeds **only** when `type_of(x)` structurally **equals** `b` (reusing `Type.eq`'s equality) — no
subtype widening. Coordinated with `v-metaprogramming` (owns `Type.eq`; ack: reflection reads
structure, does not redefine equality — no conflict).

### 5.5 Implementation seams (mirror `Type.of`/`Type.eq`/`Type.ast`)
| Concern | Anchor | Change |
|---|---|---|
| Prim | `resolved.rs:71` enum + `from_name` ~1028 (Type prims) | add `Prim::TryAsType`, intrinsic `type-try-as` |
| Prelude field | `type_module` (`prelude.rs:1926`) | register `try-as` via `ctor_record` with a `(meta t)` scheme `a -> Option b` |
| Reduction | `eval.rs:2893` (`Prim::TypeOf`), 3183-3213 (structural recognition) | add the `Prim::TryAsType` arm: read the expected/ascribed `b`, compare `type_of(x)` via `Type.eq` equality, fold `Some`/`None` |
| Typing | inference of `Type.*` fields | result `Option b` where `b` is unified with the expected type; if `b` stays free → the "cannot infer target" error |
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
diagnostics vertical's bar (a concrete applicable fix, not just "what went wrong").

---

## 7. Peer coordination & the territory boundary (CRITICAL)

- **`v-ast-compound` — the `(.. v)` marker.** They own **value-position** spread (collection
  *construction* `#list(a (.. c))`). I own **parameter-position** `(.. v)` (fn/def formals). The two
  share ONLY the arena node shape `(.. operand)` — which is exactly the point (one consistent marker).
  They are disjoint syntactic contexts (a ctor child vs a param-list element), touched in different
  resolvers (`resolve_list`/`resolve_record` vs `resolve_lambda`/`is_param_occurrence`), so there is no
  file race on the recognition site. **Coordination confirmed (their ack):** the canonical shared
  recognizer is **`Arenas::spread_operand(id) -> Option<StructId>`** (in `cadenza-ast/src/ast.rs`) — the
  thin per-element wrapper over `as_form(id, "..")`; their value-position spread detects spreads
  exclusively through it. **Param-position recognition routes through the SAME `spread_operand`** so both
  contexts stay byte-consistent (there is also `Arenas::rest_marker(elems)` for the single-rest scan used
  by pattern destructuring — for a fn-formal list I use `spread_operand` for the trailing-element test).
  They committed to ping me before any rename and to shape a name-binder-accepting variant if formals
  need one.
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
2. **`Type.try-as` primitive (§5).** New `Prim::TryAsType` on the `Type` module, fold, typing (result
   `Option b` with `b` inferred from usage/ascription), the cannot-infer-target error, corpus (a
   `Some`/`None`/`cannot-infer` triad). *(Also standalone — usable on any value, not just varargs.)*
3. **List-rest varargs (§3.1).** Extend `is_param_occurrence` + `resolve_lambda`/`def_as_resolved` to
   recognize a trailing `(.. (: v (List T)))`; the placement diagnostics §6.1-6.3; the `apply_lambda`
   list-gather §3.4; the `≥ fixed` arity rule; typing. Corpus: `sum(..)`, zero-arg empty-list, a
   fixed+rest mix, and the mismatch reject. *(Depends on nothing above; the simplest varargs shape,
   single runtime function.)*
4. **Tuple-rest varargs + monomorphization (§3.2/§3.3).** The unannotated / `: Tuple` rest, the
   per-call-site `Core::Tuple` gather (value-dual of `TupleRestFrom`), body specialization. Corpus:
   `describe(1, "hi", true)` reading `Tuple.size` + projecting elements; the compile-time type-branch
   via `Type.try-as` (§5.3). *(Depends on 1, 2, 3.)*
5. **Polish (§5.3).** ML surface grammar (§2.1) if not already landed with 3; the guide note.
   (No `Type.assert-as` helper — operator ruling: assert via `Option.expect((Type.try-as x : Option T), …)`.)
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
- **Corpus is the authoritative gate for ALL observable semantics** (fleet corpus-first directive): the
  `Tuple.size` values, the `Type.try-as` `Some`/`None`/inference-error outcomes, the varargs binding, and
  every diagnostic are pinned as `spec/semantics/` cases — NOT Rust `#[test]`s. Rust unit tests in
  `rcdzc` are reserved for **non-observable internal invariants** the corpus cannot express — e.g. the
  `apply_lambda` gather producing the right `Core::Tuple`/`List.new` shape, or the reparenting
  correctness of a synthesized node — checked via `dev-gate` / `cargo test -p rcdzc --lib`. Any behavior
  a Cadenza program can observe belongs in the corpus.
- **New `Prim`s need their RUST-backend arm** per the standing rule — but both new prims ALWAYS fold at
  compile time (§4, §5.2) and never reach a backend, so the arm is the "unreachable/const-only" shape;
  confirm with the rust-backend owner that a fold-only prim needs no emit arm (as `Type.eq` today).

## 10. Decisions — RESOLVED by the operator's PR #7462 review (2026-09-01)
1. **Unannotated `(.. xs)` default** — §3.3, **tuple-rest** (no operator objection; proceeding on the
   pinned default per the greenlight-on-defaults directive).
2. **`Type.try-as` name** — §5.1, **`Type.try-as`** ✅ (*"looks good to me"*).
3. **`try-as` match: `==` vs `<:`** — §5.4, **strict structural equality (same as `Type.eq`)** ✅
   (*"Yes strict checks"* / *"same behavior as Type.eq"*).
4. **`try-as` target type** — §5.2, **NOT an explicit `Type` argument — inferred from usage; error and
   ask for an annotation if not inferrable; force via type ascription** ✅ (*"drop the type argument
   altogether … it should be inferred on usage … if it's not inferrable then we should error and ask
   for explicit annotations"*).
5. **Assert-of-type helper** — **none; use `Option.expect`** ✅ (*"The caller can always use
   `Option.expect`. I don't think we need the helper."*).
6. **List-rest empty-list element-type inference** — when zero trailing args and `T` otherwise
   unconstrained: **require the annotation to fix `T`** (else a diagnostic), not an unbound `List<?>`
   (consistent with the operator's "infer-or-error-and-ask" principle for `try-as`).

All rulings incorporated above. No blockers remain; increments 1-2 (the standalone primitives) proceed.

---

## Addendum A — call-site argument SPLAT (`f(.. t)` / `f(.. xs)`)

> **Status:** DESIGN ADDENDUM — DRAFT FOR OPERATOR REVIEW (do NOT auto-merge). Follows the merged
> base design; operator follow-on (relayed by concierge, 2026-09-01).
>
> **Operator intent (verbatim):** *"we probably need the same mechanism at the call site. So you'd be
> able to splat a number of args from a compile-time known tuple, or even a runtime list?"* and the
> refinement *"I guess the tuple values can be runtime on the splat too, assuming they're not also
> marked const in the function definition."*

### A.1 The shape
The param-side rest `(.. v)` GATHERS trailing args; the call-site splat is its DUAL — it SPREADS a
tuple/list into a call's argument list, reusing the same value-position `(.. operand)` marker
v-ast-compound ships for collection construction:

| call | splat source | expands to |
|---|---|---|
| `(f a (.. t) b)` | tuple `t : Tuple(T0 T1)` | `(f a (. t 0) (. t 1) b)` — arity static |
| `(f a (.. xs))` | list `xs : List T` | feeds `f`'s list-rest param the (segment-folded) list |

Both are the value-position `(.. operand)` node appearing in a CALL argument position — a NEW consumer
site for the marker. **Territory (confirmed with v-ast-compound):** the `Arenas::spread_operand`
recognizer is context-INDEPENDENT (shape-only) so it is REUSED verbatim to detect a `(.. v)` child in a
call's argument list; but the LOWERING is v-varargs' own new consumer site — v-ast-compound's
segment-and-fold fires ONLY in the compound-CONSTRUCTOR resolved arms (`Resolved::List/Set/Tuple` +
`entry_spread_desugar` for Record/Map), which never see a CALL application node. So v-ast-compound owns
`(.. v)` in COMPOUND CONSTRUCTION; v-varargs owns it in PARAM position AND CALL-ARGUMENT position.

### A.2 Tuple splat — compile-time positional expansion (arity static, values may be runtime)
`(f … (.. t) …)` with `t : Tuple(T_0 … T_{n-1})`: the arity `n` is a **static property of the tuple's
type** (`Tuple.size`, increment-1 primitive), so the splat **expands at compile time** into `n`
positional arguments `(. t 0) … (. t (n-1))` spliced into the call in place. Each `(. t i)` is an
ordinary tuple projection (`Core::Proj`, already exists) — so **the values may be RUNTIME** (a
projection of a runtime-valued tuple); only the ARITY must be static. This is the same one-tier
compile-time expansion the reducer already does; no new runtime capability.

**Implementation (mirror v-ast-compound's `lower_tuple_spread`, per their ack):** the static-tuple
arg-splat is exactly the tuple-CONSTRUCTION splat, emitting call ARGS instead of tuple elements —
expand the tuple operand into per-slot `Core::Proj` occurrences (arity from its type), and MATERIALIZE
a runtime tuple operand ONCE via a self-keyed `Core::Let` (the `materialize_row_op_operand` pattern) so
the `n` projections share ONE evaluation (no re-eval of a runtime tuple per slot). v-ast-compound
offered to factor `lower_tuple_spread`'s projection-flatten + materialize-once into a shared helper
v-varargs can call — take them up on that to avoid duplicating the materialize logic.

- **Interleaving:** inline args and tuple splats compose positionally — `(f a (.. t) b)` with
  `t : Tuple(X Y)` → `(f a x0 y0 b)`, a 4-arg call. Multiple tuple splats concatenate positionally.
- **Into a tuple-rest param:** a tuple splat feeding a tuple-rest callee (base design §3.2) contributes
  its elements to the monomorphized rest tuple — the two mechanisms compose.

#### A.2a `const`-param interaction (operator refinement)
`const` parameters already exist (`(const (: d T))` binder, `strip_const_params`, `db.const_params`).
The rule: a splatted tuple's VALUES need **not** be const in general — but if the target `f`'s parameter
at the landing position is declared `const`, then the corresponding splatted element **must** be a
compile-time constant (the existing const-param requirement), else the existing const-param diagnostic
fires. Const-ness is governed by the FUNCTION DEFINITION, not the splat: `arity` is what the splat
requires to be static; per-position value-const-ness is `f`'s contract. Because the tuple splat expands
to positional `(. t i)` occurrences BEFORE the const-param pass, `strip_const_params`/`type_specialize`
sees ordinary positional args and enforces const-ness per position with no new machinery.

### A.3 List splat — dynamic count, feeds a list-rest param
`(f … (.. xs))` with `xs : List T`: a runtime list has **no static arity**, so it cannot expand to
fixed positions. It is only well-formed when the argument it lands in is a **list-typed rest parameter**
(base design §3.1): the splat contributes `xs`'s elements to that rest list. Inline trailing args +
list splats compose via the **same segment-and-fold** v-ast-compound built for collection construction
(`[a, ..xs, b]` ≡ `concat`): the rest list is `concat(segment_0, xs, segment_1, …)`. A list splat into a
FIXED (non-rest) parameter position, or into a tuple-rest (which needs a static arity), is a
compile-time error (A.4).

### A.4 Diagnostics (each pins a corpus reject)
1. **List splat into a fixed / tuple-rest position** — "a list splat `(.. xs)` has no static arity;
   it can only feed a list-typed rest parameter. Use a tuple to splat into fixed positions."
2. **Arity mismatch after tuple expansion** — a tuple splat expanding to the wrong positional count
   reuses the ordinary arity diagnostic (the expansion is checked like hand-written positional args).
3. **`const` param fed a non-const splatted value** — the existing const-param diagnostic, at the
   landing position.
4. **Splat of a non-tuple / non-list** — "`(.. v)` in a call argument requires a tuple (static-arity
   splat) or a list (into a rest parameter); found `<T>`."

### A.5 Increments (append after base-design increment 6)
7. **Tuple splat into fixed params** — recognize `(.. t)` in a call arg list; expand to `(. t i)`
   positionally via `Tuple.size` + `Core::Proj`; arity + const-param checks. Corpus: static-arity
   splat, runtime-valued tuple splat, interleaved inline+splat, the const-param case, the reject.
8. **List splat into a list-rest param** — the segment-and-fold feeding a list rest (reuses
   v-ast-compound's construction fold). Corpus: pure list splat, inline+splat concat, the
   fixed-position reject.
9. **Composition** — tuple splat into a tuple-rest callee; list splat into a list-rest callee; mixed.
   Corpus + guide.

### A.6 Coordination
v-ast-compound owns the value-position `(.. v)` marker + segment-and-fold; the call-arg site is a NEW
consumer — split territory so the recognizer (`spread_operand`) stays one shared idiom. v-inference for
the tuple-expansion typing + list-rest concat typing + the const-param-at-position rule.

### A.7 Implementation note — expansion MUST run at type-check time (learned from #7712)

> **Status:** DESIGN NOTE — DRAFT FOR OPERATOR REVIEW (do NOT auto-merge). Records the timing
> constraint discovered while landing #7712 so the remaining two cases (A.5 increments 7–9) are built
> with the correct expansion phase; the earlier addendum framed the expansion as a *lowering* step,
> which is insufficient for the fixed-params callee.

**What #7712 shipped (all the common paths):** a single-source **transparent resolve** in
`resolve.rs` (`resolved_of`, before `compute`) resolves a call-arg `(.. operand)` node structurally
*as the operand* (no `type_of` recursion) when the parent is a non-construction application AND either
the operand is a syntactic tuple OR the callee `callee_is_varargs`. `apply_lambda_uncached` then
expands the marker via the AST (literal-tuple splice; per-slot `(. t i)` for a tuple Ref/Param;
list-rest gather). This fixes: literal-tuple splat into ANY callee, tuple-var splat into a **varargs**
callee, and mixed/multi list-splat (`f(a, .. xs)`, `f(.. a, .. b)`).

**The two cases still declining (both decline cleanly — CDZ0201/CDZ0203, never miscompile):**
1. **param-relay** — a tuple **Ref/Param** splat into a **non-varargs, fixed-params** callee, e.g.
   `def relay(t: Tuple(A B C)) = a3(.. t)`.
2. **non-ref-operand materialize** — `(.. <expr>)` where the operand is not a bare Ref/Param and must
   be evaluated once before its `n` projections.

**Why resolve-time transparency is not enough for case 1.** Making `(.. t)` resolve *as* the tuple
lets it type **as the tuple** — so `check_application` then unifies the whole `Tuple(A B C)` against the
callee's **first fixed parameter** (e.g. `A = Int64`) and reports `CDZ0203 annotation type Int64 does
not match value type (Tuple …)` — *before* any positional expansion runs. The expansion is invisible
to the type checker because it lives in `apply_lambda_uncached` (a reduce/lower-adjacent phase) and in
resolve (which only rewrites the node's *identity*, not the call's **arity**).

**The fix is a type-time call-arg expansion pass.** `(.. t)` in a call-argument list must be expanded
into its `n` positional projection args **before `check_application` types the call** — i.e. the arity
rewrite `(f a (.. t) b)` → `(f a (. t 0) (. t 1) (. t 2) b)` must be observable to the type checker,
not just the reducer. Concretely, the pass:
- runs where call arguments are assembled for typing (`collect_node`'s `Resolved::Apply` arm →
  `infer/application.rs` `check_application`), so each projected slot is typed as an ordinary
  positional arg against the callee's corresponding fixed param;
- reads the operand's tuple arity from its **type** (`Tuple.size` on the inferred `Ty::Tuple`), which
  requires the operand already be typed — so the expansion is a *pre-pass over the arg list keyed on
  the operand's inferred type*, distinct from the pure-structural resolve guard;
- for case 2, binds the operand once (self-keyed `Core::Let` / `materialize_row_op_operand`) so the `n`
  projections share one evaluation — the materialize half of the same pass.

This unifies both remaining cases under **one** designed pass and leaves the resolve-time transparency
of #7712 in place for the varargs/list-splat paths it correctly handles. It is deliberately NOT a
monitor-tick patch: two prior CI regressions (#7612 round-trip, #7629 diagnostic) came from touching
this seam reactively, so it is scoped here as increment 7's real shape.

### A.7a — param-relay LANDED (#7802); the mechanism, corrected

**Status: DONE.** Increment 7's param-relay half shipped in #7802. The implementation is SIMPLER than the
speculation above and corrects two of its assumptions — recorded so the materialize-once half (A.7b) is
built on the real mechanism:

- **NO transparent-resolve change was needed or wanted.** Extending #7712's transparent resolve to the
  tuple-Ref case is actively HARMFUL: β-reduction copies a `(.. t)` argument through its *resolved* form,
  so a transparent `(.. t)` → `t` copy LOSES the spread marker, and the callee is then curried with one
  tuple argument (`(+ Tuple Int64)` / a `(-> Any …)` residual). The resolve layer is left exactly as
  #7712 shipped it.
- **The fix is the shared expansion helper + an annotation peel — not a resolve rewrite.**
  1. `apply_lambda_uncached`'s call-splat expansion was extracted into `eval::expand_call_splat_args`, and
     `check_application` calls it **before the param↔arg zip** in its lambda arm. So the type checker and
     the reducer run the *same* expansion and agree on the positional arg list; the zip binds the tuple's
     elements to the fixed params instead of unifying the whole tuple against param 0. (No `Tuple.size`
     pre-pass keyed on inferred type was needed — the existing branch (ii) already reads `Ty::Tuple` arity.)
  2. The helper PEELS a leading `(: v T)` annotation: β-substitution wraps a splatted argument in its
     parameter's annotation (`substituted_arg`), so the reduced `main → relay` body's operand arrives as
     `(: #tuple(…) (Tuple …))`; peeling lets branch (i)/(ii) reach the underlying tuple.
- **The helper is a strict no-op unless a `(.. )` call-argument is present**, so it cannot regress a
  non-splat program — which is what made a direct-to-main self-merge safe (gated on `corpus-09-functions`
  / `-15-rows` / `-07-type-system` + `corpus_roundtrip`).

### A.7b — materialize-once (the sole remaining case)

`(.. <expr>)` where the operand is a tuple-COMPUTING expression (not a syntactic tuple, not a bare
Ref/Param) — `f(.. (mk))` — still declines cleanly (CDZ0201). `expand_call_splat_args` cannot handle it
because expanding to `(. (mk) 0) … (. (mk) k-1)` would re-evaluate `(mk)` per slot (wrong if it performs
an effect), and the helper returns a flat arg list so it cannot introduce the binding that would evaluate
`(mk)` ONCE. The materialize-once needs a `(let ((tmp (mk))) (f (. tmp 0) …))` wrap around the
application — reusing the `evalonce_wraps` / `materialize_row_op_operand` machinery — gated on the operand
actually needing it (a pure operand could re-eval). This is a distinct, more invasive change than the
arg-list expansion; it is the one open varargs follow-up.
