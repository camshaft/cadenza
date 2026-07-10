I have verified every seam cited in the survey against the live source. All line numbers check out. Here is the redirect spec.

---

# REDIRECT SPEC — rcdzc Layer 2: replace the hard-coded `TypeCtor` spike with the Type-builder Intrinsic (closures winner, Design A)

**Crate root:** `/Users/bythewc/Projects/camshaft/cadenza/implementation/seed/crates/rcdzc/src/`

## Rationale (why closures beat the hard-coded spike)

The spike bolted a fourth compile-time value onto the IR — a `TypeCtorKind` enum plus three parallel IR rungs (`Hir`/`TypedNode`/`Mir::TypeCtor`), a bespoke `Apply` fold arm (`fold.rs:766-795`), a dedicated infer typing arm (`infer.rs:578-590`), a second annotation-time β-reducer that *duplicates* the fold's compound-builder (`extract_type_value`, `infer.rs:1119-1156`), a `lower` arm, a `select` decline, and ~10 inert-leaf guards. Every one of those is a hand-rolled parallel path that must be kept coherent with the existing `Intrinsic` machinery it mirrors. The winning design observes that a type constructor **is already an intrinsic**: "an op applied to value args produces a constructed result." A `List`/`Map`/`Set`/`Tuple`/`Option`/`Result` bound as its own `Intrinsic` singleton rides the *existing* `Hir::Intrinsic → TypedNode::Intrinsic → Mir::Intrinsic` rungs, which already thread through fold's `Apply(Intrinsic)` const-fold arm (`fold.rs:744-751`), the generic `signature()`/`param_count`/`instantiate` typing path (`infer.rs:222-231`), `lower`, `select`'s bare-`Intrinsic` decline (`select.rs:180`), layout's inert-leaf guards, and `is_transient` (`fold.rs:964`, which already admits `Mir::Intrinsic`). So the redesign **deletes** three IR node variants, an enum, two full β-reducers, a typing arm, a lower arm, a select decline, and every `TypeCtor` guard mention, and **adds only** enum arms plus `fold_const`/`signature`/`build_compound_ty` logic. Nominal identity is the enum variant (mirroring the scalar `TypeVal`s at `prelude.rs:58-59` and the sum singletons at `prelude.rs:78-101`); the constructed type's identity for `Option`/`Result` still routes through the `ty.rs` process-global `SumRef` singletons. The one genuine cost — widening the `all(is_const)` gate at `fold.rs:745` to admit `Mir::TypeVal` — is contained because only type-builder intrinsics ever receive `TypeVal` args in well-typed code, and every other `fold_const` arm already returns `None` on shapes it does not recognize. `is_const` was **already** documented (`fold.rs:960-961`) as intended to widen for exactly this.

---

## 1. WHAT TO REVERT (the complete spike surface)

Delete each of the following. Line numbers are current; delete by content.

**`ir.rs`**
- The `TypeCtorKind` enum — `ir.rs:103-121` (the doc comment at 103-106 and the `enum TypeCtorKind { List, Map, Set, Tuple2, Option, Result }` at 107-121).
- `Hir::TypeCtor(TypeCtorKind)` — `ir.rs:486-491` (variant + doc).
- `TypedNode::TypeCtor(TypeCtorKind)` — `ir.rs:591-592`.
- `Mir::TypeCtor(TypeCtorKind)` — `ir.rs:679-681`.

**`fold.rs`**
- The entire `Mir::TypeCtor(kind)` arm inside the `Apply` match — `fold.rs:766-795` (from `Mir::TypeCtor(kind) => {` through the closing `}` before the `other =>` arm at 796). This is the hand-rolled parallel β-reducer that the winner removes.
- Remove `| Mir::TypeCtor(_)` from each inert-leaf guard: `fold.rs:280` (`collect_reached_poisons`), `fold.rs:316` (`alpha_rename` leaf), `fold.rs:480` (`fold` catch-all leaf), `fold.rs:964` (`is_transient`), `fold.rs:1125` (`max_local_id`), `fold.rs:1140` (`substitute` leaf), `fold.rs:1318` (`collect_calls`). Leave `Mir::TypeVal(_)` in every one of those arms — TypeVal is unchanged.
- Fix stale comment at `fold.rs:477` (mentions "TypeVal/TypeCtor" — drop the TypeCtor half) and `fold.rs:637`.

