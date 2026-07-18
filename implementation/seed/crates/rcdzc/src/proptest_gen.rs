//! Compiler-directed generators for property tests over COLLECTION types (F1 / approach A —
//! compiler-synthesis; see `implementation/design/DESIGN-property-test-collection-generators-rcdzc.md`).
//!
//! `cdz test` already property-tests a `@test` def with SCALAR parameters: the runner generates each
//! scalar and (for a guest that performs `Test.gen : Unit -> Int64`) drives a seeded int pool with
//! shrinking. A `@test` with a COMPOUND parameter (`List Int64`, …) has no boundary representation, so it
//! could not be property-tested — it declined at the export boundary.
//!
//! This pass closes that gap by SYNTHESIS. For a `@test` whose single parameter is a generatable
//! compound type (a `(List ELEM)`, `(Tuple T…)`, or `(Record (f T)…)` over integer / `Bool` leaves,
//! nested arbitrarily), e.g.
//!
//! ```text
//! (@ test (def (p (: xs (List Int64))) BODY))
//! ```
//!
//! it rewrites the top-level block to
//!
//! ```text
//! (effect Test (op gen (-> Unit Int64)))            ; appended once, if absent
//! (def (p (: xs (List Int64))) BODY)                ; the original — @test STRIPPED (now a plain callee)
//! (@ test (def (p-gen)                              ; the synthesized nullary wrapper, @test-marked
//!   (host (Test)
//!     (let ((g0 ((. Test gen)))) (let ((g1 …)) (let ((g2 …))
//!       (p (list g0 g1 g2))))))))
//! ```
//!
//! The wrapper builds the argument by a recursive `<gen:T>` derivation over the parameter type: a scalar
//! consumes one `Test.gen` int (`Bool` = `(= (% gen 2) 0)`, the parity), a `(List ELEM)` builds a
//! VARIABLE-length list (`0..=G1_LIST_LEN`, a gen'd count picking a prefix — so the empty + short lists
//! are exercised), a `(Tuple T…)` builds `(tuple <gen:T> …)`. Every `Test.gen` is hoisted into a `let`
//! (an inlined one under a constructor is not seen within the `host` scope). The existing gen-driven
//! runner detects the wrapper (it pulls `Test.gen` ints), runs `--trials` trials, and shrinks over the
//! int pool — no ABI change, no runner change. Increments so far cover G1 `List<Int>`, G2 `List<Bool>` +
//! element recursion, G3 `Tuple` + nesting, G4 `Record`, G5 user `sum` (bounded — a recursive sum
//! declines), G6 `Set`/`Map`, G7 variable-length lists, G8 multi-parameter, and G9 `Float32`/`Float64`
//! leaves (integer-valued via `float-of-int`); still to come is a `Char` leaf and a lone single-form file
//! (which currently needs a `do`-block root). The pass fires for `@exhaustive` as well as `@test` (and the
//! stacked `@exhaustive @test`): a compound-param `@exhaustive` gains a wrapper carrying the `@exhaustive`
//! marker, so the runner declines it cleanly (a collection domain is unbounded) instead of the compound
//! param aborting the whole file at the export boundary.
//!
//! Runs at load BEFORE `strip_annotations`/`scan_top_level`, so the synthesized `(effect …)` and
//! `(@ test (def …))` flow through the ordinary effect-synthesis + test-hoist + resolve/infer/lower with
//! NO name special-casing (the `no-keys-outside-the-prelude` rule): everything is ordinary AST the reader
//! could have produced.

use crate::ast::{Arenas, Leaf, StructId};
use crate::prelude::{push_atom, push_list};

/// Append a bare-`Name` atom occurrence — the module's node-builder shorthand (the `Builder::name`
/// convenience is on the builder, not `&mut Arenas`; this is its `Arenas`-level twin over `push_atom`).
fn name(ast: &mut Arenas, n: &str) -> StructId {
    push_atom(ast, Leaf::Name(n.to_string()))
}

/// The fixed list length the G1 wrapper generates. Small — enough to exercise a non-trivial list while
/// keeping the synthesized `let`-chain short. Variable length (a `Test.gen`-derived, bounded count) is a
/// later increment.
const G1_LIST_LEN: usize = 3;

/// Rewrite the top-level block so every `@test` whose def takes a single `(List <Int>)` parameter gains a
/// synthesized nullary generator wrapper (and the original loses its `@test` marker). A no-op for a
/// program with no such test. Idempotent in practice (it only fires on a `(@ test (def …))` with a
/// compound param, and after the rewrite that def is no longer `@test`-marked).
pub fn synthesize(ast: &mut Arenas) {
    // The synthesis needs a top-level ITEM LIST to append the wrapper + effect to. A multi-form test file
    // has a `(do …)` root (the common case). A LONE single-form file — one `@test def` with nothing else —
    // parses as the bare `(@ test (def …))` AS the root (no enclosing `do`); treat it as a one-item list so
    // a single-test file is handled too, rebuilding a `(do …)` root below. A `(module …)` root is left
    // untouched (its own item list; not the `cdz test` file shape G1 targets).
    let root = ast.root;
    let items: Vec<StructId> = if let Some(do_items) = ast.as_form(root, "do").map(<[_]>::to_vec) {
        do_items
    } else if ast.as_form(root, "@").is_some() {
        // A bare annotated def at the root — a lone single-form test file. Its one item is the root itself.
        vec![root]
    } else {
        return;
    };

    // NOTE: the `@test`-stacked `@ensures` postcondition rewrite USED to live here (a TESTED-tier pre-pass
    // `rewrite_ensures_stacked_tests`). It moved to v-verification's `verify_enforce::enforce` pass, which now
    // owns `@ensures` enforcement UNIVERSALLY — bare AND `@test`/`@exhaustive`-stacked — rewriting a def body
    // to `(let ((it BODY)) (if Q it (trap)))` BEFORE this pass runs (load-sequence order: verify_enforce →
    // proptest_gen). So a `@test @ensures` def arrives here already postcondition-checked; the ordinary `@test`
    // machinery below just runs it over generated inputs (scalar via the boundary-arg route, compound via the
    // synthesized `-gen` wrapper). One owner, no double-injection (which crashed with expression-nests-too-
    // deeply when both passes rewrote the same body). This pass NEVER touches an `@ensures` node now.

    // If the program declares an effect named `Test` WITHOUT a `gen` op, this pass cannot proceed: the
    // wrapper needs `Test.gen`, appending its own `(effect Test …)` would collide with the existing name,
    // and the existing one has no `gen` to reuse. Bail out (the compound-param `@test` then declines at
    // the boundary as before) rather than emit a wrapper calling a non-existent `Test.gen` (PR #406).
    if test_declared_without_gen(ast, &items) {
        return;
    }

    // Find each `(@ test (def SIG BODY))` item whose SIG has exactly one `(: name (List ELEM))` param
    // with ELEM an integer type. Record (item index, def-name occ text, param count) to synthesize for.
    let mut plans: Vec<TestPlan> = Vec::new();
    for (idx, &item) in items.iter().enumerate() {
        if let Some(plan) = plan_for_item(ast, idx, item, &items) {
            plans.push(plan);
        }
    }
    if plans.is_empty() {
        return;
    }

    // Neutralize each planned `(@ test (def …))` IN PLACE: rewrite the annotation node to BE its inner
    // `(def …)` (adopt the def's children). This matters because `strip_annotations` (which runs after
    // this pass) scans EVERY arena node, not just root-reachable ones — so merely dropping the annotation
    // from the root's child list would leave the orphaned `(@ test …)` node behind, and `strip_annotations`
    // would still record the original compound-param def as a test (→ the boundary decline we are avoiding).
    // Rewriting in place makes the original a plain `(def …)` everywhere: no longer a test, just the
    // wrapper's callee. (Mirrors `strip_annotations`'s own in-place unwrap.)
    for plan in &plans {
        if let crate::ast::Struct::List(inner_children) = ast.get(plan.inner_def).clone() {
            let item = items[plan.item_idx];
            ast.structure[item.0 as usize] = crate::ast::Struct::List(inner_children.clone());
            // A stacked `@exhaustive @test` leaves a MIDDLE `(@ test (def…))` node in the arena; rewrite it
            // to the def too, so `strip_annotations`'s full-arena scan does not re-record the compound def
            // as a test (which would revive the boundary decline).
            for &mid in &plan.nested_anns {
                ast.structure[mid.0 as usize] = crate::ast::Struct::List(inner_children.clone());
            }
        }
    }
    // Build the new item list: the (now-unwrapped) original items, then one wrapper per plan, then a
    // single `(effect Test …)` if the program does not already declare one. Appending keeps ids stable.
    let mut new_items: Vec<StructId> = items.clone();
    for plan in &plans {
        let wrapper = build_wrapper(ast, plan);
        new_items.push(wrapper);
    }
    // Ensure the `Test` effect (with `gen`) is declared exactly once.
    if !declares_test_gen(ast, &items) {
        let eff = build_test_effect(ast);
        new_items.push(eff);
    }

    // Replace the root `(do …)` children with the rewritten list (head "do" + new items).
    let do_head = name(ast, "do");
    let mut do_children = Vec::with_capacity(new_items.len() + 1);
    do_children.push(do_head);
    do_children.extend(new_items);
    let new_root = push_list(ast, do_children);
    ast.root = new_root;
}

