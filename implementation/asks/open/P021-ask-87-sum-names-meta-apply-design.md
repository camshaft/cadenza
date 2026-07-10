# Implementation Spec: Revert TypeOption/TypeResult Intrinsics + Build (meta apply) for Sum Types

## Rationale

**OPERATOR PRINCIPLE**: "There should be no difference between a user defining an Option and the compiler defining it."

Commit 7aa0cc5 added six type-constructor intrinsics (TypeList/TypeMap/TypeSet/TypeTuple/TypeOption/TypeResult). The structural intrinsics (List/Map/Set/Tuple) are LEGITIMATE—a user cannot define them, so name-privilege is justified. But TypeOption/TypeResult privilege prelude sums by Intrinsic identity, violating the operator principle: a user `(type Box (a) (Wrap a))` has no equivalent path.

**Core technical constraint**: `Intrinsic` is Copy (ir.rs:112), so TypeOption/TypeResult cannot carry the sum's `SumRef` (Arc<SumDef>, Clone-not-Copy, ty.rs:90). The def must be hard-wired by matching the Copy enum variant. This forces asymmetry: prelude sums are privileged by Intrinsic identity, while user sums resolve to ctor records with no type-application path.

This spec **removes the asymmetry** by: (1) deleting TypeOption/TypeResult intrinsics (the 8-site revert), and (2) building the (meta apply) machinery so that BOTH prelude and user sums resolve to ctor records with a projectable type-constructor.

---

## Scope

**REVERT**: Delete `Intrinsic::TypeOption` and `Intrinsic::TypeResult` from 8 sites. List/Map/Set/Tuple intrinsics **STAY** (structural, not sums).

**BUILD (meta apply)**: Add the reserved (meta ...) namespace projection (resolve.rs:972-978) to return a SumRef-carrying builder that reduces at infer-time to Ty::Sum{def, args}.

**REPRESENTATION CHOICE** (minimal blast radius): Reuse `Hir::Ctor{def: SumRef, index: usize}` as the (meta apply) builder. It already carries SumRef, is Clone, and flows through substitute/alpha_rename as an inert leaf. For the type-builder role, use any ctor from the sum's record (e.g., fields[0]); the `index` is vestigial—only `def` matters for type-application. This avoids adding a new IR node and avoids a sidecar field on Record (which would widen ~6-8 Record arms).

**DEFER to task #157**: Parsing user-sum params from `(type Box (a) ...)` syntax. resolve.rs:154 currently hard-codes `params: vec![]`. This spec's machinery (the meta-apply builder + infer-time reduction) is #157's consumption target. For now, test with **prelude Option/Result ONLY** (their params are already captured, ty.rs:180-189).

---

## Changes

### 1. Revert the TypeOption/TypeResult Intrinsic (8 Sites)

All changes **split grouped arms** to remove TypeOption|TypeResult while keeping TypeList|TypeMap|TypeSet|TypeTuple.

#### ir.rs:174-176 — Delete variant declarations
```rust
// DELETE these lines:
TypeOption,
TypeResult,
```

#### ir.rs:223-224 — Split param_count grouped arm
```rust
// BEFORE:
TypeList | TypeMap | TypeSet | TypeTuple | TypeOption | TypeResult => 0,

// AFTER:
TypeList | TypeMap | TypeSet | TypeTuple => 0,
```

#### ir.rs:346-349 — Split signature grouped arms
```rust
// BEFORE:
Intrinsic::TypeList | TypeSet | TypeOption => (vec![Type], Type),
Intrinsic::TypeMap | TypeResult | TypeTuple => (vec![Type, Type], Type),

// AFTER:
Intrinsic::TypeList | TypeSet => (vec![Type], Type),
Intrinsic::TypeMap | TypeTuple => (vec![Type, Type], Type),
```

#### ir.rs:395-398 — Delete build_compound_ty arms
```rust
// DELETE these lines (ir.rs:395-398):
(Intrinsic::TypeOption, [a]) =>
    Some(Ty::Sum { def: crate::ty::prelude_option(), args: vec![a.clone()] }),
(Intrinsic::TypeResult, [a, e]) =>
    Some(Ty::Sum { def: crate::ty::prelude_result(), args: vec![a.clone(), e.clone()] }),
```