**`infer.rs`**
- The `Hir::TypeCtor(kind)` typing arm — `infer.rs:578-590`. (Its `Fn([Type,…],Type)` result is reproduced free by the generic `Intrinsic` `signature()` path — see §4.)
- The `TypedNode::TypeCtor(kind)` finalize passthrough — `infer.rs:1104`.
- Remove `| Hir::TypeCtor(_)` from `hir_uses_local` — `infer.rs:950`.
- `extract_type_value` — `infer.rs:1119-1156`: do **not** delete the function; **rewrite** it (see §4). Its `TypeCtorKind` match at 1133-1149 is deleted and re-pointed at the shared helper.

**`lower.rs`**
- The `TypedNode::TypeCtor(kind) => Mir::TypeCtor(kind)` arm — `lower.rs:221-223`.

**`select.rs`**
- The `Mir::TypeCtor(_)` decline — `select.rs:316`. (A bare/under-applied type-builder now declines through the pre-existing bare-`Intrinsic` arm at `select.rs:180` — see §7.)

**`layout.rs`**
- Remove `| Mir::TypeCtor(_)` from `collect_callees` (`layout.rs:186`) and `body_uses_heap` (`layout.rs:228`). Leave `Mir::TypeVal(_)`.

**`prelude.rs`**
- The TypeCtor block — `prelude.rs:65-72`: the `use crate::ir::TypeCtorKind;` (65), the four `p.insert(... Hir::TypeCtor(...))` for List/Map/Set/Tuple (66-69), and the Option/Result skip comment (70-72). Replaced per §2. (Note: the List/Map/Set inserts here are already dead — shadowed by the module records at `prelude.rs:145-190` via HashMap overwrite; only `Tuple` was ever live.)

After reverting, `grep -rn TypeCtor src/` must return **zero** hits.

---

## 2. WHAT TO ADD — IR surface + prelude bindings

### 2a. Six new `Intrinsic` enum variants (`ir.rs`, in `enum Intrinsic` at `ir.rs:132-190`)

Add, before `Heap(HeapIntrinsic)`:

```rust
    /// `List : Type → Type` — the parametric list type constructor as a first-class compile-time
    /// value. Applied to a type-value it folds to `TypeVal(Ty::List(elem))`. Layer 2.
    TypeList,
    /// `Map : Type → Type → Type` — key type, value type → `TypeVal(Ty::Map(k, v))`.
    TypeMap,
    /// `Set : Type → Type` — element type → `TypeVal(Ty::Set(elem))`.
    TypeSet,
    /// `Tuple : Type → Type → Type` — 2-arity for L2 → `TypeVal(Ty::Tuple([a, b]))`.
    TypeTuple,
    /// `Option : Type → Type` — → `TypeVal(Ty::Sum{prelude_option(), [a]})`. Identity is this variant;
    /// the constructed type's identity is the shared `ty::prelude_option()` singleton.
    TypeOption,
    /// `Result : Type → Type → Type` — → `TypeVal(Ty::Sum{prelude_result(), [a, e]})`.
    TypeResult,
```

These reuse the existing `Hir::Intrinsic` (ir.rs:585 in `TypedNode`, and `Hir` at the value level), `TypedNode::Intrinsic`, and `Mir::Intrinsic(ir.rs:669)` rungs — **no new node variant is added.**

### 2b. Prelude bindings (`prelude.rs`, replacing the deleted 65-72)

Replace with:

```rust
    // ── Layer 2: parametric type constructors as first-class compile-time VALUES — each bound to its
    //    own `Intrinsic` singleton (nominal identity IS the enum variant, mirroring the scalar TypeVals
    //    above and the sum singletons below). Applied to type-value args, the op's `fold_const` builds
    //    the compound `Ty`. `(List Int64)` → `TypeVal(Ty::List(Int))`. `List`/`Map`/`Set` ALSO name
    //    their operation MODULE RECORD (inserted below at ~145-190); the collision is resolved in
    //    resolve.rs — bare-name → the type-builder Intrinsic, member access `(. List push)` → the
    //    record field. `Tuple` has no module record; `Option`/`Result` also name their ctor records. ──
    p.insert("List".to_string(), Hir::Intrinsic(Intrinsic::TypeList));
    p.insert("Map".to_string(), Hir::Intrinsic(Intrinsic::TypeMap));
    p.insert("Set".to_string(), Hir::Intrinsic(Intrinsic::TypeSet));
    p.insert("Tuple".to_string(), Hir::Intrinsic(Intrinsic::TypeTuple));
    p.insert("Option".to_string(), Hir::Intrinsic(Intrinsic::TypeOption));
    p.insert("Result".to_string(), Hir::Intrinsic(Intrinsic::TypeResult));
```

**IMPORTANT — the collision (survey `prelude_shape`, "CRITICAL COLLISION").** `List`/`Map`/`Set` are re-inserted as operation module records at `prelude.rs:145-190` (a HashMap `insert` overwrites), and `Option`/`Result` are re-inserted as ctor records at `prelude.rs:100`. So the six inserts above are dead unless bare-name resolution is routed to them explicitly. That routing is §5's resolve change — do **not** attempt to keep both roles in the prelude map. (The module records at 100 and 145-190 stay untouched, so `(. List push)` / `(. Option Some)` continue to project via the separate `member()` path, `resolve.rs:953-1010`.)

### Exact Hir for `List` and for a 2-arg ctor (`Map`)

- `List` as a value → `Hir::Intrinsic(Intrinsic::TypeList)`.
- `(List Int64)` → `Hir::Apply { func: Box::new(Hir::Intrinsic(Intrinsic::TypeList)), args: vec![Hir::TypeVal(Ty::Int)] }`.
- `Map` as a value → `Hir::Intrinsic(Intrinsic::TypeMap)`.
- `(Map K V)` → `Hir::Apply { func: Box::new(Hir::Intrinsic(Intrinsic::TypeMap)), args: vec![Hir::TypeVal(<K>), Hir::TypeVal(<V>)] }`.

### 2c. The leaf builder primitive's home + fold behavior

The irreducible leaf lives in **`Intrinsic::fold_const` (`ir.rs:356-398`)**. Add, in the match, before `Intrinsic::Heap(_) => None`:

```rust
            // Layer 2 type builders: read the `Ty` out of each `Mir::TypeVal` arg and construct the
            // compound type-value. The one place a compound `Ty` is fabricated from type args; shared
            // with annotation-time extraction via `build_compound_ty` so the two paths cannot drift.
            Intrinsic::TypeList | Intrinsic::TypeMap | Intrinsic::TypeSet
            | Intrinsic::TypeTuple | Intrinsic::TypeOption | Intrinsic::TypeResult => {
                let arg_tys: Vec<Ty> = args.iter()
                    .filter_map(|a| match a { Mir::TypeVal(t) => Some(t.clone()), _ => None })
                    .collect();
                if arg_tys.len() != args.len() { return None; } // not all TypeVals → stays residual
                self.build_compound_ty(&arg_tys).map(Mir::TypeVal)
            }
```

Add the **shared helper** (best_graft 3 — unify the two reducers) as an `impl Intrinsic` method in `ir.rs`, reproducing the `(kind, arg_tys)` match currently at `fold.rs:778-790` / `infer.rs:1134-1149`:

```rust
    /// Build the compound `Ty` a type-builder intrinsic produces from its type arguments, or `None`
    /// (arity mismatch / not a type builder). The ONE compound-Ty constructor — called by both
    /// `fold_const` (fold time, wrapping in `Mir::TypeVal`) and `infer::extract_type_value`
    /// (annotation time, before fold) so the two paths cannot drift. Option/Result route through the
    /// process-global `ty::prelude_option()`/`prelude_result()` singletons so identity is `Arc::ptr_eq`.
    pub fn build_compound_ty(self, arg_tys: &[Ty]) -> Option<Ty> {
        match (self, arg_tys) {
            (Intrinsic::TypeList, [e]) => Some(Ty::List(Box::new(e.clone()))),
            (Intrinsic::TypeSet, [e]) => Some(Ty::Set(Box::new(e.clone()))),
            (Intrinsic::TypeMap, [k, v]) => Some(Ty::Map(Box::new(k.clone()), Box::new(v.clone()))),
            (Intrinsic::TypeTuple, [a, b]) => Some(Ty::Tuple(vec![a.clone(), b.clone()])),
            (Intrinsic::TypeOption, [a]) =>
                Some(Ty::Sum { def: crate::ty::prelude_option(), args: vec![a.clone()] }),
            (Intrinsic::TypeResult, [a, e]) =>
                Some(Ty::Sum { def: crate::ty::prelude_result(), args: vec![a.clone(), e.clone()] }),
            _ => None,
        }
    }
```

(`Ty` and `Mir` are already in scope in `ir.rs`.)

---

## 3. HOW `(List Int64)` AND `(Map K V)` FOLD

Both ride the **existing `Apply(Intrinsic)` const-fold arm at `fold.rs:744-751`** — no new arm.

For `(List Int64)`: fold produces `Mir::Apply { func: Mir::Intrinsic(TypeList), args: [Mir::TypeVal(Ty::Int)] }`. `func` folds to `Mir::Intrinsic(TypeList)`; spine collapse (`fold.rs:685-701`) does not match (callee is neither `FuncRef`/`Lambda`/`Call`), so `callee = Mir::Intrinsic(TypeList)`, `all_args = [Mir::TypeVal(Int)]`. The `Mir::Intrinsic(op)` arm (`fold.rs:744`) checks the (now-widened, §4a) gate, calls `op.fold_const(&[TypeVal(Int)])` → `Some(Mir::TypeVal(Ty::List(Int)))` and returns it.

For `(Map K V)`: identical, two args `[TypeVal(<K>), TypeVal(<V>)]` → `fold_const` → `Some(Mir::TypeVal(Ty::Map(K,V)))`.

---

## 4. `infer` CHANGES

### 4a. Widen the const-fold gate (`fold.rs:745`) — the one invariant cost

Change:
```rust
                        if all_args.iter().all(is_const) {
```
to:
```rust
                        if all_args.iter().all(|a| is_const(a) || matches!(a, Mir::TypeVal(_))) {
```
Update the doc comment at `fold.rs:741-743` to note type-builder intrinsics fold over `TypeVal` args. This is the deliberate widening the code already anticipated at `fold.rs:960-961`. It is safe: a non-type-builder op handed a `TypeVal` returns `None` from `fold_const` (every arm matches only its own value shapes) and stays a residual `Apply` — which in well-typed programs never occurs (mixing a `TypeVal` into a numeric op is a `Ty` mismatch caught at infer). Do **not** touch the `is_const`/`is_transient` definitions themselves (`fold.rs:940-946`, `962-971`); `is_transient` already admits `Mir::Intrinsic` and `Mir::TypeVal`.

### 4b. Application typed to `Ty::Type` — comes free