/// A `@test` def that needs a generator wrapper: its position in the top-level list, the inner `(def …)`
/// node (the `@test` unwrapped), the def name (the wrapper calls `(NAME arg…)`), and the `GenTy` for EACH
/// parameter (the wrapper builds + passes one `<gen:T>` per param, in order).
struct TestPlan {
    item_idx: usize,
    inner_def: StructId,
    /// The def's name as text (e.g. `"p"`) — the wrapper calls `(p <gen-arg>…)`.
    def_name: String,
    /// The synthesized wrapper's name (`"<def_name>-gen"`).
    wrapper_name: String,
    /// The generatable type of EACH parameter, in signature order — one `<gen:T>` per param.
    gen_tys: Vec<GenTy>,
    /// Whether the source annotation was `@exhaustive` (vs `@test`). A compound-param `@exhaustive` cannot
    /// be exhaustively enumerated (a collection domain is unbounded), so the synthesized wrapper keeps the
    /// `@exhaustive` marker and the `cdz test` runner declines it cleanly — rather than the whole file
    /// aborting at the compound param's export boundary (the pre-fix behavior).
    exhaustive: bool,
    /// NESTED annotation nodes to ALSO neutralize, when the source STACKED annotations above the def —
    /// `@exhaustive @test` (`(@ exhaustive (@ test (def…)))`) or a verification wrapper `@test @requires`/
    /// `@test @ensures` (`(@ test (@ (requires Q) (def…)))`). Each middle `(@ … (def…))` node must be
    /// rewritten to the def too: `strip_annotations` scans EVERY arena node (not just root-reachable ones),
    /// so a left-intact middle node would re-record the original compound-param def as a test → the boundary
    /// decline this pass avoids. Empty for a single (unstacked) annotation. In peel order (outermost-first).
    nested_anns: Vec<StructId>,
}

/// A type this pass can GENERATE, and the recursive shape of its `<gen:T>` expression (each built from
/// one or more `Test.gen` ints). This is the compiler-directed "Arbitrary-like" derivation over the type
/// structure: a scalar consumes one gen int; a `List`/`Tuple` recurses into its element/slot types. A
/// type outside this set (`Char`, a bare/unresolved type) is not (yet) generatable — `classify_ty` returns
/// `None`, so the `@test` declines at the boundary as before.
#[derive(Clone)]
pub enum GenTy {
    /// An integer type (`Int8`…`UInt64`): `<gen>` = `((. Test gen))` (the int at the element width).
    Int,
    /// `Bool`: `<gen>` = `(= (% ((. Test gen)) 2) 0)` (the gen int's parity → a ~50/50 boolean).
    Bool,
    /// A float type (`Float32`/`Float64`), carrying its width: `<gen>` = `((. FloatWIDTH of-int) <gen-int>)`
    /// — an integer-valued float from a fresh `Test.gen` int (the TOTAL `float-of-int` conversion, realized
    /// in both backends). A LONE float parameter already crosses the boundary (the runner generates it), so
    /// this variant only matters NESTED under a `List`/`Tuple`/… where no boundary representation exists.
    Float(u32),
    /// `(List ELEM)`: `<gen>` = a fixed-length `(list <gen:ELEM> …)` of `G1_LIST_LEN` elements.
    List(Box<GenTy>),
    /// `(Tuple T…)`: `<gen>` = `(tuple <gen:T> …)`, one generated value per slot.
    Tuple(Vec<GenTy>),
    /// `(Record (f T)…)`: `<gen>` = `(record (f <gen:T>) …)`, one generated value per named field.
    Record(Vec<(String, GenTy)>),
    /// A user SUM `(type NAME (V PAYLOAD?)…)` named by a bare type name: `<gen>` picks a variant by
    /// `Test.gen % k` and constructs `((. NAME V) <gen:PAYLOAD>)` (a nullary variant is just `(. NAME V)`).
    /// Carries the sum's NAME and each variant's `(ctor-name, optional payload GenTy)`.
    Sum {
        #[allow(dead_code)] // read by the cdz-test counterexample renderer, not within this crate
        type_name: String,
        variants: Vec<(String, Option<GenTy>)>,
    },
    /// `(Set ELEM)`: `<gen>` = `(Set.of (list <gen:ELEM> …))` — build a fixed-length list then dedup into
    /// a set (a collision just yields a smaller set, which is fine).
    Set(Box<GenTy>),
    /// `(Map K V)`: `<gen>` = a fold of `Map.insert` over `G1_LIST_LEN` generated key/value pairs, seeded
    /// from `Map.empty` (a repeated key is last-write-wins, yielding a smaller map — fine).
    Map(Box<GenTy>, Box<GenTy>),
}

/// Recognize `(@ test (def (NAME (: PARAM (List ELEM))) BODY))` with ELEM an integer type; return its
/// plan, or `None` if the item is not such a test.
fn plan_for_item(
    ast: &Arenas,
    item_idx: usize,
    item: StructId,
    items: &[StructId],
) -> Option<TestPlan> {
    // `(@ NAME INNER)` where NAME is `test` or `exhaustive` — the two annotations that mark a property
    // test this pass synthesizes a generator wrapper for. `@exhaustive` is included so a COMPOUND-param
    // `@exhaustive` def gets a wrapper too (else its compound param declines at the export boundary,
    // aborting the whole file); the wrapper carries the `@exhaustive` marker forward so the runner reports
    // it (a compound domain is unbounded → the runner declines it as not-exhaustively-enumerable).
    let ann = ast.as_form(item, "@")?;
    let (&name_occ, &inner) = (ann.first()?, ann.get(1)?);
    let ann_name = ast.as_name(name_occ)?;
    if ann_name != "test" && ann_name != "exhaustive" {
        return None;
    }
    let exhaustive = ann_name == "exhaustive";
    // The inner may itself be one or more STACKED annotations before the `(def …)`:
    //   • `@exhaustive @test def` → `(@ exhaustive (@ test (def…)))` — a nested `@test`;
    //   • `@test @requires(Q) def` / `@test @ensures(Q) def` → `(@ test (@ (requires Q) (def…)))` — a
    //     verification wrapper whose inner-`@` head is a call-style `(requires Q)`/`(ensures Q)` LIST, not a
    //     bare name. verify_enforce runs BEFORE this pass and rewrites such a def's BODY in place but LEAVES
    //     the `(@ (requires|ensures …) …)` wrapper (so strip_annotations still records the predicate), so a
    //     COMPOUND-param def under a @requires/@ensures wrapper must be peeled here too or its `-gen` wrapper
    //     is never synthesized (→ the compound param declines at the boundary).
    // Peel every such layer to reach the `(def …)`, remembering each MIDDLE node so `synthesize` neutralizes
    // it (else strip_annotations re-records the compound def as a test). A single annotation is the common
    // case (`inner` is already the def, no peel).
    let mut inner = inner;
    let mut nested_anns: Vec<StructId> = Vec::new();
    while let Some(nested) = ast.as_form(inner, "@") {
        let &head = nested.first()?;
        // A peelable layer's head is `test`/`exhaustive` (bare name) OR a call-style `(requires …)`/
        // `(ensures …)` application. Anything else (already a `(def …)`, or an unknown annotation) stops.
        let peelable = ast.as_name(head) == Some("test")
            || ast.as_name(head) == Some("exhaustive")
            || ast.as_form(head, "requires").is_some()
            || ast.as_form(head, "ensures").is_some();
        if !peelable {
            break;
        }
        nested_anns.push(inner);
        inner = *nested.get(1)?;
    }
    // INNER must be `(def SIG BODY…)`.
    let def_tail = ast.as_form(inner, "def")?;
    let &sig = def_tail.first()?;
    // SIG is `(NAME PARAM…)` — a list whose head is the def name and whose tail are the parameters.
    let sig_items = match ast.get(sig) {
        crate::ast::Struct::List(items) => items.as_slice(),
        _ => return None,
    };
    let (&name_head, params) = sig_items.split_first()?;
    let def_name = ast.as_name(name_head)?.to_string();
    // At least one parameter (a nullary test needs no generation). Classify EVERY parameter; if ANY is
    // not generatable, decline the whole test (the boundary declines it as before). Then synthesize a
    // wrapper only if AT LEAST ONE param is COMPOUND — an all-scalar signature is handled by the existing
    // boundary-arg route (the runner generates each scalar + passes it as `--arg`, no wrapper needed).
    if params.is_empty() {
        return None;
    }
    let mut gen_tys = Vec::with_capacity(params.len());
    let mut any_compound = false;
    for &p in params {
        let ann_param = ast.as_form(p, ":")?; // `(: name TYPE)`
        let &ty = ann_param.get(1)?;
        let gt = classify_ty(ast, ty, items)?;
        // A SCALAR param (`Int`/`Bool`/`Float`) has a component-boundary representation, so the runner
        // generates it directly (the boundary-arg route) — no wrapper needed. Only a COMPOUND param
        // (a collection/tuple/record/sum, which has no boundary form) forces the synthesized wrapper.
        if !matches!(gt, GenTy::Int | GenTy::Bool | GenTy::Float(_)) {
            any_compound = true;
        }
        gen_tys.push(gt);
    }
    if !any_compound {
        return None; // all-scalar signature — the boundary-arg route handles it; no wrapper
    }
    Some(TestPlan {
        item_idx,
        inner_def: inner,
        // Suffix `-gen`: a hyphen-delimited segment that begins with a letter, so the wrapper name is a
        // valid component extern name (an extern name's `-`-separated segments must each start with a
        // letter — a `$` or a digit-led segment fails boundary-name validation). The wrapper is what
        // `cdz test` reports, so the name stays readable (`p` → `p-gen`).
        wrapper_name: format!("{def_name}-gen"),
        def_name,
        gen_tys,
        exhaustive,
        nested_anns,
    })
}