#### ir.rs:450-460 (fold_const) — Split grouped arm
**Find the line** (approximately ir.rs:450-460) where `fold_const` calls `build_compound_ty` for type-constructor intrinsics. Remove TypeOption|TypeResult from that grouped arm. (The survey indicates this is at ir.rs:424-425, but the read showed line 450 region; adapt to actual code.)

```rust
// BEFORE (grouped arm):
Intrinsic::TypeList | TypeMap | TypeSet | TypeTuple | TypeOption | TypeResult => { ... }

// AFTER:
Intrinsic::TypeList | TypeMap | TypeSet | TypeTuple => { ... }
```

#### select.rs:820-821 — Split emit_intrinsic decline grouped arm
```rust
// BEFORE:
Intrinsic::TypeList | TypeMap | TypeSet | TypeTuple | TypeOption | TypeResult => {
    return Err(Reject::decline("type-constructor intrinsic survived fold"))
}

// AFTER:
Intrinsic::TypeList | TypeMap | TypeSet | TypeTuple => {
    return Err(Reject::decline("type-constructor intrinsic survived fold"))
}
```

#### resolve.rs:730-731 — Delete bare-name override arms
```rust
// DELETE these lines (resolve.rs:730-731):
"Option" => Hir::Intrinsic(Intrinsic::TypeOption),
"Result" => Hir::Intrinsic(Intrinsic::TypeResult),
```

After deletion, "Option" and "Result" fall through to the `_ => node.clone()` arm at resolve.rs:732, returning the prelude's `Hir::Record` of ctors (inserted at prelude.rs:103)—the SAME path user sums already take.

**KEEP** the List/Map/Set/Tuple arms (resolve.rs:726-729).

#### prelude.rs:87-88 — Delete dead intrinsic inserts
```rust
// DELETE these lines (prelude.rs:87-88):
p.insert("Option".to_string(), Hir::Intrinsic(Intrinsic::TypeOption));
p.insert("Result".to_string(), Hir::Intrinsic(Intrinsic::TypeResult));
```

These are **already overwritten** by the sum-record loop at prelude.rs:101-110 (the comment at prelude.rs:74-82 admits they are "DEAD"). Deleting them is cleanup.

**KEEP** the List/Map/Set/Tuple inserts (prelude.rs:83-86).

---

### 2. Implement (meta apply) Projection (resolve.rs:972-978)

The `(meta ...)` syntactic arm at resolve.rs:979-981 currently declines. Replace it with a dispatch that projects the sum's type-constructor (any ctor works—return the first, since all ctors carry the same SumRef).

```rust
// resolve.rs:979-981 — REPLACE the decline block with:
if let Node::List(k) = &items[2] {
    if k.first().and_then(name_of) == Some("meta") {
        let meta_key = k.get(1).and_then(name_of).unwrap_or("");
        match meta_key {
            "apply" => {
                // (. SumName (meta apply)) → the sum's type-constructor.
                // Project the FIRST ctor from the sum's record (any ctor works—all carry the same def).
                let operand = &items[1];
                let node = match name_of(operand) {
                    Some(n) if !scope.contains_key(n) => self.prelude.get(n),
                    _ => None,
                };
                if let Some(Hir::Record(fields)) = node {
                    if let Some((_, ctor)) = fields.first() {
                        return ctor.clone(); // Hir::Ctor{def, index} — the SumRef-carrying builder.
                    }
                }
                return Hir::Error(Reject::decline("(meta apply) operand is not a sum name"));
            }
            "t" => {
                // (. SumName (meta t)) → the type-of-types constant (Ty::Type).
                // Orthogonal to (meta apply); simple to add but not load-bearing for the revert.
                return Hir::TypeVal(crate::ty::Ty::Type);
            }
            _ => {
                return Hir::Error(Reject::decline("unknown meta key (only `apply` and `t` implemented)"));
            }
        }
    }
}
```