No change. A type-builder is `Hir::Intrinsic`, typed by the generic arm at `infer.rs:222-231`: `op.signature()` → `(params, ret)`, `op.param_count()` = 0 (see §6), so it instantiates trivially to `Ty::Fn([Ty::Type,…], Ty::Type)`. Application then rides the strict `Fn`-unification path (`infer.rs:336-339`): `List : Fn([Type],Type)` unifies against `Fn([Type], fresh)` and the `Apply` node's type is `Ty::Type` — exactly what the deleted dedicated `Hir::TypeCtor` typing arm (`infer.rs:578-590`) produced. **You must add the signatures in §6.**

### 4c. `(: e (List Int64))` extracts `Ty` and checks → CDZ0203

Rewrite `extract_type_value` (`infer.rs:1119-1156`) to recognize the intrinsic-headed form and delegate to the shared helper:

```rust
fn extract_type_value(node: &TypedNode) -> Option<Ty> {
    match node {
        TypedNode::TypeVal(ty) => Some(ty.clone()),
        // An Apply whose func is a type-builder Intrinsic — extract each arg's Ty, build the compound.
        TypedNode::Apply { func, args } => {
            if let TypedNode::Intrinsic(op) = &func.node {
                let arg_tys: Vec<Ty> =
                    args.iter().filter_map(|a| extract_type_value(&a.node)).collect();
                if arg_tys.len() != args.len() { return None; }
                return op.build_compound_ty(&arg_tys);
            }
            None
        }
        _ => None,
    }
}
```

The `Hir::Annot` arm (`infer.rs:591-629`) is **unchanged**: it still checks the RHS types as `Ty::Type` (603), calls `extract_type_value` (613), and unifies `e`'s type against the extracted target, mapping failure to `Code::AnnotMismatch` = CDZ0203 (619-627). So `(: (list 1 2) (List Int64))` now checks, and `(: (list 1 2) (List Bool))` is CDZ0203.

### 4d. Remove the parametric-annotation UNCODED decline

This decline is **already correct and stays** as a decline for genuinely-non-type RHSs — but confirm it is not reachable on a *valid* compound annotation. With 4c, `extract_type_value` now succeeds for `(List Int64)`/`(Map K V)`/etc., so the `None => Reject::decline("annotation type did not reduce to a type-value")` at `infer.rs:613-616` no longer fires on valid parametric annotations (it previously would if a case slipped through). Leave the decline in place for truly malformed RHSs (e.g. `(: e (List))` under-arity — a decline, never a coded reject; see §10). Do **not** convert it to a coded CDZ.

---

## 5. `parse_type_expr` — **STAYS** (deletion is out of scope for this redirect)

Per JUDGE verdict (e): **no closures design unblocks `parse_type_expr` on its own** — the blocker is structural, not the type-ctor mechanism. `collect_user_types` phase 2 (`resolve.rs:97-117`) needs the payload `Ty` **synchronously at resolve time**, before infer/fold exist for that fragment, to fill `SumDef::set_variants` (`resolve.rs:116`); and bare user-sum names resolve to `Hir::Record` of ctors (`prelude.rs:100`, `resolve.rs:91`), **not** a type-value. Routing payloads through the value pipeline is task #151's own structural work.

**Therefore: keep `parse_type_expr` (`resolve.rs:184-216`) and its sole caller at `resolve.rs:104` exactly as-is.** Sum-decl payloads — including the recursive `(type Tree (Leaf Int64 | Node (Tuple Tree Tree)))` — continue to resolve through `parse_type_expr` + `prelude::sum_ref` against the two-phase forward-declared `Arc<SumDef>` (`resolve.rs:154`, `SumDef::forward` `ty.rs:56`, `set_variants` `ty.rs:62`): `Tree` is forward-declared, so `(Tuple Tree Tree)` resolves `Tree` via `sum_ref` to `Ty::Sum { def: <the in-progress Tree Arc>, args: [] }` while its variants are still unset, and phase 2 fills them. This path is unchanged and must keep passing (see BAR — the recursive-sum test at `tests.rs:436-437` and cdzc.cdz).