/// Recursively classify a parameter TYPE occurrence into the [`GenTy`] whose `<gen:T>` this pass builds,
/// or `None` if it is not (yet) generatable. Handles integer scalars, `Bool`, `(List ELEM)`, `(Tuple T…)`,
/// `(Record (f T)…)`, `Float32`/`Float64`, and a bare-name USER SUM (resolved against the program's
/// `(type …)` declarations in `items`). A `Char` or an unresolvable name is `None` — the caller then declines
/// the synthesis. `items` is the top-level form list, so a bare type name can find its declaration.
fn classify_ty(ast: &Arenas, ty: StructId, items: &[StructId]) -> Option<GenTy> {
    classify_ty_at(ast, ty, items, 0)
}

/// The `GenTy` for the parameter of a synthesized `-gen` wrapper — the EXACT generator shape the wrapper
/// draws, so a caller can decode a shrunk `Test.gen` int pool back into the concrete value (the `cdz test`
/// counterexample renderer). `proptest_gen` leaves the ORIGINAL def in place beside the wrapper (name =
/// `<wrapper-without-"-gen">`, its `@test` stripped so it is a plain callee) with its single compound
/// parameter's TYPE-EXPRESSION intact, so we classify that node the same way the wrapper's generator was
/// built from it. `None` if the name is not a `-gen` wrapper, no such sibling def exists, it has no
/// parameter, or the parameter's type is not generatable. Shares `classify_ty`, so a `Sum`/nested shape is
/// covered identically to the wrapper (no separate decode vocabulary to drift out of sync).
pub fn gen_ty_of_wrapper_param(db: &crate::db::Db, wrapper_name: &str) -> Option<GenTy> {
    let orig = wrapper_name.strip_suffix("-gen")?;
    let def = db.defs.iter().position(|d| d.name == orig)?;
    let &param = db.defs[def].params.first()?;
    // The param is a bare name (inference-typed — not generatable here) or an annotated `(: name TYPE)`;
    // the generatable shape lives in the TYPE node (the annotation's second child).
    let ty_node = *db.ast.as_form(param, ":")?.get(1)?;
    // Top-level items — the same list `synthesize`/`classify_sum` scan for a user `(type NAME …)` decl (so a
    // sum payload resolves). A `(do …)` root's children, or a lone bare-annotated root as a one-item list.
    let root = db.ast.root;
    let items: Vec<StructId> = if let Some(do_items) = db.ast.as_form(root, "do").map(<[_]>::to_vec)
    {
        do_items
    } else {
        vec![root]
    };
    classify_ty(&db.ast, ty_node, &items)
}

/// The type-nesting depth beyond which classification declines. Bounds the recursion so a RECURSIVE sum
/// (`type Tree = Leaf Int64 | Node (Tuple Tree Tree)`) — whose generator would be unbounded / infinite —
/// declines rather than recursing forever (a stack overflow). Also caps pathological deep nesting. A
/// generatable value's type tree is shallow in practice, so this never rejects a real finite shape.
const MAX_GEN_DEPTH: usize = 8;

/// The depth-tracked worker for [`classify_ty`]. `depth` counts type-constructor / sum nesting; past
/// [`MAX_GEN_DEPTH`] it declines (→ the recursive-sum guard).
fn classify_ty_at(ast: &Arenas, ty: StructId, items: &[StructId], depth: usize) -> Option<GenTy> {
    if depth > MAX_GEN_DEPTH {
        return None;
    }
    // A bare NAME type — a scalar (`Int64`/`Bool`/…) or a user sum named by its declaration.
    if let Some(n) = ast.as_name(ty) {
        return match n {
            "Int8" | "Int16" | "Int32" | "Int64" | "UInt8" | "UInt16" | "UInt32" | "UInt64" => {
                Some(GenTy::Int)
            }
            "Bool" => Some(GenTy::Bool),
            "Float32" => Some(GenTy::Float(32)),
            "Float64" => Some(GenTy::Float(64)),
            // A bare name that is neither a scalar nor `Bool` MAY be a user `(type NAME …)` sum.
            other => classify_sum(ast, other, items, depth),
        };
    }
    // `(List ELEM)` — recurse into the element type.
    if let Some(list_tail) = ast.as_form(ty, "List") {
        let &elem = list_tail.first()?;
        return Some(GenTy::List(Box::new(classify_ty_at(
            ast,
            elem,
            items,
            depth + 1,
        )?)));
    }
    // `(Tuple T…)` — recurse into each slot; every slot must be generatable.
    if let Some(tup_tail) = ast.as_form(ty, "Tuple") {
        if tup_tail.is_empty() {
            return None; // a zero-slot tuple — nothing to generate; not modeled
        }
        let mut slots = Vec::with_capacity(tup_tail.len());
        for &slot in tup_tail {
            slots.push(classify_ty_at(ast, slot, items, depth + 1)?);
        }
        return Some(GenTy::Tuple(slots));
    }
    // `(Record (f T)…)` — each field is `(FIELD-NAME TYPE)`; recurse into each field's type. Every field
    // must be generatable; a zero-field record is not modeled.
    if let Some(rec_tail) = ast.as_form(ty, "Record") {
        if rec_tail.is_empty() {
            return None;
        }
        let mut fields = Vec::with_capacity(rec_tail.len());
        for &field in rec_tail {
            // `field` = `(NAME TYPE)` — a two-element list. Bind its children as `field_pair` (NOT `items`,
            // which would SHADOW the top-level `items` param this arm's recursion must keep passing — the
            // shadow is scoped to the arm so behavior was already correct, but the reused name reads as a
            // bug, PR #419; renamed for clarity).
            let field_pair = match ast.get(field) {
                crate::ast::Struct::List(field_pair) if field_pair.len() == 2 => field_pair,
                _ => return None,
            };
            let fname = ast.as_name(field_pair[0])?.to_string();
            let fty = classify_ty_at(ast, field_pair[1], items, depth + 1)?;
            fields.push((fname, fty));
        }
        return Some(GenTy::Record(fields));
    }
    // `(Set ELEM)` — recurse into the element type.
    if let Some(set_tail) = ast.as_form(ty, "Set") {
        let &elem = set_tail.first()?;
        return Some(GenTy::Set(Box::new(classify_ty_at(
            ast,
            elem,
            items,
            depth + 1,
        )?)));
    }
    // `(Map K V)` — recurse into the key and value types (both must be generatable).
    if let Some(map_tail) = ast.as_form(ty, "Map") {
        let (&kty, &vty) = (map_tail.first()?, map_tail.get(1)?);
        let k = classify_ty_at(ast, kty, items, depth + 1)?;
        let v = classify_ty_at(ast, vty, items, depth + 1)?;
        return Some(GenTy::Map(Box::new(k), Box::new(v)));
    }
    None
}