**Correctness**: `(. Option (meta apply))` returns `Hir::Ctor{def: prelude_option(), index: 0}` (the "Some" ctor). `(. Box (meta apply))` returns `Hir::Ctor{def: user_box_sumref, index: 0}` (the "Wrap" ctor). The **index is irrelevant** to type-application—the builder only uses `def` (the SumRef identity + `def.params.len()` arity). All ctors of the same sum carry the same `def` (Arc clones), so returning any ctor is correct; `fields[0]` is the cheapest.

**Fence**: The (meta apply) value (a Hir::Ctor) is typed as `Fn([Type...], Type)` at infer (infer.rs:233-260 already instantiates sum params with fresh vars). When applied to [TypeVal...] args, it reduces at infer-time (next step) to TypeVal(Ty::Sum{def, args}), which is typed as Ty::Type (comptime-only). If leaked to runtime, the erasure fence catches it.

---

### 3. Infer-Time Type-Value Reduction (infer.rs:1268, extract_type_value)

The existing `extract_type_value` at infer.rs:1268-1309 reduces `Apply(Intrinsic(op), [TypeVal...])` inline during `(: e T)` annotation typing. After the TypeOption/TypeResult revert, this path no longer handles `(: x (Option Int64))` (since Option is not an Intrinsic).

**Add a parallel arm** for `Apply(Ctor, [TypeVal...])` BEFORE the Intrinsic arm (at infer.rs:1273, just after the match on Apply):

```rust
// infer.rs:1273 — Inside extract_type_value, in the Apply arm, BEFORE the Intrinsic check:
TypedNode::Apply { func, args } => {
    // NEW: Sum type-constructor application (e.g., (Option Int64) in an annotation).
    // The func is a Hir::Ctor (projected via (meta apply)) carrying the sum's SumRef.
    if let TypedNode::Ctor { def, .. } = &func.node {
        let type_args: Vec<Ty> = args.iter().filter_map(|a| extract_type_value(&a.node)).collect();
        if type_args.len() == args.len() {
            // Arity check: type_args.len() must equal def.params().len().
            if type_args.len() != def.params().len() {
                return None; // Arity mismatch → fall to infer's normal arity check/decline.
            }
            return Some(Ty::Sum {
                def: def.clone(),
                args: type_args,
            });
        }
    }
    // EXISTING: Intrinsic type-constructor (List/Map/Set/Tuple).
    if let TypedNode::Intrinsic(op) = &func.node {
        // ... existing logic unchanged ...
    }
    None
}
```

**Why here, not fold?** Type-application `(Option Int64)` appears in ANNOTATIONS `(: x (Option Int64))` (infer.rs:720-728). The annotation arm calls `extract_type_value` to reduce the annotation to a `Ty` for unification. If `(Option Int64)` is not reduced at infer-time, it remains `Apply(Record, ...)` or `Apply(Ctor, ...)` unevaluated → later decline at fold/select. Reducing it here ensures `(: x (Option Int64))` unifies correctly.

**Fold-time path unchanged**: The existing `Apply(Ctor, [arg])` arm at fold.rs:762-766 handles runtime ctor application `(Some 42)` → `Mir::Sum{...}`. We do NOT change this arm—it already works. Applying a Ctor to TypeVal args at fold-time is RARE (type-values are extracted at infer); the infer-time path is primary.

---

### 4. Type the Ctor as a Polytype Builder (No Change Needed)

The existing `Hir::Ctor` typing at infer.rs:233-260 already instantiates the sum's params with fresh unification vars:

```rust
// infer.rs:244-246 (existing, NO CHANGE):
let args: Vec<Ty> = def.params().iter().map(|_| supply.fresh()).collect();
let ret = Ty::Sum { def: def.clone(), args: args.clone() };
```

For `Hir::Ctor{def: prelude_option(), index:0}` (Some), this yields `Fn([Var(α)], Sum{prelude_option(), [Var(α)]})` — a polytype. When `(meta apply)` is applied to `[TypeVal(Int64)]`, unification solves `Var(α) = Int64`, and the extract_type_value arm produces `TypeVal(Ty::Sum{prelude_option(), [Int64]})`.