The one **required** resolve change is the collision fix. In the bare-name prelude arm (`resolve.rs:711-718`), extend the special-case block (currently `Int64`/`Bool`/`String`/`Bytes`/`Unit` → `TypeVal`) so the six parametric names resolve to their type-builder `Intrinsic`, parallel to the existing scalar dual-role:

```rust
                    match name.as_str() {
                        "Int64" => Hir::TypeVal(crate::ty::Ty::Int),
                        "Bool" => Hir::TypeVal(crate::ty::Ty::Bool),
                        "String" => Hir::TypeVal(crate::ty::Ty::String),
                        "Bytes" => Hir::TypeVal(crate::ty::Ty::Bytes),
                        "Unit" => Hir::TypeVal(crate::ty::Ty::Unit),
                        "List" => Hir::Intrinsic(Intrinsic::TypeList),
                        "Map" => Hir::Intrinsic(Intrinsic::TypeMap),
                        "Set" => Hir::Intrinsic(Intrinsic::TypeSet),
                        "Tuple" => Hir::Intrinsic(Intrinsic::TypeTuple),
                        "Option" => Hir::Intrinsic(Intrinsic::TypeOption),
                        "Result" => Hir::Intrinsic(Intrinsic::TypeResult),
                        _ => node.clone(),
                    }
```

This affects only **bare-name value position** (the head of `(List Int64)`, or a bare `List` value). Member access `(. List push)` / `(. Option Some)` is untouched — it takes the separate `member()` path (`resolve.rs:953-1010`), which reads the prelude **record** directly and never enters this arm. (`Intrinsic` must be in scope in resolve.rs — it already is via the prelude imports; add `use crate::ir::Intrinsic;` if not.)

**Verification obligation (BAR-critical):** grep the corpus + `tests.rs` for a bare `Option`/`Result`/`List`/`Map`/`Set` used as a *value record* (not member-accessed, not applied to a type). Since Option/Result are unqualified (variants used bare as `Some`/`None`/`Ok`/`Err`) and List/Map/Set values are only ever member-accessed, none is expected. If any exists, restrict that name's arm to type-application-head position rather than all bare uses. This must not introduce a new FAIL.

---

## 6. NOMINAL IDENTITY — route through `ty.rs` singletons

- **Value identity** of each constructor is its `Intrinsic` enum variant (`Intrinsic` derives `PartialEq`, `ir.rs:132`), mirroring how the scalar `TypeVal`s (`prelude.rs:58-59`) and sum singletons (`prelude.rs:78-101`) give identity.
- **Constructed-type identity** for `Option`/`Result` routes through the process-global `SumRef` singletons `crate::ty::prelude_option()` / `prelude_result()` inside `build_compound_ty` (§2c) — identical to the spike's `fold.rs:784`/`788`. Two `Option T` types are the same type iff `Arc::ptr_eq` on the def, preserved because `build_compound_ty` always uses the shared singleton.

Add `param_count` (`ir.rs:221-236`) and `signature` (`ir.rs:292-348`) arms. All six are **monomorphic** (`Fn` over concrete `Ty::Type`, no `Ty::Param`), so `param_count` = 0:

```rust
// in param_count, fold into the `=> 0` group:
    Intrinsic::TypeList | Intrinsic::TypeMap | Intrinsic::TypeSet
    | Intrinsic::TypeTuple | Intrinsic::TypeOption | Intrinsic::TypeResult => 0,
```
```rust
// in signature, before `Intrinsic::Heap(h) => h.signature()`:
    Intrinsic::TypeList | Intrinsic::TypeSet | Intrinsic::TypeOption =>
        (vec![Ty::Type], Ty::Type),
    Intrinsic::TypeMap | Intrinsic::TypeResult | Intrinsic::TypeTuple =>
        (vec![Ty::Type, Ty::Type], Ty::Type),
```

