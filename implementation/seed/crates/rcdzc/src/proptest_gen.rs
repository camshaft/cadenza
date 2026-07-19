//! Compiler-directed generators for property tests over COLLECTION types (F1 / approach A —
//! compiler-synthesis; see `implementation/design/DESIGN-property-test-collection-generators-rcdzc.md`).
//!
//! `cdz test` already property-tests a `@test` def with SCALAR parameters: the runner generates each
//! scalar and (for a guest that performs `Test.gen-int : Unit -> Int64`) drives a seeded int pool with
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
//! consumes one `Test.gen-int` int (`Bool` = `(= (% gen 2) 0)`, the parity), a `(List ELEM)` builds a
//! VARIABLE-length list (`0..=G1_LIST_LEN`, a gen'd count picking a prefix — so the empty + short lists
//! are exercised), a `(Tuple T…)` builds `(tuple <gen:T> …)`. Every `Test.gen-int` is hoisted into a `let`
//! (an inlined one under a constructor is not seen within the `host` scope). The existing gen-driven
//! runner detects the wrapper (it pulls `Test.gen-int` ints), runs `--trials` trials, and shrinks over the
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

/// Append a decimal integer literal atom — the node-builder shorthand for a `Leaf::Int` (used by the
/// range-constrained generator to emit the bound/span constants).
fn int_lit(ast: &mut Arenas, v: i64) -> StructId {
    push_atom(
        ast,
        Leaf::Int {
            value: crate::ast::IntValue::from_i64(v),
            radix: crate::ast::Radix::Dec,
        },
    )
}

/// The fixed list length the G1 wrapper generates. Small — enough to exercise a non-trivial list while
/// keeping the synthesized `let`-chain short. Variable length (a `Test.gen-int`-derived, bounded count) is a
/// later increment.
const G1_LIST_LEN: usize = 3;