/// Resolve a bare type name to a user SUM `(type NAME (V PAYLOAD?)…)` declared in the top-level `items`,
/// classifying it into a [`GenTy::Sum`] — or `None` if no such declaration exists, it has no variants, or
/// any variant's payload is not generatable. A variant is `(VNAME PAYLOAD)` (one payload type) or
/// `(VNAME)` (nullary); a multi-field payload is written as a single `(Tuple …)`/`(Record …)`, so exactly
/// zero or one payload occurrence per variant.
/// The `(type NAME variant…)` tail of `item`, seeing through any annotation wrapper — `item` may be a bare
/// `(type …)` or an annotated `(@ ANN (type …))` (e.g. a type-level `@invariant`, whose `(@ (invariant Q)
/// (type …))` wrapper is left in place by strip_annotations). Peels annotation layers until it reaches a
/// `(type …)` form, returning its tail (`[NAME, variant…]`); `None` if `item` is not a (possibly annotated)
/// type declaration. Mirrors the `plan_for_item` def-annotation peel.
fn type_decl_form(ast: &Arenas, item: StructId) -> Option<&[StructId]> {
    let mut node = item;
    // Peel annotation wrappers `(@ ANN INNER)` — bounded by arena size (each step descends to a child).
    for _ in 0..=MAX_GEN_DEPTH {
        if let Some(tail) = ast.as_form(node, "type") {
            return Some(tail);
        }
        let ann = ast.as_form(node, "@")?;
        node = *ann.get(1)?;
    }
    None
}

fn classify_sum(ast: &Arenas, type_name: &str, items: &[StructId], depth: usize) -> Option<GenTy> {
    // Find `(type NAME variant…)` with a matching NAME — SEEING THROUGH any annotation wrapper. A type
    // declaration may be bare `(type NAME …)` OR annotated `(@ (invariant …) (type NAME …))` (a type-level
    // `@invariant` records a refinement over the value binder `it`; verify_enforce/strip_annotations leave
    // the `(@ …)` wrapper in place). `type_decl_form` peels the wrapper so an `@invariant`-refined type is
    // still recognized as generatable (its underlying variants), not declined as an unknown type.
    let decl_tail = items.iter().find_map(|&it| {
        let tail = type_decl_form(ast, it)?;
        (ast.as_name(*tail.first()?) == Some(type_name)).then_some(tail)
    })?;
    let variant_forms = decl_tail.get(1..).filter(|v| !v.is_empty())?;
    let mut variants = Vec::with_capacity(variant_forms.len());
    for &vf in variant_forms {
        // A variant is a list `(VNAME PAYLOAD?)`.
        let vitems = match ast.get(vf) {
            crate::ast::Struct::List(v) if !v.is_empty() => v.as_slice(),
            _ => return None,
        };
        let vname = ast.as_name(vitems[0])?.to_string();
        let payload = match vitems.get(1) {
            None => None, // nullary variant
            // depth+1 so a RECURSIVE sum (a payload naming the sum itself, directly or through a
            // List/Tuple/Record) exceeds MAX_GEN_DEPTH and declines rather than recursing forever.
            Some(&pty) => Some(classify_ty_at(ast, pty, items, depth + 1)?),
        };
        // A variant with more than one payload occurrence is not the modeled shape (payloads are a single
        // type — a tuple/record for several fields).
        if vitems.len() > 2 {
            return None;
        }
        variants.push((vname, payload));
    }
    Some(GenTy::Sum {
        type_name: type_name.to_string(),
        variants,
    })
}

/// Whether the program already declares an effect named `Test` carrying a `gen` operation — so the pass
/// does not append a second, colliding declaration. A shallow check over the top-level items.
fn declares_test_gen(ast: &Arenas, items: &[StructId]) -> bool {
    for &item in items {
        if let Some(eff) = ast.as_form(item, "effect")
            && eff.first().and_then(|&n| ast.as_name(n)) == Some("Test")
        {
            // The op children follow the effect name: `(effect Test (op gen …) (op fail …) …)`. Reuse the
            // existing `Test` ONLY if it actually declares a `gen` op — a `Test` effect that declares only
            // `fail` (or anything but `gen`) does NOT provide `Test.gen`, and treating it as if it did
            // would make the wrapper call a non-existent op (Copilot PR #406). Check the op names, not
            // just the effect name.
            return eff[1..]
                .iter()
                .filter_map(|&op| ast.as_form(op, "op"))
                .any(|op_tail| op_tail.first().and_then(|&n| ast.as_name(n)) == Some("gen"));
        }
    }
    false
}

/// Whether the program declares an effect named `Test` that does NOT carry a `gen` op — the case where
/// this pass CANNOT proceed: the wrapper needs `Test.gen`, but appending its own `(effect Test …)` would
/// collide with the existing `Test` name, and the existing one has no `gen` to reuse. Synthesis bails
/// out for such a program (the compound-param `@test` then declines at the boundary as before, rather
/// than emitting a wrapper that calls a non-existent `Test.gen`).
fn test_declared_without_gen(ast: &Arenas, items: &[StructId]) -> bool {
    for &item in items {
        if let Some(eff) = ast.as_form(item, "effect")
            && eff.first().and_then(|&n| ast.as_name(n)) == Some("Test")
        {
            let has_gen = eff[1..]
                .iter()
                .filter_map(|&op| ast.as_form(op, "op"))
                .any(|op_tail| op_tail.first().and_then(|&n| ast.as_name(n)) == Some("gen"));
            return !has_gen;
        }
    }
    false
}

/// Build `(effect Test (op gen (-> Unit Int64)))`.
fn build_test_effect(ast: &mut Arenas) -> StructId {
    let arrow = {
        let head = name(ast, "->");
        let unit = name(ast, "Unit");
        let i64 = name(ast, "Int64");
        push_list(ast, vec![head, unit, i64])
    };
    let op = {
        let head = name(ast, "op");
        let gen_nm = name(ast, "gen");
        push_list(ast, vec![head, gen_nm, arrow])
    };
    let head = name(ast, "effect");
    let test = name(ast, "Test");
    push_list(ast, vec![head, test, op])
}

/// Build the `@test`-marked nullary wrapper for a plan:
/// `(@ test (def (NAME-gen) (host (Test) (NAME <gen:ParamType>))))`, where `<gen:ParamType>` is the
/// recursively-built generator expression for the parameter's type. Every `Test.gen` performance is a
/// fresh `((. Test gen))`, so the runner's seeded int pool drives (and shrinks) the whole generated value.
fn build_wrapper(ast: &mut Arenas, plan: &TestPlan) -> StructId {
    // Build `<gen:ParamType>`, HOISTING every `Test.gen` performance into its own `let` binding: a
    // `Test.gen` inlined directly inside a compound constructor argument (`(tuple (Test.gen) …)`) is not
    // seen as within the enclosing `(host (Test) …)` scope and is rejected ("no enclosing handler"),
    // whereas a `let`-bound one is fine. So each leaf becomes `gk`, bound to its gen expression, and the
    // constructors reference the bound names.
    let mut binds: Vec<(StructId, StructId)> = Vec::new();
    // Build one `<gen:T>` per parameter, in signature order (each hoists its own `Test.gen`s into `binds`).
    let gen_args: Vec<StructId> = plan
        .gen_tys
        .iter()
        .map(|gt| build_gen(ast, gt, &mut binds))
        .collect();
    // `(NAME <gen-arg>…)` — call the original test with the generated arguments.
    let call = {
        let callee = name(ast, &plan.def_name);
        let mut children = vec![callee];
        children.extend(gen_args);
        push_list(ast, children)
    };
    // Wrap the call in nested `let`s (innermost = the call), one per hoisted gen binding, in REVERSE so
    // the first-generated binding is the OUTERMOST `let` (evaluated first → pulls the first pool int).
    let mut inner = call;
    for &(var, expr) in binds.iter().rev() {
        let binder = push_list(ast, vec![var, expr]);
        let binds_list = push_list(ast, vec![binder]);
        let let_head = name(ast, "let");
        inner = push_list(ast, vec![let_head, binds_list, inner]);
    }
    // `(host (Test) <let-chain>)` — delegate the Test effect to the boundary.
    let host = {
        let head = name(ast, "host");
        let test = name(ast, "Test");
        let effs = push_list(ast, vec![test]);
        push_list(ast, vec![head, effs, inner])
    };
    // `(def (NAME-gen) host)` — nullary signature (a name-only sig list).
    let def = {
        let head = name(ast, "def");
        let sig = {
            let nm = name(ast, &plan.wrapper_name);
            push_list(ast, vec![nm])
        };
        push_list(ast, vec![head, sig, host])
    };
    // `(@ test def)` / `(@ exhaustive def)` — mark the wrapper as the test to hoist, carrying the SOURCE
    // annotation forward so an `@exhaustive` def's wrapper is still seen as exhaustive by the runner.
    let at = name(ast, "@");
    let ann = name(
        ast,
        if plan.exhaustive {
            "exhaustive"
        } else {
            "test"
        },
    );
    push_list(ast, vec![at, ann, def])
}