This reproduces the deleted `Hir::TypeCtor` typing arm's arities (`infer.rs:584-588`) exactly.

---

## 7. THE ERASURE FENCE STILL HOLDS — no change

`check_erasure_fence` (`fold.rs:74-216`) is unchanged. A bare `Mir::TypeVal` is a direct CDZ0305 leak (`fold.rs:77-79`), and any slot whose solved `Ty` `is_comptime_only` (`ty.rs:377-391`) leaks. A residual, under-reduced type-builder application `Apply(Mir::Intrinsic(TypeList), [Mir::TypeVal(..)])` is caught by the fence's `Apply` recursion (`fold.rs:193-201`) → the `TypeVal` arg hits the bare-`TypeVal` check (77-79) → CDZ0305. A fully-bare unapplied type-builder (`Mir::Intrinsic(TypeList)` with no args reaching runtime) declines through the **pre-existing** bare-`Intrinsic` `select` arm (`select.rs:180`, "a built-in operation value cannot yet cross to run time") — an UNCODED decline, exactly as any other unapplied intrinsic. `is_comptime_only`, `is_transient` (which already admits `Mir::Intrinsic`, `fold.rs:964`), and the `Mir::TypeVal` `select` decline (`select.rs:315`) all stay verbatim.

Best_graft 4 (extend the fence to code the residual under-arity case as CDZ0305) is **OPTIONAL polish** — skip it for this redirect. The existing fence already codes the common case (TypeVal-arg-bearing residual) as CDZ0305, and the bare case declines uncoded at `select.rs:180`. Adding it risks a coded reject on an edge case; §10 discipline forbids that unless proven safe.

---

## 8. UNIT TESTS TO ADD (`tests.rs`, style: `compile_program(&program_v2(body))` → `.component()` / `.diagnostics[0].code.as_deref()`)

Add one test `layer2_parametric_type_annotations`:

```rust
/// Layer 2 first-class parametric types: `(: e (List Int64))` and friends type-check via the
/// type-builder Intrinsics (List/Map/Set/Tuple/Option/Result bound as prelude Intrinsic singletons,
/// their `fold_const` building the compound Ty). A matching annotation compiles; a mismatch is CDZ0203.
#[test]
fn layer2_parametric_type_annotations() {
    // Matching compound annotations compile.
    for body in [
        "(: (list 1 2 3) (List Int64))",
        "(: (set 1 2) (Set Int64))",
        "(: (map (1 2)) (Map Int64 Int64))",
        "(: (tuple 1 true) (Tuple Int64 Bool))",
        "(: (Some 42) (Option Int64))",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_some(),
            "expected {body:?} to compile: {:?}", out.diagnostics);
    }

    // Element-type mismatch inside a compound annotation is CDZ0203.
    for body in [
        "(: (list 1 2) (List Bool))",
        "(: (tuple 1 2) (Tuple Int64 Bool))",
        "(: (Some 42) (Option Bool))",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_none(), "{body:?} should be rejected");
        assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0203"),
            "{body:?} should be CDZ0203");
    }
}
```

Add `layer2_type_ctor_as_value_declines` (proves the bare-Intrinsic path, §7):

```rust
/// A bare, unapplied parametric type constructor cannot cross to runtime — it declines (UNCODED),
/// like any bare intrinsic; and a type-value it builds leaking to runtime is the erasure fence
/// (CDZ0305). Neither is a coded reject on a *valid* program — this is decline/fence discipline.
#[test]
fn layer2_type_ctor_as_value_declines() {
    // A type-value returned from main must hit the erasure fence (CDZ0305), not emit.
    let out = compile_program(&program_v2("(List Int64)"));
    assert!(out.component().is_none(), "a bare type-value cannot cross to runtime");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0305"));

    // A type-builder bound to a local (never crossing to runtime) is fine — it inlines (is_transient).
    let out = compile_program(&program_v2("(let ((l List)) 42)"));
    assert!(out.component().is_some(),
        "a type-builder bound but not run should compile: {:?}", out.diagnostics);
}
```