/// The `Test` effect's driver-op NAME — the op each generator performs to pull one random `Int64`. Named
/// `gen-int` (NOT bare `gen`): the browser test driver transpiles the component with jco, which emits an
/// internal top-level `let gen = …` (its `_initGenerator`), and importing a component op member literally
/// named `gen` collides — `const { gen } = imports.test` → `SyntaxError: Identifier 'gen' has already been
/// declared`, throwing at import before any run (v-guide-infra 2026-07-19). `gen-int` sidesteps the jco
/// internal (also avoiding bare `next`, which jco calls on generators). A single source of truth for the
/// name so the effect decl, the member-access call, and the collision guards can never drift.
const GEN_OP: &str = "gen-int";

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
    // wrapper needs `Test.gen-int`, appending its own `(effect Test …)` would collide with the existing name,
    // and the existing one has no `gen` to reuse. Bail out (the compound-param `@test` then declines at
    // the boundary as before) rather than emit a wrapper calling a non-existent `Test.gen-int` (PR #406).
    if test_declared_without_gen(ast, &items) {
        return;
    }

    // Find each `(@ test (def SIG BODY))` item whose SIG has exactly one `(: name (List ELEM))` param
    // with ELEM an integer type. Record (item index, def-name occ text, param count) to synthesize for.
    let mut plans: Vec<TestPlan> = Vec::new();
    for &item in &items {
        if let Some(plan) = plan_for_item(ast, item, &items) {
            plans.push(plan);
        }
    }
    if plans.is_empty() {
        return;
    }

    // Neutralize ONLY the `@test`/`@exhaustive` MARKER layers of each plan IN PLACE: rewrite that annotation
    // node to BE its inner `(def …)` (adopt the def's children). `strip_annotations` (which runs after this
    // pass) scans EVERY arena node, not just root-reachable ones — so an orphaned `(@ test …)`/`(@ exhaustive …)`
    // left behind would make it re-record the original compound-param def as a test → the boundary decline we
    // are avoiding. So the test-marker layers MUST be neutralized. But a `(@ (requires Q) …)`/`(@ (ensures Q) …)`
    // VERIFICATION layer is DELIBERATELY LEFT INTACT: strip records its predicate into `db.requires`/`db.ensures`
    // (keyed on the now-plain def) — which the DECODER reads (`gen_ty_of_wrapper_param`) to re-apply a
    // param-level `@requires` min-length floor, so a shrunk counterexample renders the correct in-domain LENGTH
    // (not `p([])` for a `len>=2` property). Neutralizing the requires layer too would drop that predicate and
    // desync the decode. `plan.nested_anns` holds every `@` layer (outermost-first, incl. `item`); rewrite the
    // test/exhaustive ones, skip requires/ensures.
    for plan in &plans {
        if let crate::ast::Struct::List(inner_children) = ast.get(plan.inner_def).clone() {
            for &layer in &plan.nested_anns {
                let is_test_marker = ast
                    .as_form(layer, "@")
                    .and_then(|l| l.first())
                    .and_then(|&h| ast.as_name(h))
                    .is_some_and(|n| n == "test" || n == "exhaustive");
                if is_test_marker {
                    ast.structure[layer.0 as usize] =
                        crate::ast::Struct::List(inner_children.clone());
                }
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
    inner_def: StructId,
    /// The def's name as text (e.g. `"p"`) — the wrapper calls `(p <gen-arg>…)`.
    def_name: String,
    /// The synthesized wrapper's name (`"<def_name>-gen"`).
    wrapper_name: String,
    /// The generatable type of EACH parameter, in signature order — one `<gen:T>` per param. EMPTY when
    /// `declining` (a non-generatable-leaf compound has no `<gen:T>` to build).
    gen_tys: Vec<GenTy>,
    /// A DECLINING wrapper: a COMPOUND param carries a leaf the generator can't produce yet (e.g. `Char` in
    /// `(List Char)`). Rather than let the compound param hit the export boundary and ABORT THE WHOLE FILE
    /// (killing sibling tests), synthesize a nullary wrapper that TRAPS with an actionable message — so the
    /// runner reports a clean per-test `FAIL <name>-gen: …` and the file's other tests still run. Mirrors the
    /// `@exhaustive`-compound clean-decline, extended to the non-generatable-LEAF case. `gen_tys` is empty.
    declining: bool,
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
/// one or more `Test.gen-int` ints). This is the compiler-directed "Arbitrary-like" derivation over the type
/// structure: a scalar consumes one gen int; a `List`/`Tuple` recurses into its element/slot types. A
/// type outside this set (`Char`, a bare/unresolved type) is not (yet) generatable — `classify_ty` returns
/// `None`, so the `@test` declines at the boundary as before.
#[derive(Clone, Debug)]
pub enum GenTy {
    /// An integer type (`Int8`…`UInt64`): `<gen>` = `((. Test gen))` (the int at the element width).
    Int,
    /// An integer CONSTRAINED to an inclusive range `[lo, hi]` — from a type-level `@invariant` whose
    /// predicate is a recognized range over `self` (`(and (>= self LO) (<= self HI))` and mirrors). `<gen>` maps a
    /// fresh `Test.gen-int` int into the range so the generated value ALWAYS satisfies the invariant (no wasted
    /// reject cycle): `(+ LO (Int64.rem-euclid ((. Test gen)) SPAN))` where `SPAN = HI-LO+1`. This is the
    /// constrained-generation path (operator directive: "invariants inform how random values are generated,
    /// so we never waste a cycle just to get rejected"). An unrecognized invariant shape stays plain `Int`.
    IntRange { lo: i64, hi: i64 },
    /// `Bool`: `<gen>` = `(= (% ((. Test gen)) 2) 0)` (the gen int's parity → a ~50/50 boolean).
    Bool,
    /// A float type (`Float32`/`Float64`), carrying its width: `<gen>` = `((. FloatWIDTH of-int) <gen-int>)`
    /// — an integer-valued float from a fresh `Test.gen-int` int (the TOTAL `float-of-int` conversion, realized
    /// in both backends). A LONE float parameter already crosses the boundary (the runner generates it), so
    /// this variant only matters NESTED under a `List`/`Tuple`/… where no boundary representation exists.
    Float(u32),
    /// `(List ELEM)`: `<gen>` = a VARIABLE-length list, length in `[min_len, G1_LIST_LEN]`, drawn from the
    /// gen pool. `min_len` (0 by default = unconstrained) is a floor from a recognized MIN-LENGTH refinement
    /// — a param-level `@requires`/type-level `@invariant` `(< 0 (List.len self))` / `(<= K (List.len self))` —
    /// so a "non-empty"/"at least K" precondition GENERATES in-domain rather than drawing a shorter list that
    /// trips the enforced precondition (the reject-free constrained-gen path). `min_len` is clamped to
    /// `G1_LIST_LEN` (a floor above the max candidate count would be unsatisfiable — capped, not an error).
    List(Box<GenTy>, usize),
    /// `(Tuple T…)`: `<gen>` = `(tuple <gen:T> …)`, one generated value per slot.
    Tuple(Vec<GenTy>),
    /// `(Record (f T)…)`: `<gen>` = `(record (f <gen:T>) …)`, one generated value per named field.
    Record(Vec<(String, GenTy)>),
    /// A user SUM `(type NAME (V PAYLOAD?)…)` named by a bare type name: `<gen>` picks a variant by
    /// `Test.gen-int % k` and constructs `((. NAME V) <gen:PAYLOAD>)` (a nullary variant is just `(. NAME V)`).
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
fn plan_for_item(ast: &Arenas, item: StructId, items: &[StructId]) -> Option<TestPlan> {
    // `(@ NAME INNER)` where NAME is `test` or `exhaustive` — the two annotations that mark a property
    // test this pass synthesizes a generator wrapper for. `@exhaustive` is included so a COMPOUND-param
    // `@exhaustive` def gets a wrapper too (else its compound param declines at the export boundary,
    // aborting the whole file); the wrapper carries the `@exhaustive` marker forward so the runner reports
    // it (a compound domain is unbounded → the runner declines it as not-exhaustively-enumerable).
    // Peel the ANNOTATION STACK down to the `(def …)`, in ANY order, requiring a `test`/`exhaustive` marker
    // SOMEWHERE in the stack. The stack is one or more nested `(@ HEAD INNER)` layers:
    //   • `@exhaustive @test def` → `(@ exhaustive (@ test (def…)))`;
    //   • `@test @requires(Q) def` → `(@ test (@ (requires Q) (def…)))` — @test OUTER (verify_enforce runs
    //     before this pass and LEAVES the `(@ (requires|ensures …) …)` wrapper so strip records the predicate);
    //   • `@requires(Q) @test def` → `(@ (requires Q) (@ test (def…)))` — @requires OUTER, the NATURAL
    //     precondition-first spelling. This ordering must ALSO synthesize the wrapper, else a compound-param
    //     def under an outer @requires hits the export boundary and ABORTS THE WHOLE FILE.
    // A peelable layer's head is `test`/`exhaustive` (bare name) OR a call-style `(requires …)`/`(ensures …)`.
    // Track whether a `test`/`exhaustive` marker was seen (and whether it was `exhaustive`), and record EVERY
    // annotation node so `synthesize` neutralizes them (else strip_annotations re-records the compound def as
    // a test). Anything else (an unknown annotation) stops the peel — the def underneath then isn't ours.
    let mut node = item;
    let mut nested_anns: Vec<StructId> = Vec::new();
    let mut saw_test = false;
    let mut exhaustive = false;
    // Also collect any `@requires(Q)` predicate nodes in the stack — a param-level precondition constrains
    // generation (a min-length `(<= K (List.len xs))` floors the drawn list so the enforced (D) pre never
    // spuriously trips). `(@ (requires Q) …)` → the head is `(requires Q)`, whose first child is `Q`.
    let mut requires_preds: Vec<StructId> = Vec::new();
    while let Some(layer) = ast.as_form(node, "@") {
        let &head = layer.first()?;
        if ast.as_name(head) == Some("test") {
            saw_test = true;
        } else if ast.as_name(head) == Some("exhaustive") {
            saw_test = true;
            exhaustive = true;
        } else if let Some(req) = ast.as_form(head, "requires") {
            if let Some(&q) = req.first() {
                requires_preds.push(q);
            }
        } else if ast.as_form(head, "ensures").is_none() {
            break; // an unknown annotation head — not a peelable test/verification layer
        }
        nested_anns.push(node);
        node = *layer.get(1)?;
    }
    // Must have found a `test`/`exhaustive` marker somewhere in the stack (else not a property test we own).
    if !saw_test {
        return None;
    }
    // `nested_anns` now holds EVERY `@` layer (outermost-first); `node` is the `(def …)`. The OUTERMOST layer
    // is `item` itself (neutralized by `synthesize` rewriting it to the def); the rest are the middle nodes.
    let inner = node;
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
        let &param_name_occ = ann_param.first()?;
        let &ty = ann_param.get(1)?;
        // A COMPOUND param is a type FORM (`(List …)`, `(Tuple …)`, `(Record …)`) or a bare user-sum NAME —
        // it has no component-boundary representation, so a `@test` over it needs the synthesized wrapper
        // (or, if a leaf isn't generatable, a DECLINING wrapper). A bare SCALAR name (`Int64`/`Bool`/…) or an
        // unresolvable bare name is NOT our concern here: a generatable scalar goes the boundary-arg route,
        // and an ungeneratable bare name (`Char` scalar) is a LAYOUT/boundary matter, not proptest_gen's.
        let is_compound_form = ast.as_name(ty).is_none();
        match classify_ty(ast, ty, items) {
            Some(mut gt) => {
                // Apply a param-level `@requires` MIN-LENGTH to a `(List …)` param: a precondition like
                // `@requires(<= 2 (List.len xs))` floors the drawn list at 2, so generation stays IN-DOMAIN
                // and the enforced (D) precondition trap never spuriously trips (mirrors the type-level
                // `@invariant` min-length path, but keyed on the PARAM NAME here, not the invariant binder).
                if let (GenTy::List(_, ml), Some(pname)) = (&mut gt, ast.as_name(param_name_occ)) {
                    let floor = requires_preds
                        .iter()
                        .filter_map(|&q| min_len_for_param(ast, q, pname))
                        .max();
                    if let Some(m) = floor {
                        *ml = (*ml).max(m);
                    }
                }
                // A SCALAR param (`Int`/`Bool`/`Float`) has a boundary representation → boundary-arg route,
                // no wrapper. Only a COMPOUND param (no boundary form) forces the synthesized wrapper.
                if !matches!(gt, GenTy::Int | GenTy::Bool | GenTy::Float(_)) {
                    any_compound = true;
                }
                gen_tys.push(gt);
            }
            // A COMPOUND FORM whose leaf the generator can't produce yet (e.g. `Char` in `(List Char)`):
            // DECLINE CLEANLY per-test rather than let the compound param abort the whole file. A
            // non-generatable BARE NAME (a `Char` scalar param) is NOT ours — leave it to the boundary
            // (layout reports "parameter type is ambiguous"); returning None here keeps that path unchanged.
            None if is_compound_form => {
                return Some(TestPlan {
                    inner_def: inner,
                    wrapper_name: format!("{def_name}-gen"),
                    def_name,
                    gen_tys: Vec::new(),
                    declining: true,
                    exhaustive,
                    nested_anns,
                });
            }
            None => return None, // a non-generatable bare-name (scalar) param — layout's concern, not ours
        }
    }
    if !any_compound {
        return None; // all-scalar signature — the boundary-arg route handles it; no wrapper
    }
    Some(TestPlan {
        inner_def: inner,
        // Suffix `-gen`: a hyphen-delimited segment that begins with a letter, so the wrapper name is a
        // valid component extern name (an extern name's `-`-separated segments must each start with a
        // letter — a `$` or a digit-led segment fails boundary-name validation). The wrapper is what
        // `cdz test` reports, so the name stays readable (`p` → `p-gen`).
        wrapper_name: format!("{def_name}-gen"),
        def_name,
        gen_tys,
        declining: false,
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
/// draws, so a caller can decode a shrunk `Test.gen-int` int pool back into the concrete value (the `cdz test`
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
    let mut gt = classify_ty(&db.ast, ty_node, &items)?;
    // RE-APPLY a type-level `@invariant` constraint from `db.invariants` (NOT the AST wrapper). This runs at
    // `cdz test` time — AFTER `strip_annotations`, which REMOVES the `(@ (invariant Q) (type …))` wrapper
    // (`invariant` is in KNOWN_ANNOTATIONS) but RECORDS the predicate into `db.invariants`. So `classify_sum`
    // above (reading the now-stripped AST) sees a BARE type and produces an UNCONSTRAINED `Int`/`List` — but
    // the GENERATOR ran pre-strip and DID constrain it, so the decoder must match or a counterexample renders
    // the raw pool int (e.g. `Pct(-716…)`) instead of the in-domain value. Re-apply via `Db::invariant_of`.
    reapply_recorded_invariant(db, &mut gt);
    // RE-APPLY a param-level `@requires` MIN-LENGTH on the DECODE side, matching the generation-side floor
    // (`plan_for_item`). `synthesize` deliberately LEAVES the `(@ (requires …) …)` wrapper intact on the
    // neutralized def (it neutralizes only the test/exhaustive markers), so `strip_annotations` records the
    // predicate into `db.requires` and `db.requires_of(def)` carries it here. Floor the `(List …)` param's
    // `min_len` so the shrunk counterexample decodes the SAME count the generator drew (a bare-min decode
    // would render a wrong LENGTH — `p([])` for a `len>=2` property, which a user can't even replay).
    if let GenTy::List(_, ml) = &mut gt
        && let Some(pname) = db
            .ast
            .as_form(param, ":")
            .and_then(|t| t.first())
            .and_then(|&n| db.ast.as_name(n))
    {
        let floor = db
            .requires_of(def)
            .iter()
            .filter_map(|&q| min_len_for_param(&db.ast, q, pname))
            .max();
        if let Some(m) = floor {
            *ml = (*ml).max(m);
        }
    }
    Some(gt)
}

/// Re-apply a type-level `@invariant` (from `db.invariants`, which survives `strip_annotations`) to EVERY
/// `GenTy::Sum` newtype in `gt` — RECURSIVELY, descending Tuple slots / List elements / Record fields / Sum
/// payloads — mirroring `classify_sum`'s AST-sourced application (which recurses via `classify_ty_at`), but
/// sourcing the predicate from the Db so it works POST-STRIP (the decoder path). For a single-variant newtype
/// whose payload is an `Int` with a recognized range invariant → `IntRange`; a `List` with a min-length
/// invariant → floored `min_len`. Keeps the decoder's `GenTy` identical to the generator's at every nesting
/// depth, so a counterexample decodes to the in-domain value the wrapper actually drew — even for a refined
/// newtype NESTED inside a compound (e.g. `(Tuple Pct Bool)`), not just a top-level newtype param.
fn reapply_recorded_invariant(db: &crate::db::Db, gt: &mut GenTy) {
    match gt {
        // A newtype sum: re-source + apply its own recorded invariant, THEN recurse into its payloads (a
        // payload may itself be / contain a refined newtype).
        GenTy::Sum {
            type_name,
            variants,
        } => {
            if let Some(decl) = db.type_decls.iter().find(|d| &d.name == type_name)
                && let Some(pred) = db.invariant_of(decl.occ)
            {
                if let Some((lo, hi)) = invariant_int_range(&db.ast, pred)
                    && let [(_, Some(GenTy::Int))] = variants.as_slice()
                {
                    variants[0].1 = Some(GenTy::IntRange { lo, hi });
                } else if let Some(min) =
                    min_len_for_param(&db.ast, pred, crate::invariant_establish::VALUE_BINDER)
                    && let [(_, Some(GenTy::List(_, ml)))] = variants.as_mut_slice()
                {
                    *ml = (*ml).max(min);
                }
            }
            for (_, payload) in variants.iter_mut() {
                if let Some(p) = payload {
                    reapply_recorded_invariant(db, p);
                }
            }
        }
        // Compound shapes: descend into every constituent so a nested refined newtype is re-constrained.
        GenTy::Tuple(slots) => slots
            .iter_mut()
            .for_each(|s| reapply_recorded_invariant(db, s)),
        GenTy::Record(fields) => fields
            .iter_mut()
            .for_each(|(_, f)| reapply_recorded_invariant(db, f)),
        GenTy::List(elem, _) => reapply_recorded_invariant(db, elem),
        GenTy::Set(elem) => reapply_recorded_invariant(db, elem),
        GenTy::Map(k, v) => {
            reapply_recorded_invariant(db, k);
            reapply_recorded_invariant(db, v);
        }
        // Leaves — nothing nested to re-constrain.
        GenTy::Int | GenTy::IntRange { .. } | GenTy::Bool | GenTy::Float(_) => {}
    }
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
    // `(List ELEM)` — recurse into the element type. `min_len` 0 (unconstrained) here; a min-length
    // refinement is applied later by the predicate consumer (plan_for_item for a param-level `@requires`).
    if let Some(list_tail) = ast.as_form(ty, "List") {
        let &elem = list_tail.first()?;
        return Some(GenTy::List(
            Box::new(classify_ty_at(ast, elem, items, depth + 1)?),
            0,
        ));
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

/// The type-level `@invariant` PREDICATE occ on `item`, if it carries one — the `Q` in `(@ (invariant Q)
/// (type …))`. Scans the annotation wrappers `type_decl_form` peels for the one whose head is a call-style
/// `(invariant Q)` application. `None` for a bare (un-refined) type or any other annotation. Used to
/// CONSTRAIN generation of the type (a recognized range → an `IntRange` leaf).
fn type_invariant_pred(ast: &Arenas, item: StructId) -> Option<StructId> {
    let mut node = item;
    for _ in 0..=MAX_GEN_DEPTH {
        if ast.as_form(node, "type").is_some() {
            return None; // reached the type decl with no `@invariant` seen
        }
        let ann = ast.as_form(node, "@")?;
        let &head = ann.first()?;
        if let Some(inv) = ast.as_form(head, "invariant") {
            return inv.first().copied(); // the predicate Q
        }
        node = *ann.get(1)?;
    }
    None
}

/// The generation WINDOW width used to close a ONE-SIDED integer `@invariant` bound. A lower-bound-only
/// invariant `(>= self LO)` admits every value in `[LO, i64::MAX]` — too wide to sample uniformly, but the
/// generator only needs to draw values that SATISFY the bound. So we map into `[LO, LO+WINDOW]` (or, for an
/// upper-only bound, `[HI-WINDOW, HI]`): every drawn value is in-domain (the whole point — §Refinements
/// Constrain Generation), and the window is wide enough to exercise the property meaningfully. Chosen as a
/// round power-of-ten so a rendered counterexample reads legibly.
const ONE_SIDED_INVARIANT_WINDOW: i64 = 1_000_000;

/// Recognize an inclusive integer RANGE `[lo, hi]` from a type-level `@invariant` predicate over the value
/// binder `self` — `(and (>= self LO) (<= self HI))`, `(<= LO self)`/mirrors, a lone bound, or `(= self K)`. A
/// TWO-SIDED range maps in directly; a ONE-SIDED bound is CLOSED with a generation window
/// ([`ONE_SIDED_INVARIANT_WINDOW`]) so a lower-bound-only `(>= self 0)` still generates in-domain (was: fell
/// through to unconstrained → drew out-of-domain values the construct-site `@invariant` trap then rejected
/// as a spurious counterexample). `None` only for an unrecognized shape (no bound on `self`, a non-linear /
/// opaque predicate) or a contradictory two-sided range (`lo > hi`) → generation stays unconstrained
/// (reject-free fallback). Mirrors the scalar `@requires` bound recognizer, but over `self` and guest-side.
/// (`self` = [`crate::invariant_establish::VALUE_BINDER`], the operator-canonical `@invariant` binder.)
fn invariant_int_range(ast: &Arenas, pred: StructId) -> Option<(i64, i64)> {
    let (mut lo, mut hi): (Option<i64>, Option<i64>) = (None, None);
    // Collect comparison conjuncts: descend a top-level `(and …)`/`(& …)`, else treat `pred` as one cmp.
    let mut stack = vec![pred];
    let mut conjuncts = Vec::new();
    while let Some(p) = stack.pop() {
        if let Some(t) = ast.as_form(p, "and").or_else(|| ast.as_form(p, "&")) {
            stack.extend(t.iter().copied());
        } else {
            conjuncts.push(p);
        }
    }
    for c in conjuncts {
        for op in [">=", ">", "<=", "<", "="] {
            let Some(t) = ast.as_form(c, op) else {
                continue;
            };
            if t.len() != 2 {
                return None;
            }
            let it_is =
                |n: StructId| ast.as_name(n) == Some(crate::invariant_establish::VALUE_BINDER);
            let lit = |n: StructId| ast.as_int(n).and_then(|v| v.to_i64());
            // `(op self LIT)` or the mirror `(op LIT self)` — normalize to `self OP' LIT`.
            let (val, mirrored) = if it_is(t[0]) {
                (lit(t[1])?, false)
            } else if it_is(t[1]) {
                (lit(t[0])?, true)
            } else {
                return None; // a comparison not against `it` + a literal — unrecognized
            };
            let op = if mirrored {
                match op {
                    "<" => ">",
                    ">" => "<",
                    "<=" => ">=",
                    ">=" => "<=",
                    other => other,
                }
            } else {
                op
            };
            match op {
                ">=" => lo = Some(lo.map_or(val, |l| l.max(val))),
                ">" => lo = Some(lo.map_or(val + 1, |l| l.max(val + 1))),
                "<=" => hi = Some(hi.map_or(val, |h| h.min(val))),
                "<" => hi = Some(hi.map_or(val - 1, |h| h.min(val - 1))),
                "=" => {
                    lo = Some(val);
                    hi = Some(val);
                }
                _ => {}
            }
            break;
        }
    }
    match (lo, hi) {
        // A fully-bounded range maps in directly (a contradictory `l > h` is rejected → unconstrained).
        (Some(l), Some(h)) => (l <= h).then_some((l, h)),
        // Lower-bound only `(>= self LO)`: generate `[LO, LO+WINDOW]` — every value satisfies `>= LO`. Clamp
        // the top with a saturating add so a LO near i64::MAX doesn't overflow (it degenerates to `[LO, MAX]`).
        (Some(l), None) => Some((l, l.saturating_add(ONE_SIDED_INVARIANT_WINDOW))),
        // Upper-bound only `(<= self HI)`: generate `[HI-WINDOW, HI]` — every value satisfies `<= HI`. Clamp
        // the bottom with a saturating sub so a HI near i64::MIN doesn't underflow.
        (None, Some(h)) => Some((h.saturating_sub(ONE_SIDED_INVARIANT_WINDOW), h)),
        // No bound on `it` recognized → don't constrain.
        (None, None) => None,
    }
}

/// The minimum LIST LENGTH a refinement predicate `pred` requires of the parameter named `param`, if it is
/// a recognized lower-bound on `(List.len param)` — so a "non-empty"/"at least K" precondition GENERATES a
/// long-enough list rather than drawing a shorter one that trips the enforced precondition. Recognizes
/// `(< K (List.len p))` → K+1, `(<= K (List.len p))` → K, `(> (List.len p) K)` → K+1, `(>= (List.len p) K)`
/// → K (and the mirrors), AND descends a top-level `(and …)`/`(& …)` — taking the MAX floor across its
/// lower-bound conjuncts — so a compound `(and (< 0 (List.len self)) (<= (List.len self) 10))` still floors the
/// length (was: matched only a BARE comparison, so a conjunction fell through to unconstrained and drew the
/// empty list the construct-site `@invariant` trap then rejected as a spurious counterexample). Upper-bound
/// conjuncts (`<`/`<=`) don't floor the length (the generator already caps at `G1_LIST_LEN`). `None` if no
/// lower-bound conjunct is recognized → the list stays unconstrained (reject-free fallback). Only a POSITIVE
/// lower bound yields a floor (a `min_len` of 0 is the default, no constraint). Mirrors the conjunct descent
/// in [`invariant_int_range`].
fn min_len_for_param(ast: &Arenas, pred: StructId, param: &str) -> Option<usize> {
    // Is `n` the term `(List.len param)` = `((. List len) param)`? (a call of the member access to `param`).
    let is_len_of_param = |n: StructId| -> bool {
        let call = match ast.get(n) {
            crate::ast::Struct::List(v) => v.as_slice(),
            _ => return false,
        };
        // `[ (. List len), param ]`
        if call.len() != 2 || ast.as_name(call[1]) != Some(param) {
            return false;
        }
        ast.as_form(call[0], ".").is_some_and(|m| {
            m.len() == 2 && ast.as_name(m[0]) == Some("List") && ast.as_name(m[1]) == Some("len")
        })
    };
    let lit = |n: StructId| ast.as_int(n).and_then(|v| v.to_i64());
    // Collect comparison conjuncts: descend a top-level `(and …)`/`(& …)`, else treat `pred` as one cmp.
    let mut stack = vec![pred];
    let mut conjuncts = Vec::new();
    while let Some(p) = stack.pop() {
        if let Some(t) = ast.as_form(p, "and").or_else(|| ast.as_form(p, "&")) {
            stack.extend(t.iter().copied());
        } else {
            conjuncts.push(p);
        }
    }
    // The floor is the MAX over every recognized lower-bound conjunct (a stricter bound wins). Non-length
    // and upper-bound conjuncts contribute nothing (they don't floor the length) and are skipped.
    let mut floor: Option<usize> = None;
    for c in conjuncts {
        for op in [">=", ">", "<=", "<"] {
            let Some(t) = ast.as_form(c, op) else {
                continue;
            };
            if t.len() != 2 {
                break; // a malformed comparison of this op — not a length bound
            }
            // Normalize to `len OP' K`: `(op len K)` direct, or `(op K len)` mirrored (flip the operator).
            let (k, op) = if is_len_of_param(t[0]) {
                let Some(k) = lit(t[1]) else { break };
                (k, op)
            } else if is_len_of_param(t[1]) {
                let Some(k) = lit(t[0]) else { break };
                let flipped = match op {
                    "<" => ">",
                    ">" => "<",
                    "<=" => ">=",
                    ">=" => "<=",
                    o => o,
                };
                (k, flipped)
            } else {
                break; // not a bound on `(List.len param)`
            };
            // A LOWER bound on the length → a min floor. `len > K` ⇒ ≥ K+1; `len >= K` ⇒ ≥ K. Upper bounds
            // (`<`/`<=`) don't floor the length (the generator already caps at G1_LIST_LEN) → no floor.
            let this = match op {
                ">" => usize::try_from(k + 1).ok().filter(|&m| m > 0),
                ">=" => usize::try_from(k).ok().filter(|&m| m > 0),
                _ => None,
            };
            if let Some(m) = this {
                floor = Some(floor.map_or(m, |f| f.max(m)));
            }
            break;
        }
    }
    floor
}

fn classify_sum(ast: &Arenas, type_name: &str, items: &[StructId], depth: usize) -> Option<GenTy> {
    // Find `(type NAME variant…)` with a matching NAME — SEEING THROUGH any annotation wrapper. A type
    // declaration may be bare `(type NAME …)` OR annotated `(@ (invariant …) (type NAME …))` (a type-level
    // `@invariant` records a refinement over the value binder `self`; verify_enforce/strip_annotations leave
    // the `(@ …)` wrapper in place). `type_decl_form` peels the wrapper so an `@invariant`-refined type is
    // still recognized as generatable (its underlying variants), not declined as an unknown type.
    let decl_item = items.iter().copied().find(|&it| {
        type_decl_form(ast, it).is_some_and(|tail| {
            tail.first()
                .is_some_and(|&n| ast.as_name(n) == Some(type_name))
        })
    })?;
    let decl_tail = type_decl_form(ast, decl_item)?;
    // A recognized type-level `@invariant` constrains the newtype payload's generation (below): an integer
    // RANGE → an `IntRange` int; a min-length `(< 0 (List.len self))` → a min-length `List`.
    let inv_pred = type_invariant_pred(ast, decl_item);
    let inv_range = inv_pred.and_then(|p| invariant_int_range(ast, p));
    let inv_min_len =
        inv_pred.and_then(|p| min_len_for_param(ast, p, crate::invariant_establish::VALUE_BINDER));
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
    // Apply a recognized type-level `@invariant` RANGE to a NEWTYPE-int: `self` (the whole nominal value)
    // maps to the underlying int of a single-variant, single-Int-payload type (`Percent = Pct(Int64)` — a
    // range `(and (>= self 0) (<= self 100))` on the Percent value IS a bound on the Pct payload int). Replace
    // that payload's `Int` with an `IntRange` so it generates in-domain. Only the newtype shape (one variant,
    // one Int payload) is constrained here — a multi-variant or non-Int-payload type keeps its raw generator
    // (a range invariant over such a value doesn't map to a single int; unconstrained + reject-free fallback).
    if let Some((lo, hi)) = inv_range
        && let [(_, Some(GenTy::Int))] = variants.as_slice()
    {
        variants[0].1 = Some(GenTy::IntRange { lo, hi });
    }
    // A recognized MIN-LENGTH invariant on a newtype-List (`NonEmpty = Mk (List T)` with `@invariant(< 0
    // (List.len self))`): floor that payload list's length so it generates non-empty (in-domain). Same
    // newtype shape (one variant, one List payload); multi-variant/other keeps its raw generator.
    if let Some(min) = inv_min_len
        && let [(_, Some(GenTy::List(_, ml)))] = variants.as_mut_slice()
    {
        *ml = (*ml).max(min);
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
            // The op children follow the effect name: `(effect Test (op gen-int …) (op fail …) …)`. Reuse
            // the existing `Test` ONLY if it actually declares the driver op — a `Test` effect that declares
            // only `fail` (or anything but the driver op) does NOT provide `Test.gen-int`, and treating it as
            // if it did would make the wrapper call a non-existent op (Copilot PR #406). Check the op names,
            // not just the effect name.
            return eff[1..]
                .iter()
                .filter_map(|&op| ast.as_form(op, "op"))
                .any(|op_tail| op_tail.first().and_then(|&n| ast.as_name(n)) == Some(GEN_OP));
        }
    }
    false
}

/// Whether the program declares an effect named `Test` that does NOT carry the driver op — the case where
/// this pass CANNOT proceed: the wrapper needs `Test.gen-int`, but appending its own `(effect Test …)` would
/// collide with the existing `Test` name, and the existing one has no driver op to reuse. Synthesis bails
/// out for such a program (the compound-param `@test` then declines at the boundary as before, rather
/// than emitting a wrapper that calls a non-existent `Test.gen-int`).
fn test_declared_without_gen(ast: &Arenas, items: &[StructId]) -> bool {
    for &item in items {
        if let Some(eff) = ast.as_form(item, "effect")
            && eff.first().and_then(|&n| ast.as_name(n)) == Some("Test")
        {
            let has_gen = eff[1..]
                .iter()
                .filter_map(|&op| ast.as_form(op, "op"))
                .any(|op_tail| op_tail.first().and_then(|&n| ast.as_name(n)) == Some(GEN_OP));
            return !has_gen;
        }
    }
    false
}

/// Build `(effect Test (op gen-int (-> Unit Int64)) (op fail (-> String Unit)))`.
fn build_test_effect(ast: &mut Arenas) -> StructId {
    // `gen-int : Unit -> Int64` — the driver op every generator pulls from (see [`GEN_OP`] for why not `gen`).
    let gen_op = {
        let arrow = {
            let head = name(ast, "->");
            let unit = name(ast, "Unit");
            let i64 = name(ast, "Int64");
            push_list(ast, vec![head, unit, i64])
        };
        let head = name(ast, "op");
        let gen_nm = name(ast, GEN_OP);
        push_list(ast, vec![head, gen_nm, arrow])
    };
    // `fail : String -> Unit` — the report op a test performs to name WHY it failed. The runner recovers a
    // FAIL message from a `.fail`-suffixed observed op (`observed_failure_message`), so a wrapper that
    // performs `Test.fail("reason")` before trapping surfaces an actionable reason instead of a bare trap.
    // The DECLINING wrapper uses this to name a non-generatable-leaf cause (e.g. Char); it mirrors the
    // `assert`-prelude's own `Test.fail` op, so a user program that already declares `Test` with `gen`+`fail`
    // reuses it (declares_test_gen only requires `gen`).
    let fail_op = {
        let arrow = {
            let head = name(ast, "->");
            let string = name(ast, "String");
            let unit = name(ast, "Unit");
            push_list(ast, vec![head, string, unit])
        };
        let head = name(ast, "op");
        let fail_nm = name(ast, "fail");
        push_list(ast, vec![head, fail_nm, arrow])
    };
    let head = name(ast, "effect");
    let test = name(ast, "Test");
    push_list(ast, vec![head, test, gen_op, fail_op])
}

/// Build the `@test`-marked nullary wrapper for a plan:
/// `(@ test (def (NAME-gen) (host (Test) (NAME <gen:ParamType>))))`, where `<gen:ParamType>` is the
/// recursively-built generator expression for the parameter's type. Every `Test.gen-int` performance is a
/// fresh `((. Test gen))`, so the runner's seeded int pool drives (and shrinks) the whole generated value.
fn build_wrapper(ast: &mut Arenas, plan: &TestPlan) -> StructId {
    // A DECLINING wrapper: the compound param has a non-generatable leaf (e.g. `Char`), so there is no
    // `<gen:T>` to build. Emit a nullary `(def (NAME-gen) (trap "…"))` that TRAPS with an actionable
    // message — the runner reports a clean per-test `FAIL NAME-gen: <message>` (a trap is the existing
    // per-test failure channel) and the file's SIBLING tests still run, instead of the compound param
    // aborting the whole file at the export boundary. No `host`/`Test.gen-int` — a bare trap is a plain nullary
    // test the runner invokes directly.
    if plan.declining {
        // Perform `Test.fail("reason")` THEN `trap`, inside `(host (Test) …)`. The runner recovers a FAIL
        // message from the `.fail`-suffixed observed op (`observed_failure_message`), so the per-test decline
        // NAMES its cause (a non-generatable leaf like Char) instead of a bare `body trapped: wasm
        // unreachable`. The trailing `trap` forces the FAIL outcome (a returning test would PASS). Mirrors
        // the assert-prelude's `(Test.fail msg); trap(…)` shape; `Test.fail` is declared on the synthesized
        // effect (`build_test_effect`).
        let fail_call = {
            // `((. Test fail) "reason")` — the dotted member-access call, same shape as `gen_call`.
            let member = {
                let dot = name(ast, ".");
                let test = name(ast, "Test");
                let fail_nm = name(ast, "fail");
                push_list(ast, vec![dot, test, fail_nm])
            };
            let msg = push_atom(
                ast,
                Leaf::Str(format!(
                    "{}: a compound parameter has a leaf the generator cannot produce yet (e.g. Char) — not \
                     property-testable; narrow the element type or drop the @test",
                    plan.def_name
                )),
            );
            push_list(ast, vec![member, msg])
        };
        let trap = {
            let t = name(ast, "trap");
            let m = push_atom(ast, Leaf::Str("not property-testable".to_string()));
            push_list(ast, vec![t, m])
        };
        // `(do (Test.fail "…") (trap "…"))` — sequence: perform the report, then trap. (`do` is the
        // AST-level sequencing form; the ML `;` is surface-only and unbound in the arena.)
        let seq = {
            let do_head = name(ast, "do");
            push_list(ast, vec![do_head, fail_call, trap])
        };
        // `(host (Test) <seq>)` — delegate the Test effect to the boundary (the runner answers `Test.fail`).
        let host = {
            let head = name(ast, "host");
            let test = name(ast, "Test");
            let effs = push_list(ast, vec![test]);
            push_list(ast, vec![head, effs, seq])
        };
        let def = {
            let head = name(ast, "def");
            let sig = {
                let nm = name(ast, &plan.wrapper_name);
                push_list(ast, vec![nm])
            };
            push_list(ast, vec![head, sig, host])
        };
        // Mark it a test (declining `@exhaustive` stays `@exhaustive` for symmetry, though both decline).
        let at = name(ast, "@");
        let ann = name(
            ast,
            if plan.exhaustive {
                "exhaustive"
            } else {
                "test"
            },
        );
        return push_list(ast, vec![at, ann, def]);
    }
    // Build `<gen:ParamType>`, HOISTING every `Test.gen-int` performance into its own `let` binding: a
    // `Test.gen-int` inlined directly inside a compound constructor argument (`(tuple (Test.gen-int) …)`) is not
    // seen as within the enclosing `(host (Test) …)` scope and is rejected ("no enclosing handler"),
    // whereas a `let`-bound one is fine. So each leaf becomes `gk`, bound to its gen expression, and the
    // constructors reference the bound names.
    let mut binds: Vec<(StructId, StructId)> = Vec::new();
    // Build one `<gen:T>` per parameter, in signature order (each hoists its own `Test.gen-int`s into `binds`).
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
/// derivation — while HOISTING every `Test.gen-int` performance into `binds` (a `(var, gen-expr)` list the
/// caller wraps in `let`s). A scalar consumes one gen int (a hoisted `let`, returning the bound var); a
/// `(List ELEM)` builds `(list …)` of `G1_LIST_LEN` recursively-generated elements; a `(Tuple T…)` builds
/// `(tuple …)`. Hoisting is required because an inlined `Test.gen-int` inside a constructor argument is not
/// seen within the enclosing `host` scope (rejected), but a `let`-bound one is.
fn build_gen(ast: &mut Arenas, ty: &GenTy, binds: &mut Vec<(StructId, StructId)>) -> StructId {
    match ty {
        // A scalar: hoist `gk = ((. Test gen))` (or the Bool form) and return the bound `gk`.
        GenTy::Int => hoist_scalar(ast, binds, gen_call),
        // A RANGE-CONSTRAINED int (from a recognized type-level `@invariant`): map a fresh gen int INTO
        // `[lo, hi]` so the value ALWAYS satisfies the invariant — no reject cycle. `<gen>` =
        // `(+ LO (% (& ((. Test gen)) i64::MAX) SPAN))`: mask the gen int to NON-NEGATIVE (clear the sign
        // bit — `%` in Cadenza takes the sign of the dividend, so a raw negative gen would push below LO),
        // then `% SPAN` to `[0, SPAN)`, then `+ LO` to `[LO, HI]`. `SPAN = HI-LO+1` (checked non-zero: a
        // valid range has lo<=hi so SPAN>=1). A degenerate lo>hi (contradictory invariant) is not built as a
        // range (classify rejects it → plain Int), so SPAN is always >= 1 here.
        GenTy::IntRange { lo, hi } => {
            let (lo, hi) = (*lo, *hi);
            hoist_scalar(ast, binds, move |ast| {
                let g = gen_call(ast);
                // `(& gen 0x7FFF_FFFF_FFFF_FFFF)` — clear the sign bit → a non-negative i64.
                let nonneg = {
                    let andop = name(ast, "&");
                    let mask = int_lit(ast, i64::MAX);
                    push_list(ast, vec![andop, g, mask])
                };
                // `(% <nonneg> SPAN)` → `[0, SPAN)`.
                let span = hi.wrapping_sub(lo).wrapping_add(1);
                let modded = {
                    let rem = name(ast, "%");
                    let span_lit = int_lit(ast, span);
                    push_list(ast, vec![rem, nonneg, span_lit])
                };
                // `(+ LO <modded>)` → `[LO, HI]`.
                let plus = name(ast, "+");
                let lo_lit = int_lit(ast, lo);
                push_list(ast, vec![plus, lo_lit, modded])
            })
        }
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
        // hoisted count `c = (% Test.gen-int (LEN+1))`, then an `if`-chain picking the length-`c` prefix
        // (`(list)` / `(list e0)` / `(list e0 e1)` / …). This exercises the EMPTY list + short lists (the
        // classic off-by-one / empty-case property-test coverage) that a fixed-length never reached — with
        // no recursive-helper synthesis (all inline, still let-hoisted so each `Test.gen-int` lives in `host`).
        GenTy::List(elem, min_len) => build_var_list_gen(ast, elem, *min_len, binds),
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
        // A user sum: pick a variant by a hoisted `Test.gen-int % k`, then a nested `if`-chain constructs the
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
    // own `Test.gen-int`s into the same `binds` — evaluated unconditionally before the `if`, which is fine:
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

/// Build a VARIABLE-length `(List ELEM)` in `min_len..=G1_LIST_LEN`: hoist a length selector + the `LEN`
/// candidate element values (each let-hoisted through `build_gen`), then a nested `if`-chain returning the
/// length-`c` prefix: `(if (<= c 0) (list) (if (<= c 1) (list e0) … (list e0 … e_{LEN-1})))`. `min_len` (a
/// floor from a recognized min-length refinement, clamped to `G1_LIST_LEN`) shifts the count into
/// `[min_len, LEN]` so a "non-empty"/"at least K" list generates in-domain: `c = MIN + (% gen (LEN+1-MIN))`
/// (MIN=0 → the original `% (LEN+1)`). All inline (no recursive helper); every `Test.gen-int` is a `let` in the
/// caller's chain, so it lives within the wrapper's `host`.
fn build_var_list_gen(
    ast: &mut Arenas,
    elem: &GenTy,
    min_len: usize,
    binds: &mut Vec<(StructId, StructId)>,
) -> StructId {
    // A floor above the max candidate count is unsatisfiable — cap it at G1_LIST_LEN (generate the longest
    // available list rather than error). The span the gen int is reduced over is `LEN+1-MIN` (≥1).
    let min = min_len.min(G1_LIST_LEN);
    let span = (G1_LIST_LEN + 1 - min) as i64;
    // Hoist the count `c = MIN + (% (& gen i64::MAX) SPAN)` (∈ [MIN, LEN]). The gen int is masked to
    // NON-NEGATIVE first (`& i64::MAX` clears the sign bit) — `%` in Cadenza takes the sign of the dividend,
    // so a raw negative gen would make `c` negative and the `(<= c 0)` guard wrongly pick the empty list
    // (violating a min_len floor). Capture the name for the `(<= c i)` guards.
    let count_name = format!("g{}", binds.len());
    {
        let g = gen_call(ast);
        let nonneg = {
            let andop = name(ast, "&");
            let mask = int_lit(ast, i64::MAX);
            push_list(ast, vec![andop, g, mask])
        };
        let span_lit = int_lit(ast, span);
        let rem = name(ast, "%");
        let modded = push_list(ast, vec![rem, nonneg, span_lit]);
        let expr = if min == 0 {
            modded
        } else {
            let plus = name(ast, "+");
            let min_lit = int_lit(ast, min as i64);
            push_list(ast, vec![plus, min_lit, modded])
        };
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
/// stable in generation order (`g0`, `g1`, …). Keeping every `Test.gen-int` in a `let` is what makes it live
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

/// `((. Test gen-int))` — one `Test.gen-int` performance (the nullary application of the member access
/// `Test.gen-int`). A fresh occurrence each call, so each pulls the next int from the runner's seeded pool.
fn gen_call(ast: &mut Arenas) -> StructId {
    let dot = name(ast, ".");
    let test = name(ast, "Test");
    let gen_nm = name(ast, GEN_OP);
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
            "(do (@ (ensures (>= ret 0)) (def (g (: n Int64)) n)) (export g))",
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
            "(do (@ test (@ (ensures (<= 0 (List.len ret))) (def (f (: xs (List Int64))) xs))) (def (o) 1))",
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
        let src = "(do (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64))) \
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

    /// A recognized RANGE `@invariant` over a newtype-int constrains generation: the payload becomes a
    /// `GenTy::IntRange{lo,hi}` (generate in-domain), not a plain `Int`. `(and (>= self 0) (<= self 100))` on
    /// `Percent = Pct(Int64)` → the Pct payload is `IntRange{0,100}`. A ONE-SIDED bound is CLOSED with a
    /// generation window (`(>= self 0)` → `IntRange{0, 1_000_000}`) so it too generates in-domain. Checks the
    /// `GenTy` classification directly.

    #[test]
    fn a_range_invariant_constrains_a_newtype_int_to_an_intrange() {
        // `classify_sum(name)` resolves the `(type NAME …)` from `items` itself, so we pass the top-level
        // items + the type NAME string — no need to synthesize a name-atom node.
        let ast = crate::testkit::parse(
            "(do (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64))) (def (o) 1))",
        );
        let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
        let gt = super::classify_sum(&ast, "Percent", &items, 0);
        match gt {
            Some(super::GenTy::Sum { variants, .. }) => match variants.as_slice() {
                [(v, Some(super::GenTy::IntRange { lo: 0, hi: 100 }))] if v == "Pct" => {}
                other => panic!("expected Pct→IntRange{{0,100}}, got {other:?}"),
            },
            other => panic!("expected a Sum GenTy, got {other:?}"),
        }
        // A ONE-SIDED lower-bound invariant `(>= self 0)` is closed with the generation window → the payload
        // is `IntRange{0, ONE_SIDED_INVARIANT_WINDOW}`, so generation stays in-domain (never draws a value
        // below the bound that the construct-site @invariant trap would reject as a spurious counterexample).
        let ast2 = crate::testkit::parse(
            "(do (@ (invariant (>= self 0)) (type NonNeg (Mk Int64))) (def (o) 1))",
        );
        let items2: Vec<_> = ast2.as_form(ast2.root, "do").unwrap().to_vec();
        let gt2 = super::classify_sum(&ast2, "NonNeg", &items2, 0);
        match gt2 {
            Some(super::GenTy::Sum { variants, .. }) => match variants.as_slice() {
                [(v, Some(super::GenTy::IntRange { lo: 0, hi }))]
                    if v == "Mk" && *hi == super::ONE_SIDED_INVARIANT_WINDOW => {}
                other => panic!("expected Mk→IntRange{{0, WINDOW}}, got {other:?}"),
            },
            other => panic!("expected a Sum GenTy, got {other:?}"),
        }
    }

    /// A recognized MIN-LENGTH `@invariant` over a newtype-List constrains its payload list to be non-empty:
    /// `@invariant(< 0 (List.len self))` on `NEList = Mk (List Int64)` → the Mk payload is `List(_, min_len=1)`,
    /// so generation floors the length at 1 (never draws the empty list that would violate the invariant).
    #[test]
    fn a_min_length_invariant_constrains_a_newtype_list_to_non_empty() {
        let ast = crate::testkit::parse(
            "(do (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64)))) (def (o) 1))",
        );
        let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
        let gt = super::classify_sum(&ast, "NEList", &items, 0);
        match gt {
            Some(super::GenTy::Sum { variants, .. }) => match variants.as_slice() {
                [(v, Some(super::GenTy::List(_, 1)))] if v == "Mk" => {}
                other => panic!("expected Mk→List(_, min_len=1), got {other:?}"),
            },
            other => panic!("expected a Sum GenTy, got {other:?}"),
        }
    }

    /// A min-length `@invariant` inside a CONJUNCTION still floors the length: `min_len_for_param` descends a
    /// top-level `(and …)` (like `invariant_int_range`) and takes the MAX lower-bound floor. `(and (<= 2
    /// (List.len self)) (<= (List.len self) 8))` on `Buf = Mk (List Int64)` → the Mk payload is `List(_,
    /// min_len=2)` (the `<= 2` conjunct floors; the upper-bound conjunct is ignored). REGRESSION: a bare-
    /// comparison-only recognizer missed the conjunction, so generation drew the empty list the construct-site
    /// @invariant trap rejected as a spurious counterexample.
    #[test]
    fn a_conjunction_min_length_invariant_floors_the_newtype_list_length() {
        let ast = crate::testkit::parse(
            "(do (@ (invariant (and (<= 2 (List.len self)) (<= (List.len self) 8))) (type Buf (Mk (List Int64)))) (def (o) 1))",
        );
        let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
        let gt = super::classify_sum(&ast, "Buf", &items, 0);
        match gt {
            Some(super::GenTy::Sum { variants, .. }) => match variants.as_slice() {
                [(v, Some(super::GenTy::List(_, 2)))] if v == "Mk" => {}
                other => {
                    panic!("expected Mk→List(_, min_len=2) from the conjunction, got {other:?}")
                }
            },
            other => panic!("expected a Sum GenTy, got {other:?}"),
        }
    }

    /// A CONTRADICTORY range invariant `(and (>= self 10) (<= self 5))` — an empty range no value satisfies —
    /// must NOT produce a broken `IntRange` (a negative SPAN = HI-LO+1 = -4 would generate garbage). The
    /// `lo <= hi` guard in `invariant_int_range` returns `None`, so the payload stays a PLAIN `Int`
    /// (unconstrained): generation falls back safely and the unsatisfiable invariant surfaces as an honest
    /// property failure (every value violates it), rather than the generator silently claiming in-domain.
    /// Pins that the `lo <= hi` guard is load-bearing (breaker probe, 2026-07-18).
    #[test]
    fn a_contradictory_range_invariant_falls_back_to_unconstrained() {
        let ast = crate::testkit::parse(
            "(do (@ (invariant (and (>= self 10) (<= self 5))) (type Bad (Mk Int64))) (def (o) 1))",
        );
        let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
        let gt = super::classify_sum(&ast, "Bad", &items, 0);
        assert!(
            matches!(gt, Some(super::GenTy::Sum { ref variants, .. }) if matches!(variants.as_slice(), [(_, Some(super::GenTy::Int))])),
            "a contradictory range invariant leaves the payload a plain Int (no broken negative-SPAN IntRange): {gt:?}"
        );
    }

    /// Recognizer-level coverage of `invariant_int_range` across the shapes the e2e tests don't pin directly
    /// (point equality, one-sided upper, negative/strict bounds, no-bound). Extracts the `(invariant Q)`
    /// predicate from a parsed decl via `type_invariant_pred`, then asserts the exact `(lo, hi)` the range
    /// recognizer distills — pinning the arithmetic (strict `>`/`<` = ±1, `saturating` one-sided window,
    /// mirrored `(op K self)`, point `(= self K)`) that the generator + decoder both depend on.
    #[test]
    fn invariant_int_range_distills_bounds_across_shapes() {
        // Parse a `(@ (invariant Q) (type T (V Int64)))` decl and hand the predicate Q to the recognizer.
        let range_of = |src: &str| -> Option<(i64, i64)> {
            let ast = crate::testkit::parse(src);
            let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
            let decl = *items
                .iter()
                .find(|&&it| super::type_decl_form(&ast, it).is_some())
                .expect("a type decl in the fixture");
            let pred =
                super::type_invariant_pred(&ast, decl).expect("an (invariant Q) on the decl");
            super::invariant_int_range(&ast, pred)
        };
        let win = super::ONE_SIDED_INVARIANT_WINDOW;
        // Two-sided inclusive.
        assert_eq!(
            range_of(
                "(do (@ (invariant (and (>= self 0) (<= self 100))) (type T (V Int64))) (def (o) 1))"
            ),
            Some((0, 100))
        );
        // Strict two-sided: `> 0` ⇒ lo 1, `< 10` ⇒ hi 9.
        assert_eq!(
            range_of(
                "(do (@ (invariant (and (> self 0) (< self 10))) (type T (V Int64))) (def (o) 1))"
            ),
            Some((1, 9))
        );
        // Point equality `(= self 5)` ⇒ the singleton range [5, 5].
        assert_eq!(
            range_of("(do (@ (invariant (= self 5)) (type T (V Int64))) (def (o) 1))"),
            Some((5, 5))
        );
        // One-sided lower `(>= self 0)` ⇒ [0, WINDOW]; mirrored `(<= 0 self)` is the same bound.
        assert_eq!(
            range_of("(do (@ (invariant (>= self 0)) (type T (V Int64))) (def (o) 1))"),
            Some((0, win))
        );
        assert_eq!(
            range_of("(do (@ (invariant (<= 0 self)) (type T (V Int64))) (def (o) 1))"),
            Some((0, win))
        );
        // One-sided upper `(<= self 100)` ⇒ [100-WINDOW, 100].
        assert_eq!(
            range_of("(do (@ (invariant (<= self 100)) (type T (V Int64))) (def (o) 1))"),
            Some((100i64.wrapping_sub(win), 100))
        );
        // Negative lower bound `(>= self -100)` ⇒ [-100, -100+WINDOW].
        assert_eq!(
            range_of("(do (@ (invariant (>= self -100)) (type T (V Int64))) (def (o) 1))"),
            Some((-100, (-100i64).saturating_add(win)))
        );
        // No bound on `self` (an opaque predicate) ⇒ None (unconstrained fallback).
        assert_eq!(
            range_of("(do (@ (invariant (> other 0)) (type T (V Int64))) (def (o) 1))"),
            None
        );
    }

    /// G5: a `@test` over a USER SUM `(type NAME (V PAYLOAD?)…)` gains a wrapper — the generator picks a
    /// variant by `Test.gen-int % k` and builds its payload. Covers a mix of payload'd + nullary variants,
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

    /// A `@test` over a compound with a NON-generatable leaf (`(List Char)` — `Char` not yet generated) gets
    /// a DECLINING wrapper: `r-gen` IS synthesized (a trapping nullary def) and the original `r` is
    /// neutralized. This is the clean per-test decline — the compound param no longer reaches the export
    /// boundary (which would ABORT THE WHOLE FILE, killing sibling tests); instead the runner reports a
    /// per-test `FAIL r-gen` and siblings still run. (Nested `List`/`Tuple` over int/Bool/float leaves ARE
    /// generatable and get a REAL generator wrapper; only the non-generatable LEAF triggers the declining one.)
    #[test]
    fn a_nongeneratable_leaf_compound_gets_a_declining_wrapper() {
        let mut ast = crate::testkit::parse(
            "(do (@ test (def (r (: xs (List Char))) (List.len xs))) (def (other) 1))",
        );
        // Synthesize directly so we can inspect the wrapper body (a trapping nullary def), before Db load.
        super::synthesize(&mut ast);
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            test_names.iter().any(|n| n == "r-gen") && !test_names.iter().any(|n| n == "r"),
            "a non-generatable-leaf compound gets a DECLINING wrapper (r-gen), original r neutralized: \
             {test_names:?}"
        );
        // The declining wrapper's body TRAPS (a bare trap → the runner reports a per-test FAIL, siblings run).
        let r_gen = db
            .defs
            .iter()
            .find(|d| d.name == "r-gen")
            .expect("r-gen def exists");
        assert!(
            r_gen.params.is_empty(),
            "the declining wrapper is nullary (neutralizes the compound param → never hits the boundary)"
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

    /// PR #406: a program that declares `(effect Test (op fail …))` WITHOUT the `gen-int` driver op must NOT
    /// get a synthesized wrapper — appending a `(op gen-int …)` effect would collide with the existing `Test`,
    /// and reusing the driver-less one would call a non-existent `Test.gen-int`. The pass bails, leaving the
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
            "a Test effect without `gen` suppresses the wrapper (no spurious Test.gen-int): {test_names:?}"
        );
    }

    /// The complement: a program that declares `(effect Test (op gen-int …))` itself IS usable — the pass
    /// reuses it (does not append a colliding second `Test`) and still synthesizes the wrapper.
    #[test]
    fn a_test_effect_with_gen_is_reused() {
        let ast = crate::testkit::parse(
            "(do (effect Test (op gen-int (-> Unit Int64))) \
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