/// Recursively build the `<gen:T>` VALUE expression for a generatable type — the Arbitrary-like
/// derivation — while HOISTING every `Test.gen` performance into `binds` (a `(var, gen-expr)` list the
/// caller wraps in `let`s). A scalar consumes one gen int (a hoisted `let`, returning the bound var); a
/// `(List ELEM)` builds `(list …)` of `G1_LIST_LEN` recursively-generated elements; a `(Tuple T…)` builds
/// `(tuple …)`. Hoisting is required because an inlined `Test.gen` inside a constructor argument is not
/// seen within the enclosing `host` scope (rejected), but a `let`-bound one is.
fn build_gen(ast: &mut Arenas, ty: &GenTy, binds: &mut Vec<(StructId, StructId)>) -> StructId {
    match ty {
        // A scalar: hoist `gk = ((. Test gen))` (or the Bool form) and return the bound `gk`.
        GenTy::Int => hoist_scalar(ast, binds, gen_call),
        // `(= (% ((. Test gen)) 2) 0)` — the gen int's PARITY as a boolean (true iff even). Uses the low
        // bit, so it splits ~50/50 across the generated int stream — the earlier `(= gen 0)` was true only
        // for the exact int 0 (overwhelmingly false → near-zero coverage of the `true` case, PR #408).
        GenTy::Bool => hoist_scalar(ast, binds, |ast| {
            let g = gen_call(ast);
            let two = push_atom(
                ast,
                Leaf::Int {
                    value: crate::ast::IntValue::from_i64(2),
                    radix: crate::ast::Radix::Dec,
                },
            );
            let rem = name(ast, "%");
            let parity = push_list(ast, vec![rem, g, two]);
            let eq = name(ast, "=");
            let zero = push_atom(
                ast,
                Leaf::Int {
                    value: crate::ast::IntValue::zero(),
                    radix: crate::ast::Radix::Dec,
                },
            );
            push_list(ast, vec![eq, parity, zero])
        }),
        // A float: hoist `gk = ((. FloatWIDTH of-int) ((. Test gen)))` — the TOTAL integer→float conversion
        // of a fresh gen int, yielding an integer-valued float (…, -1.0, 0.0, 1.0, …). Not every float bit
        // pattern (no fractional/subnormal/NaN draws), but it exercises the sign + magnitude of the seeded
        // int pool AND shrinks with it, which is what a property test over a float collection needs.
        GenTy::Float(width) => {
            let w = *width;
            hoist_scalar(ast, binds, move |ast| {
                let g = gen_call(ast);
                let of_int = {
                    let dot = name(ast, ".");
                    let fmod = name(ast, if w == 32 { "Float32" } else { "Float64" });
                    let of = name(ast, "of-int");
                    push_list(ast, vec![dot, fmod, of])
                };
                push_list(ast, vec![of_int, g])
            })
        }
        // A VARIABLE-length list in `0..=G1_LIST_LEN`: generate `G1_LIST_LEN` candidate elements + a
        // hoisted count `c = (% Test.gen (LEN+1))`, then an `if`-chain picking the length-`c` prefix
        // (`(list)` / `(list e0)` / `(list e0 e1)` / …). This exercises the EMPTY list + short lists (the
        // classic off-by-one / empty-case property-test coverage) that a fixed-length never reached — with
        // no recursive-helper synthesis (all inline, still let-hoisted so each `Test.gen` lives in `host`).
        GenTy::List(elem) => build_var_list_gen(ast, elem, binds),
        // `(tuple <gen:T> …)` — one generated value per slot.
        GenTy::Tuple(slots) => {
            let head = name(ast, "tuple");
            let mut children = vec![head];
            for slot in slots {
                children.push(build_gen(ast, slot, binds));
            }
            push_list(ast, children)
        }
        // `(record (f <gen:T>) …)` — one generated value per named field, each a `(field-name value)` pair.
        GenTy::Record(fields) => {
            let head = name(ast, "record");
            let mut children = vec![head];
            for (fname, fty) in fields {
                let fval = build_gen(ast, fty, binds);
                let fnm = name(ast, fname);
                let pair = push_list(ast, vec![fnm, fval]);
                children.push(pair);
            }
            push_list(ast, children)
        }
        // A user sum: pick a variant by a hoisted `Test.gen % k`, then a nested `if`-chain constructs the
        // chosen variant `((. TYPE V) <gen:payload>)` (nullary variant = `(. TYPE V)`). The LAST variant is
        // the final `else`, so every draw lands on some variant (`% k` in `0..k`, and the chain covers all).
        GenTy::Sum {
            type_name,
            variants,
        } => build_sum_gen(ast, type_name, variants, binds),
        // `(Set.of (list <gen:ELEM> …))` — build a fixed-length list then dedup into a set.
        GenTy::Set(elem) => {
            let list_head = name(ast, "list");
            let mut list_children = vec![list_head];
            for _ in 0..G1_LIST_LEN {
                list_children.push(build_gen(ast, elem, binds));
            }
            let list = push_list(ast, list_children);
            let set_of = {
                let dot = name(ast, ".");
                let set = name(ast, "Set");
                let of = name(ast, "of");
                push_list(ast, vec![dot, set, of])
            };
            push_list(ast, vec![set_of, list])
        }
        // A fold of `Map.insert` over `G1_LIST_LEN` generated key/value pairs, seeded from `Map.empty`:
        // `(Map.insert (Map.insert (Map.empty) k0 v0) k1 v1) …`.
        GenTy::Map(kty, vty) => {
            // `(Map.empty)` — the seed.
            let mut acc = {
                let dot = name(ast, ".");
                let mapn = name(ast, "Map");
                let empty = name(ast, "empty");
                let member = push_list(ast, vec![dot, mapn, empty]);
                push_list(ast, vec![member])
            };
            for _ in 0..G1_LIST_LEN {
                let k = build_gen(ast, kty, binds);
                let v = build_gen(ast, vty, binds);
                let insert = {
                    let dot = name(ast, ".");
                    let mapn = name(ast, "Map");
                    let ins = name(ast, "insert");
                    push_list(ast, vec![dot, mapn, ins])
                };
                acc = push_list(ast, vec![insert, acc, k, v]);
            }
            acc
        }
    }
}

/// Build a user-sum `<gen>` — pick a variant index by a hoisted `(% ((. Test gen)) k)` and emit a nested
/// `if`-chain of variant constructions. Each variant `V` builds `((. TYPE V) <gen:payload>)` (or the bare
/// `(. TYPE V)` when nullary). Variant `i` is chosen by `(= sel i)`; the last variant is the trailing
/// `else`, so the chain is total over `sel ∈ 0..k`.
fn build_sum_gen(
    ast: &mut Arenas,
    type_name: &str,
    variants: &[(String, Option<GenTy>)],
    binds: &mut Vec<(StructId, StructId)>,
) -> StructId {
    // `sel = (% ((. Test gen)) k)` — the hoisted variant selector, bound to `gN`. Capture its NAME so
    // each `(= sel i)` comparison below can reference it with a fresh occurrence.
    let k = variants.len() as i64;
    let sel_name = format!("g{}", binds.len());
    {
        let g = gen_call(ast);
        let kn = push_atom(
            ast,
            Leaf::Int {
                value: crate::ast::IntValue::from_i64(k),
                radix: crate::ast::Radix::Dec,
            },
        );
        let rem = name(ast, "%");
        let sel_expr = push_list(ast, vec![rem, g, kn]);
        let var = name(ast, &sel_name);
        binds.push((var, sel_expr));
    }
    // Build each variant's construction expression (payloads recurse through `build_gen`, hoisting their
    // own `Test.gen`s into the same `binds` — evaluated unconditionally before the `if`, which is fine:
    // an unused draw is harmless, and it keeps every gen a plain `let`).
    let ctors: Vec<StructId> = variants
        .iter()
        .map(|(vname, payload)| {
            // `(. TYPE V)` — the variant constructor (member access).
            let ctor = {
                let dot = name(ast, ".");
                let tn = name(ast, type_name);
                let vn = name(ast, vname);
                push_list(ast, vec![dot, tn, vn])
            };
            match payload {
                None => ctor, // nullary variant: the ctor value itself
                Some(pty) => {
                    let pval = build_gen(ast, pty, binds);
                    push_list(ast, vec![ctor, pval])
                }
            }
        })
        .collect();
    // Fold the ctors into a nested `if`-chain from the LAST (the trailing else) backward: for i<k-1,
    // `(if (= sel i) ctor_i <rest>)`.
    let mut chain = *ctors
        .last()
        .expect("a sum has ≥1 variant (checked in classify_sum)");
    for i in (0..ctors.len().saturating_sub(1)).rev() {
        let cond = {
            let eq = name(ast, "=");
            // A fresh occurrence of the selector name for this comparison.
            let sel_use = name(ast, &sel_name);
            let iv = push_atom(
                ast,
                Leaf::Int {
                    value: crate::ast::IntValue::from_i64(i as i64),
                    radix: crate::ast::Radix::Dec,
                },
            );
            push_list(ast, vec![eq, sel_use, iv])
        };
        let if_head = name(ast, "if");
        chain = push_list(ast, vec![if_head, cond, ctors[i], chain]);
    }
    chain
}