Keep the existing `layer1_scalar_type_annotations` (`tests.rs:841-861`), `layer1_type_values` (867-872), and the recursive-sum test (`tests.rs:428-437`) passing unchanged.

(If `(List Int64)` as a bare main body reduces to `Mir::TypeVal` and the fence codes CDZ0305 — verify the observed code; if the harness surfaces it as the bare-`Intrinsic` select decline instead because the arg reduced away, adjust the first assertion to accept the decline. The load-bearing claim is *no component + not a false CDZ0203*.)

---

## 9. THE BAR (exact acceptance)

1. `cargo test -p rcdzc` — **green** (all existing tests + the two new tests in §8).
2. Behavior gate — **357 pass / 5 held FAIL, ZERO new FAIL** (diff the FAIL *set*, not the pass count — P/todo/skip drift is noise per MEMORY). Use a FRESH `CADENZA_RUNTIME` wasm.
3. Compound-annotation corpus cases that were **todo → pass** (the `(: e (List T))` / `(Option T)` / `(Map K V)` / `(Set T)` / `(Tuple A B)` annotation cases now reachable because bare `List`/`Map`/`Set`/`Option`/`Result` resolve to the type-builder Intrinsic).
4. `cdzc.cdz` self-host still **stops at "member access on Fresh"** — this redirect must not move that frontier (effects, task #148, is separate). Regenerate `implementation/compiler/cdzc.cdz` via `make` in `implementation/compiler` before judging any parse error there (it is `@generated`; never hand-edit).
5. The recursive-sum path (`(type Tree …)`, `(type IntList (Nil | Cons (Tuple Int64 IntList)))`) still resolves via the retained `parse_type_expr` — no regression.
6. `grep -rn TypeCtor src/` → **zero hits**.

---

## 10. DECLINE-NOT-REJECT DISCIPLINE (anything beyond L2)

- A malformed or beyond-L2 type-construction (wrong arity, an unsupported ctor shape, a curried/partial type-builder application that never reaches full arity, a bare type-builder used as runtime data) must **decline** — `Reject::decline(...)` (UNCODED) or the existing UNCODED `select.rs:180` bare-`Intrinsic` decline. **Never** emit a coded CDZ for a *valid* program.
- The only coded outcomes on this path are the two that already exist and are correct: **CDZ0203** (`Code::AnnotMismatch`, `diag.rs:23`) for a genuine annotation contradiction, and **CDZ0305** (`Code::ComptimeErasure`, `diag.rs:37`) for a comptime-only value crossing the runtime boundary. Do not add new codes.
- Currying/partial type-builder application is an **L3 luxury** — do not build spine-collapse for `Intrinsic` (the existing collapse at `fold.rs:685-701` ignores `Intrinsic`, and that is fine). An under-arity type-builder stays a residual `Apply` and is caught downstream (fence or `select.rs:180` decline). Do not pay surface for it.
- Do not delete `parse_type_expr` and do not attempt `(meta apply)` / sum-name-as-type-value (task #151) — a partial attempt will break recursive sum resolution and fail the BAR.

---

**Files touched:** `ir.rs` (delete `TypeCtorKind` + 3 node variants; add 6 `Intrinsic` arms + `fold_const`/`signature`/`param_count`/`build_compound_ty` logic), `fold.rs` (delete the `TypeCtor` `Apply` arm + guard mentions; widen the gate at 745), `infer.rs` (delete the `Hir::TypeCtor` typing arm + finalize passthrough + guard; rewrite `extract_type_value`), `lower.rs` (delete arm), `select.rs` (delete decline), `layout.rs` (drop guard mentions), `prelude.rs` (replace the TypeCtor inserts), `resolve.rs` (extend the bare-name special-case block; keep `parse_type_expr`), `tests.rs` (add two tests).