The ctor's **index** (which variant) is irrelevant to type-application — only `def` (the SumRef) and `def.params.len()` matter. The index only matters for heap-value construction (fold.rs:763 `disc: index as u32`).

---

## One-Path Proof: Option and User Box Are Byte-Identical

After the revert + (meta apply) build, **both paths resolve identically**:

| Step | Prelude `(Option Int64)` | User `(Box Int64)` (after #157) |
|------|--------------------------|----------------------------------|
| **Parse** | `Apply(Name("Option"), [TypeVal(Int64)])` | `Apply(Name("Box"), [TypeVal(Int64)])` |
| **Resolve Name** | prelude.get("Option") → `Hir::Record([(\"Some\", Ctor{prelude_option(),0}), (\"None\", Ctor{prelude_option(),1})])` | prelude.get("Box") → `Hir::Record([(\"Wrap\", Ctor{user_box,0})])` |
| **Resolve (meta apply)** | `(. Option (meta apply))` → `Ctor{prelude_option(), 0}` | `(. Box (meta apply))` → `Ctor{user_box, 0}` |
| **Infer Ctor** | `Fn([Var(α)], Sum{prelude_option(),[Var(α)]})` | `Fn([Var(β)], Sum{user_box,[Var(β)]})` |
| **Apply** | `Apply(Ctor{prelude_option(),0}, [TypeVal(Int64)])` | `Apply(Ctor{user_box,0}, [TypeVal(Int64)])` |
| **extract_type_value** | `Ty::Sum{prelude_option(), [Int64]}` | `Ty::Sum{user_box, [Int64]}` |
| **Identity** | `Arc::ptr_eq(def, prelude_option())` | `Arc::ptr_eq(def, user_box_arc)` |

**Asymmetry check**: NONE. The only difference is the SumRef identity (which **should** differ—they are distinct types). The shape, typing, resolution, and reduction logic are byte-identical.

---

## Blast Radius

**8 revert sites** (ir.rs ×5, select.rs ×1, resolve.rs ×1, prelude.rs ×1): Split grouped Intrinsic arms to remove TypeOption|TypeResult; keep List/Map/Set/Tuple.

**1 projection arm** (resolve.rs:979-981): Replace the (meta ...) decline stub with a dispatch that projects fields[0] of the sum's ctor record.

**1 infer arm** (infer.rs:1273): Add Apply(Ctor{def}, [TypeVal...]) case to extract_type_value (before the existing Intrinsic arm).

**Total: ~10 touchpoints**. No new IR nodes, no new exhaustive-match arms outside these specific sites. Much narrower than a new SumType(SumRef) leaf node (~14 Mir arms + ~6 Record arms = ~20).

---

## Unit Tests

Add to the corpus (or a unit test file):

### Test 1: Prelude Sum Type-Application
```scheme
;; (Option Int64) constructs Ty::Sum{def: prelude_option(), args: [Int]}
(: (Some 42) (Option Int64))    ;; → ok
(: (None unit) (Option Int64))  ;; → ok
(: (Some 42) (Option Bool))     ;; → CDZ0203 (type mismatch)
```

### Test 2: User Parametric Sum (Post-#157)
```scheme
;; REQUIRES task #157 to parse params; for now user sums are monomorphic.
(type Box (a) (Wrap a))

;; Type-application via (meta apply):
(: (Wrap 5) (Box Int64))     ;; → ok (after #157)
(: (Wrap 5) (Box Bool))      ;; → CDZ0203 (type mismatch, after #157)

;; Meta-namespace projection:
(. Box (meta apply))         ;; → Ctor{def: box_def, index: 0}, typed Fn([Type], Type)
(. Box (meta t))             ;; → TypeVal(Ty::Type), typed Ty::Type
```

### Test 3: Meta Namespace on Non-Sum Declines
```scheme
(. List (meta t))            ;; → decline "meta projection on non-sum" (List is Intrinsic)
(. 42 (meta apply))          ;; → decline "operand is not a sum name"
```

### Test 4: Erasure Fence
```scheme
(do
  (let ((T (Option Int64)))  ;; T is a type-value, typed Ty::Type
    T))                       ;; → CDZ0305 (type-value leaked to runtime, caught by fence)
```

---

## The Bar: Promotion Criteria

A commit is promotable when:

1. **Behavior gate = 0 FAIL**. Run `make gate` (360 + 5 cases). All must pass (exit 0).
2. **cargo test = green**. All unit/integration tests pass.
3. **cdzc stops at member-access-on-Fresh**. The known blocker (resolve.rs member() on a Fresh var) still declines (expected); no NEW declines introduced by this change.
4. **grep audit**:
   - `grep -rn 'TypeOption\|TypeResult' src/` → ZERO hits (modulo comments).
   - `grep -rn 'TypeList\|TypeMap\|TypeSet\|TypeTuple' src/` → returns structural-intrinsic sites (kept).
5. **Spot-check**: Bare `Option` in a test case resolves to its ctor record (not an Intrinsic). `(. Option Some)` works (projects the ctor). `(. Option (meta apply))` returns a Ctor. `(: x (Option Int64))` types correctly.

---

## Risks and Mitigations

### Risk 1: Corpus Regressions
**Risk**: Any test case that applies Option/Result as type ctors (`(Option Int64)`) will NOW work (was broken before the revert). Any test that relied on the Intrinsic path may behave differently.

**Mitigation**: Run the gate after the changes. New PASSes are expected (Option/Result type-application now works via meta-apply). Annotate any remaining fails as `(needs task-157)` if they require user-sum params.

### Risk 2: User Parametric Sums Still Don't Parse
**Risk**: resolve.rs:154 hard-codes `params: vec![]`—`(type Box (a) (Wrap a))` won't capture `a` until task #157.

**Mitigation**: (1) Test with **prelude Option/Result ONLY** (params already captured, ty.rs:180-189). (2) Annotate user-sum tests `(needs task-157)`. (3) This spec is #157's foundation—machinery built, parser wiring deferred.

### Risk 3: Meta Namespace Collision
**Risk**: The `(meta ...)` namespace was reserved for module metadata (capabilities—resolve.rs:976 comment). Reusing for sum-type metadata might collide.

**Mitigation**: (1) Projection targets are disjoint (module record vs. sum record—dispatch on operand type). (2) Alternative: use `(type-meta ...)` namespace, but `meta` is already carved out syntactically (cheaper to reuse). (3) Document dual use in resolve.rs:979 comment.

### Risk 4: Ctor Overloaded
**Risk**: `Hir::Ctor` now serves two roles—runtime ctor (→heap Sum) AND type-builder (→TypeVal(Sum)). The `index` field is vestigial for the builder role.

**Mitigation**: (1) Roles are disjoint by context: infer-time extract_type_value handles type-application; fold-time handles runtime ctors. (2) If this proves fragile, a follow-up can factor a new `SumType(SumRef)` leaf—but that's the expensive 14-arm path, deferred until proven necessary.

---

## Summary

This spec deletes the TypeOption/TypeResult privileged intrinsics (8 revert sites) and builds the (meta apply) machinery by: (1) replacing the decline stub at resolve.rs:979-981 with a projection that returns the sum's first ctor (a Hir::Ctor carrying SumRef), and (2) adding an extract_type_value arm at infer.rs:1273 to reduce Apply(Ctor{def}, [TypeVal...]) to Ty::Sum{def, args}. Prelude Option/Result and user sums (post-#157) resolve to identical Hir::Record structures and reduce via identical paths. The leaf primitive (Ctor reused as a sum-type builder) carries SumRef (Clone-not-Copy), survives substitute/alpha_rename as an inert leaf, and avoids the Intrinsic-is-Copy problem. Nominal identity (Arc ptr_eq) and erasure fence (is_comptime_only recurses into Sum.args) are already correct. Blast radius: 8 revert sites + 1 projection arm + 1 infer arm = ~10 touchpoints. Tests: prelude Option works immediately; user Box works post-#157. The bar: gate 360/5 + cargo test green + grep audit clean + cdzc stops at expected frontier.