/// Build a VARIABLE-length `(List ELEM)` in `0..=G1_LIST_LEN`: hoist a length selector
/// `c = (% ((. Test gen)) (LEN+1))` (so `c ∈ 0..=LEN`) + the `LEN` candidate element values (each let-
/// hoisted through `build_gen`), then a nested `if`-chain returning the length-`c` prefix:
/// `(if (<= c 0) (list) (if (<= c 1) (list e0) … (list e0 … e_{LEN-1})))`. All inline (no recursive
/// helper); every `Test.gen` is a `let` in the caller's chain, so it lives within the wrapper's `host`.
fn build_var_list_gen(
    ast: &mut Arenas,
    elem: &GenTy,
    binds: &mut Vec<(StructId, StructId)>,
) -> StructId {
    // Hoist the count `c = (% gen (LEN+1))`, capturing its name for the `(<= c i)` guards.
    let modn = (G1_LIST_LEN + 1) as i64;
    let count_name = format!("g{}", binds.len());
    {
        let g = gen_call(ast);
        let m = push_atom(
            ast,
            Leaf::Int {
                value: crate::ast::IntValue::from_i64(modn),
                radix: crate::ast::Radix::Dec,
            },
        );
        let rem = name(ast, "%");
        let expr = push_list(ast, vec![rem, g, m]);
        let var = name(ast, &count_name);
        binds.push((var, expr));
    }
    // Hoist the LEN candidate element values, each bound to its own `gN` name, so the prefix lists just
    // reference the names (never share an expression node across multiple `(list …)` parents). The bind
    // name is computed AFTER `build_gen` (which itself may push binds for a scalar element), so the index
    // is fresh.
    let elem_names: Vec<String> = (0..G1_LIST_LEN)
        .map(|_| {
            let e = build_gen(ast, elem, binds);
            let nm = format!("g{}", binds.len());
            let var = name(ast, &nm);
            binds.push((var, e));
            nm
        })
        .collect();
    // Build the prefix lists `(list)`, `(list e0)`, …, `(list e0 … e_{LEN-1})`.
    let prefixes: Vec<StructId> = (0..=G1_LIST_LEN)
        .map(|len| {
            let head = name(ast, "list");
            let mut children = vec![head];
            for enm in elem_names.iter().take(len) {
                children.push(name(ast, enm));
            }
            push_list(ast, children)
        })
        .collect();
    // Fold into `(if (<= c 0) prefix0 (if (<= c 1) prefix1 … prefixLEN))` — the last prefix is the else.
    let mut chain = prefixes[G1_LIST_LEN];
    for len in (0..G1_LIST_LEN).rev() {
        let cond = {
            let le = name(ast, "<=");
            let c_use = name(ast, &count_name);
            let iv = push_atom(
                ast,
                Leaf::Int {
                    value: crate::ast::IntValue::from_i64(len as i64),
                    radix: crate::ast::Radix::Dec,
                },
            );
            push_list(ast, vec![le, c_use, iv])
        };
        let if_head = name(ast, "if");
        chain = push_list(ast, vec![if_head, cond, prefixes[len], chain]);
    }
    chain
}

/// Hoist a scalar generator EXPRESSION into a fresh `let` binding `gN = <expr>` (recorded in `binds`) and
/// return a reference to the bound name `gN`. The binding index is `binds.len()`, so names are unique +
/// stable in generation order (`g0`, `g1`, …). Keeping every `Test.gen` in a `let` is what makes it live
/// within the wrapper's `host` scope (an inlined one under a constructor is rejected).
fn hoist_scalar(
    ast: &mut Arenas,
    binds: &mut Vec<(StructId, StructId)>,
    build_expr: impl FnOnce(&mut Arenas) -> StructId,
) -> StructId {
    let var_name = format!("g{}", binds.len());
    let expr = build_expr(ast);
    let var = name(ast, &var_name);
    binds.push((var, expr));
    // A fresh occurrence of the same name for the USE site (a distinct atom, same text).
    name(ast, &var_name)
}

/// `((. Test gen))` — one `Test.gen` performance (the nullary application of the member access
/// `Test.gen`). A fresh occurrence each call, so each pulls the next int from the runner's seeded pool.
fn gen_call(ast: &mut Arenas) -> StructId {
    let dot = name(ast, ".");
    let test = name(ast, "Test");
    let gen_nm = name(ast, "gen");
    let member = push_list(ast, vec![dot, test, gen_nm]);
    push_list(ast, vec![member])
}

#[cfg(test)]
mod tests {
    use crate::db::Db;

    /// A `@test` over a `(List Int64)` gains a synthesized `<name>-gen` wrapper that IS a test, while the
    /// original is no longer hoisted as a test (it becomes the wrapper's callee). Pins that the pass fires
    /// and neutralizes the original's `@test` in place (so it does not decline at the boundary).
    #[test]
    fn synthesizes_a_generator_wrapper_for_a_list_test() {
        let ast = crate::testkit::parse(
            "(do (@ test (def (p (: xs (List Int64))) (List.len xs))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        // The wrapper is a test; the original `p` is NOT (it lost its `@test`).
        assert!(
            test_names.iter().any(|n| n == "p-gen"),
            "the synthesized wrapper is hoisted as a test: {test_names:?}"
        );
        assert!(
            !test_names.iter().any(|n| n == "p"),
            "the original compound-param def is no longer a test: {test_names:?}"
        );
    }

    /// The TESTED tier: a `@test`-stacked `@ensures` rewrites the def body to `(let ((it BODY)) (if Q it
    /// (trap …)))`, so the postcondition `Q` is the test oracle over the def's result — and the pass branch
    /// `@ensures` enforcement (bare AND `@test`-stacked) is now v-verification's `verify_enforce::enforce`
    /// pass, which runs BEFORE `proptest_gen` — so THIS pass must never touch an `@ensures` node. Pin that
    /// `proptest_gen::synthesize` injects no postcondition machinery (no `trap`/`let`) for a bare `@ensures`:
    /// the `@ensures` is not proptest_gen's to rewrite (the earlier `rewrite_ensures_stacked_tests` pre-pass
    /// was deleted in the `@ensures`-ownership lockstep). `synthesize` still leaves the def a normal callee.
    #[test]
    fn proptest_gen_does_not_touch_a_bare_ensures() {
        let mut ast = crate::testkit::parse(
            "(do (@ (ensures (>= it 0)) (def (g (: n Int64)) n)) (export g))",
        );
        super::synthesize(&mut ast);
        let has_trap = (0..ast.structure.len()).any(|i| {
            ast.as_form(crate::ast::StructId(i as u32), "trap")
                .is_some()
        });
        assert!(
            !has_trap,
            "a bare @ensures (no @test) is not rewritten into a trapping test"
        );
    }

    /// A COMPOUND-param `@test` under a verification wrapper — `@test @requires(Q)` /`@test @ensures(Q)`,
    /// i.e. `(@ test (@ (requires|ensures Q) (def (f (: xs (List Int64))) …)))` — must still gain its `-gen`
    /// wrapper. `plan_for_item` peels the `(requires|ensures …)` layer (verify_enforce leaves that wrapper in
    /// place, rewriting only the body) to reach the compound def; without the peel the compound param would
    /// decline at the export boundary. Both stack shapes synthesize `f-gen` and unmark the original `f`.
    #[test]
    fn synthesizes_a_wrapper_for_a_compound_param_under_a_requires_or_ensures_wrapper() {
        for src in [
            "(do (@ test (@ (requires (< 0 (List.len xs))) (def (f (: xs (List Int64))) unit))) (def (o) 1))",
            "(do (@ test (@ (ensures (<= 0 (List.len it))) (def (f (: xs (List Int64))) xs))) (def (o) 1))",
        ] {
            let db = Db::load(crate::testkit::parse(src));
            let names: Vec<String> = db
                .test_defs()
                .into_iter()
                .map(|i| db.defs[i].name.clone())
                .collect();
            assert!(
                names.iter().any(|n| n == "f-gen") && !names.iter().any(|n| n == "f"),
                "a compound param under a @requires/@ensures wrapper gains f-gen: got {names:?}"
            );
        }
    }

    /// A `@test` with a SCALAR parameter is untouched (the boundary-arg route handles it — no wrapper).
    #[test]
    fn leaves_a_scalar_parameter_test_alone() {
        let ast = crate::testkit::parse(
            "(do (@ test (def (p (: n Int64)) (if (> n 0) unit (trap \"x\")))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            test_names.iter().any(|n| n == "p") && !test_names.iter().any(|n| n == "p-gen"),
            "a scalar-param test is left as-is (no wrapper): {test_names:?}"
        );
    }

    /// G2: a `(List Bool)` element is also generatable (the wrapper builds each element as `= (% gen 2) 0`), so
    /// a `@test` over `List Bool` gains a wrapper just like `List Int`.
    #[test]
    fn synthesizes_a_generator_wrapper_for_a_list_bool_test() {
        let ast = crate::testkit::parse(
            "(do (@ test (def (q (: bs (List Bool))) (List.len bs))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            test_names.iter().any(|n| n == "q-gen") && !test_names.iter().any(|n| n == "q"),
            "a List Bool test gains a generator wrapper: {test_names:?}"
        );
    }

    /// G3: a `(Tuple Int64 Bool)` parameter is generatable (`(tuple <gen> <gen>)`), and nesting composes
    /// (`(List (Tuple Int64 Bool))`) — both gain a wrapper.
    #[test]
    fn synthesizes_a_generator_wrapper_for_a_tuple_and_nested_test() {
        for (src, def, wrapper) in [
            (
                "(do (@ test (def (t (: p (Tuple Int64 Bool))) 0)) (def (o) 1))",
                "t",
                "t-gen",
            ),
            (
                "(do (@ test (def (u (: xs (List (Tuple Int64 Bool)))) (List.len xs))) (def (o) 1))",
                "u",
                "u-gen",
            ),
        ] {
            let db = Db::load(crate::testkit::parse(src));
            let names: Vec<String> = db
                .test_defs()
                .into_iter()
                .map(|i| db.defs[i].name.clone())
                .collect();
            assert!(
                names.iter().any(|n| n == wrapper) && !names.iter().any(|n| n == def),
                "{def}: expected wrapper {wrapper}, got {names:?}"
            );
        }
    }

    /// G4: a `(Record (f T)…)` parameter is generatable (`(record (f <gen>) …)`), and nesting composes
    /// (`(List (Record …))`) — both gain a wrapper.
    #[test]
    fn synthesizes_a_generator_wrapper_for_a_record_test() {
        for (src, def, wrapper) in [
            (
                "(do (@ test (def (r (: v (Record (x Int64) (y Bool)))) 0)) (def (o) 1))",
                "r",
                "r-gen",
            ),
            (
                "(do (@ test (def (lr (: xs (List (Record (a Int64) (b Bool)))))  (List.len xs))) (def (o) 1))",
                "lr",
                "lr-gen",
            ),
            // A bare-name USER SUM inside a record FIELD must resolve — the field-classify recursion must
            // keep passing the TOP-LEVEL `items` (not the field's own list) so `Ty` finds its `(type …)`
            // decl (PR #419: the arm bound `items` shadowing the param; renamed to `field_pair`). If the
            // recursion passed the field list, `Ty` would not resolve and this would decline (no wrapper).
            (
                "(do (type Ty (A Int64) (B Bool)) \
                   (@ test (def (rs (: v (Record (t Ty) (n Int64)))) 0)) (def (o) 1))",
                "rs",
                "rs-gen",
            ),
        ] {
            let db = Db::load(crate::testkit::parse(src));
            let names: Vec<String> = db
                .test_defs()
                .into_iter()
                .map(|i| db.defs[i].name.clone())
                .collect();
            assert!(
                names.iter().any(|n| n == wrapper) && !names.iter().any(|n| n == def),
                "{def}: expected wrapper {wrapper}, got {names:?}"
            );
        }
    }

    /// A `@test` over a type carrying a TYPE-LEVEL `@invariant` — `(@ (invariant Q) (type NAME …))` — must
    /// still recognize the type as generatable: the `@invariant` wrapper leaves the `(type …)` nested under
    /// `(@ …)`, so `classify_sum`'s decl scan sees through it (`type_decl_form`). Without the peel the type
    /// would be unrecognized and its `@test` decline as "not a scalar". (The invariant itself does not yet
    /// CONSTRAIN generation — that's the next increment; this pins that the annotated type at least generates.)
    #[test]
    fn generates_a_type_that_carries_a_type_level_invariant() {
        let src = "(do (@ (invariant (and (>= it 0) (<= it 100))) (type Percent (Pct Int64))) \
                     (@ test (def (p (: x Percent)) unit)) (def (o) 1))";
        let db = Db::load(crate::testkit::parse(src));
        let names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "p-gen") && !names.iter().any(|n| n == "p"),
            "an @invariant-annotated type is still generatable (its `@test` gains p-gen): got {names:?}"
        );
    }

    /// G5: a `@test` over a USER SUM `(type NAME (V PAYLOAD?)…)` gains a wrapper — the generator picks a
    /// variant by `Test.gen % k` and builds its payload. Covers a mix of payload'd + nullary variants,
    /// and a sum nested inside a `List`.
    #[test]
    fn synthesizes_a_generator_wrapper_for_a_sum_test() {
        for (src, def, wrapper) in [
            (
                "(do (type Ty (Var Int64) (Con Bool) (Nil)) \
                   (@ test (def (t (: v Ty)) 0)) (def (o) 1))",
                "t",
                "t-gen",
            ),
            (
                "(do (type Ty (Var Int64) (Con Bool)) \
                   (@ test (def (ls (: xs (List Ty))) (List.len xs))) (def (o) 1))",
                "ls",
                "ls-gen",
            ),
        ] {
            let db = Db::load(crate::testkit::parse(src));
            let names: Vec<String> = db
                .test_defs()
                .into_iter()
                .map(|i| db.defs[i].name.clone())
                .collect();
            assert!(
                names.iter().any(|n| n == wrapper) && !names.iter().any(|n| n == def),
                "{def}: expected wrapper {wrapper}, got {names:?}"
            );
        }
    }

    /// G6: `(Set ELEM)` and `(Map K V)` params are generatable (`(Set.of (list …))` / a `Map.insert`
    /// fold), and nesting composes (`List (Set Int64)`).
    #[test]
    fn synthesizes_a_generator_wrapper_for_set_and_map_tests() {
        for (src, def, wrapper) in [
            (
                "(do (@ test (def (s (: v (Set Int64))) 0)) (def (o) 1))",
                "s",
                "s-gen",
            ),
            (
                "(do (@ test (def (m (: v (Map Int64 Bool))) 0)) (def (o) 1))",
                "m",
                "m-gen",
            ),
            (
                "(do (@ test (def (ls (: xs (List (Set Int64)))) (List.len xs))) (def (o) 1))",
                "ls",
                "ls-gen",
            ),
        ] {
            let db = Db::load(crate::testkit::parse(src));
            let names: Vec<String> = db
                .test_defs()
                .into_iter()
                .map(|i| db.defs[i].name.clone())
                .collect();
            assert!(
                names.iter().any(|n| n == wrapper) && !names.iter().any(|n| n == def),
                "{def}: expected wrapper {wrapper}, got {names:?}"
            );
        }
    }

    /// A MULTI-parameter `@test` with at least one compound param gains a wrapper that generates ALL its
    /// args (in signature order). A test whose params are ALL scalars gets NO wrapper (the existing
    /// boundary-arg route generates each scalar), and a nullary test is untouched.
    #[test]
    fn multi_parameter_tests_synthesize_when_any_param_is_compound() {
        // (List Int64, Int64) — one compound + one scalar → wrapper generating both.
        let db = Db::load(crate::testkit::parse(
            "(do (@ test (def (p (: xs (List Int64)) (: n Int64)) 0)) (def (o) 1))",
        ));
        let names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "p-gen") && !names.iter().any(|n| n == "p"),
            "a multi-param test with a compound param gains a wrapper: {names:?}"
        );

        // (Int64, Bool) — ALL scalars → NO wrapper (boundary-arg route runs the original directly).
        let db2 = Db::load(crate::testkit::parse(
            "(do (@ test (def (q (: a Int64) (: b Bool)) 0)) (def (o) 1))",
        ));
        let names2: Vec<String> = db2
            .test_defs()
            .into_iter()
            .map(|i| db2.defs[i].name.clone())
            .collect();
        assert!(
            names2.iter().any(|n| n == "q") && !names2.iter().any(|n| n == "q-gen"),
            "an all-scalar multi-param test gets no wrapper: {names2:?}"
        );
    }

    /// A RECURSIVE sum (`Tree = Leaf Int64 | Node (Tuple Tree Tree)`) must DECLINE, not recurse forever
    /// (the classify depth guard) — no wrapper, so it declines at the boundary. Pins the stack-overflow
    /// guard: an unbounded generator is not synthesized.
    #[test]
    fn a_recursive_sum_declines_without_hanging() {
        let ast = crate::testkit::parse(
            "(do (type Tree (Leaf Int64) (Node (Tuple Tree Tree))) \
               (@ test (def (tr (: t Tree)) unit)) (def (o) 1))",
        );
        let db = Db::load(ast);
        let names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            !names.iter().any(|n| n == "tr-gen"),
            "a recursive sum gets no wrapper (depth guard): {names:?}"
        );
    }

    /// `classify_sum` models a variant as `(VNAME PAYLOAD?)` — zero or ONE payload occurrence (several
    /// fields are a single `(Tuple …)`/`(Record …)` payload). A variant with TWO+ payload occurrences
    /// (`(Var Int64 Bool)`) is not that shape, so the whole sum declines and no wrapper is synthesized —
    /// the `@test` then declines cleanly at the boundary rather than a mis-generated multi-slot variant.
    #[test]
    fn a_multi_payload_variant_declines() {
        let ast = crate::testkit::parse(
            "(do (type Bad (Var Int64 Bool) (Nil)) \
               (@ test (def (b (: v Bad)) unit)) (def (o) 1))",
        );
        let db = Db::load(ast);
        let names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            !names.iter().any(|n| n == "b-gen"),
            "a multi-payload variant gets no wrapper (single-payload guard): {names:?}"
        );
    }

    /// A `@test` over a genuinely NON-generatable element (`(List Char)` — `Char` is not yet generated) is
    /// left alone: no wrapper, so it declines at the boundary as before. (Nested `List`/`Tuple` over
    /// int/Bool/float leaves ARE generatable now — the non-generatable leaf is what stops it.)
    #[test]
    fn leaves_a_nongeneratable_element_alone() {
        let ast = crate::testkit::parse(
            "(do (@ test (def (r (: xs (List Char))) (List.len xs))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            !test_names.iter().any(|n| n == "r-gen"),
            "a non-generatable (Char) element gets no wrapper: {test_names:?}"
        );
    }

    /// G9: a `Float32`/`Float64` leaf is generatable when COMPOUND (nested under a collection/tuple/…),
    /// gaining a wrapper (`((. FloatN of-int) <gen-int>)`). A LONE float param stays on the boundary-arg
    /// route (the runner generates the scalar directly) — NO wrapper, like a lone `Int`/`Bool`.
    #[test]
    fn synthesizes_a_generator_wrapper_for_float_leaves() {
        // Compound floats → wrapper.
        for (src, def, wrapper) in [
            (
                "(do (@ test (def (lf (: xs (List Float64))) (List.len xs))) (def (o) 1))",
                "lf",
                "lf-gen",
            ),
            (
                "(do (@ test (def (tf (: p (Tuple Float32 Int64))) 0)) (def (o) 1))",
                "tf",
                "tf-gen",
            ),
        ] {
            let db = Db::load(crate::testkit::parse(src));
            let names: Vec<String> = db
                .test_defs()
                .into_iter()
                .map(|i| db.defs[i].name.clone())
                .collect();
            assert!(
                names.iter().any(|n| n == wrapper) && !names.iter().any(|n| n == def),
                "{def}: expected wrapper {wrapper}, got {names:?}"
            );
        }
        // A LONE Float64 param is a scalar → boundary-arg route, NO wrapper (same as lone Int/Bool).
        let db = Db::load(crate::testkit::parse(
            "(do (@ test (def (sf (: x Float64)) unit)) (def (o) 1))",
        ));
        let names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "sf") && !names.iter().any(|n| n == "sf-gen"),
            "a lone float param gets no wrapper (boundary route): {names:?}"
        );
    }

    /// PR #406: a program that declares `(effect Test (op fail …))` WITHOUT a `gen` op must NOT get a
    /// synthesized wrapper — appending a `(op gen …)` effect would collide with the existing `Test`, and
    /// reusing the gen-less one would call a non-existent `Test.gen`. The pass bails, leaving the
    /// compound-param `@test` to decline at the boundary as before.
    #[test]
    fn a_test_effect_without_gen_suppresses_synthesis() {
        let ast = crate::testkit::parse(
            "(do (effect Test (op fail (-> String Unit))) \
                 (@ test (def (p (: xs (List Int64))) (List.len xs))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            !test_names.iter().any(|n| n == "p-gen"),
            "a Test effect without `gen` suppresses the wrapper (no spurious Test.gen): {test_names:?}"
        );
    }

    /// The complement: a program that declares `(effect Test (op gen …))` itself IS usable — the pass
    /// reuses it (does not append a colliding second `Test`) and still synthesizes the wrapper.
    #[test]
    fn a_test_effect_with_gen_is_reused() {
        let ast = crate::testkit::parse(
            "(do (effect Test (op gen (-> Unit Int64))) \
                 (@ test (def (p (: xs (List Int64))) (List.len xs))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            test_names.iter().any(|n| n == "p-gen"),
            "a user-declared Test(gen) effect is reused + the wrapper synthesized: {test_names:?}"
        );
    }

    /// A compound-param `@exhaustive` def gains a synthesized wrapper (so its compound param does NOT
    /// decline at the export boundary, aborting the whole file), and the wrapper carries the `@exhaustive`
    /// marker forward (`Db::is_exhaustive` sees it) — the runner then declines it cleanly as an unbounded
    /// domain. Also covers the STACKED `@exhaustive @test` form: the middle `@test` node is neutralized so
    /// `strip_annotations` does not re-record the original compound def as a plain test.
    #[test]
    fn a_compound_exhaustive_test_synthesizes_an_exhaustive_wrapper() {
        for src in [
            // single `@exhaustive`
            "(do (@ exhaustive (def (e (: xs (List Bool))) (List.len xs))) (def (o) 1))",
            // stacked `@exhaustive @test`
            "(do (@ exhaustive (@ test (def (e (: xs (List Bool))) (List.len xs)))) (def (o) 1))",
        ] {
            let db = Db::load(crate::testkit::parse(src));
            let test_idx = db.test_defs();
            let names: Vec<String> = test_idx.iter().map(|&i| db.defs[i].name.clone()).collect();
            // The wrapper is the (only) hoisted test; the original `e` is neutralized (now a plain callee).
            assert!(
                names.iter().any(|n| n == "e-gen") && !names.iter().any(|n| n == "e"),
                "{src}: expected the exhaustive wrapper e-gen, got {names:?}"
            );
            // The wrapper carries `@exhaustive` forward.
            let gen_idx = db.def_by_name("e-gen").expect("e-gen def");
            assert!(
                db.is_exhaustive(gen_idx),
                "{src}: the synthesized wrapper is marked @exhaustive"
            );
        }
    }

    /// A LONE single-form test file — one `@test def` with a compound param and nothing else — parses as
    /// the bare `(@ test (def…))` AS the root (no enclosing `(do …)`). The pass must still fire: treat the
    /// root as a one-item list, synthesize the wrapper, and rebuild a `(do …)` root. Before this, such a
    /// file declined at the compound param's boundary (the pass only handled a `(do …)` root).
    #[test]
    fn a_lone_single_form_compound_test_synthesizes() {
        // No `(do …)` — the `(@ test …)` is the whole program.
        let db = Db::load(crate::testkit::parse(
            "(@ test (def (lp (: xs (List Int64))) (List.len xs)))",
        ));
        let names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "lp-gen") && !names.iter().any(|n| n == "lp"),
            "a lone single-form compound test gains a wrapper (no do-block needed): {names:?}"
        );
    }

    /// DEEP nesting composes: the recursive `<gen:T>` derivation threads through a `Map` VALUE that is
    /// itself compound (`Map Int64 (List Bool)` — the map-value recursion, distinct from a scalar value)
    /// and through three levels (`List (Tuple (Set Int64) (Record …))` — list-elem → tuple-slot →
    /// set-elem / record-field). Each still synthesizes one wrapper. Pins that no nesting path is missed.
    #[test]
    fn synthesizes_a_wrapper_for_deeply_nested_compounds() {
        for (src, def, wrapper) in [
            (
                "(do (@ test (def (mlb (: m (Map Int64 (List Bool)))) 0)) (def (o) 1))",
                "mlb",
                "mlb-gen",
            ),
            (
                "(do (@ test (def (deep (: v (List (Tuple (Set Int64) (Record (x Int64) (y Bool)))))) \
                   (List.len v))) (def (o) 1))",
                "deep",
                "deep-gen",
            ),
        ] {
            let db = Db::load(crate::testkit::parse(src));
            let names: Vec<String> = db
                .test_defs()
                .into_iter()
                .map(|i| db.defs[i].name.clone())
                .collect();
            assert!(
                names.iter().any(|n| n == wrapper) && !names.iter().any(|n| n == def),
                "{def}: expected wrapper {wrapper} for a deeply nested compound, got {names:?}"
            );
        }
    }
}
