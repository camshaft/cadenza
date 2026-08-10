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
    push_atom(ast, Leaf::Name(n.into()))
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
                // Apply a param-level match-based `@requires` to a `Sum` param: drop a constructor an arm
                // forbids (`((T.None) false)` → the wrapper never draws `None`, so the enforced (D)
                // precondition can't spuriously trap on `f(None)`), and narrow a kept constructor's Int
                // payload to a same-arm payload range (`((T.Some n) (>= n 0))` → `Some` draws in `[0,…]`, no
                // spurious `Some(-1)`). Shared with the decode side so the `% k` selector + payload draw stay
                // in sync. Unsatisfiable-forbid keeps every variant (honest trap over an empty sum).
                if let (GenTy::Sum { variants, .. }, Some(pname)) =
                    (&mut gt, ast.as_name(param_name_occ))
                {
                    constrain_sum_variants(ast, variants, &requires_preds, pname);
                }
                // Narrow a SCALAR `Int` param to a recognized param-level `@requires` RANGE. When another param
                // is compound, EVERY param (this scalar included) is drawn through the synthesized wrapper — but
                // the wrapper drew the scalar UNCONSTRAINED, so a precondition like `@requires(k >= 0)` on
                // `f(xs: List Int64, k: Int64)` let it draw `k < 0` and the enforced (D) precondition spuriously
                // tripped. Apply the same `int_range_over` the sum-payload guard uses, keyed on the PARAM NAME,
                // so the wrapper scalar generates in-domain (the boundary-arg route already handles an all-scalar
                // sig; an IntRange stays boundary-representable below, so this never turns an all-scalar sig into
                // a wrapper). Mirrored on the decode side (`gen_ty_of_wrapper_param`).
                if let (GenTy::Int, Some(pname)) = (&gt, ast.as_name(param_name_occ))
                    && let Some((lo, hi)) = requires_preds
                        .iter()
                        .find_map(|&q| int_range_over(ast, q, pname))
                {
                    gt = GenTy::IntRange { lo, hi };
                }
                // A SCALAR param (`Int`/`IntRange`/`Bool`/`Float`) has a boundary representation → boundary-arg
                // route, no wrapper. Only a COMPOUND param (no boundary form) forces the synthesized wrapper. A
                // narrowed `IntRange` scalar stays scalar here (else an all-scalar sig would wrongly synthesize).
                if !matches!(
                    gt,
                    GenTy::Int | GenTy::IntRange { .. } | GenTy::Bool | GenTy::Float(_)
                ) {
                    any_compound = true;
                }
                gen_tys.push(gt);
            }
            // A COMPOUND FORM whose leaf the generator can't produce yet (e.g. `Char` in `(List Char)`), OR a
            // bare-name KNOWN CONCRETE non-generatable scalar (`Char`/`Rational`/`BigInt`/`String`/`Symbol`):
            // DECLINE CLEANLY per-test rather than let the param abort the WHOLE `cdz test` file (and kill its
            // sibling tests). A heap/non-boundary scalar has no `<gen:T>` to draw — the correct outcome is a
            // per-test `FAIL NAME-gen: not property-testable`, exactly as for a non-generatable compound leaf.
            // (Before this, `Char` aborted at layout and `Rational`/`BigInt`/`String`/`Symbol` aborted at
            // serialize — all killing the file. The `@param`-of-Rational sidecar path is UNAFFECTED: it desugars
            // via a `pragma param`, never through `@test`/proptest synthesis. See the spec 26-runtime-params
            // ruling — a heap Rational has no host boundary form.)
            None if is_compound_form
                || is_ungeneratable_concrete_scalar(ast, ty)
                || name_resolves_to_user_type(ast, ty, items) =>
            {
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
            // A non-generatable bare name that is NOT a known concrete scalar AND does NOT resolve to a user
            // `(type …)` declaration (an unresolvable/ambiguous name like `Nonexistent`, or an inference-typed
            // param) — NOT ours. Returning None leaves the genuine type error to the boundary/layout (CDZ0101
            // "unknown type" / "ambiguous — annotate it"), which is the actionable diagnosis; masking it as a
            // per-test decline would hide a real mistake. (A bare name that DOES resolve to a user `(type …)`
            // whose shape the generator can't produce — a recursive sum, a multi-payload variant, or a mixed
            // nullary+payload sum classify_sum declines — is caught by the `name_resolves_to_user_type` guard
            // above: it gets a DECLINING wrapper so the sibling tests still run, rather than escaping to the
            // boundary and aborting the WHOLE `cdz test` file. Symmetric with the compound-form leaf case.)
            None => return None,
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

/// Is `ty` a BARE-NAME reference to a KNOWN CONCRETE scalar type that the generator cannot produce — a
/// heap/non-boundary scalar (`Char`, `Rational`, `BigInt`, `String`, `Symbol`)? Such a param has no
/// `<gen:T>` and no host boundary form, so a `@test` over it must DECLINE CLEANLY per-test (a nullary
/// declining wrapper) rather than abort the whole file at the export boundary. This is deliberately narrow:
/// it matches ONLY a resolved concrete scalar name, so a genuinely UNKNOWN name (`Nonexistent`) or an
/// ambiguous inference-typed param is NOT captured here — that stays a real boundary/layout type error
/// (CDZ0101), which is the actionable diagnosis a user needs, not a masked per-test decline. The
/// boundary-CROSSING scalars (`Bool`/`Float`/the int widths) never reach here: `classify_ty` returns
/// `Some` for them, so this is only consulted on the `None` (non-generatable) arm.
fn is_ungeneratable_concrete_scalar(ast: &Arenas, ty: StructId) -> bool {
    let Some(n) = ast.as_name(ty) else {
        return false;
    };
    matches!(
        crate::resolved::Prim::from_name(n),
        Some(
            crate::resolved::Prim::CharTy
                | crate::resolved::Prim::RationalTy
                | crate::resolved::Prim::BigIntTy
                | crate::resolved::Prim::StringTy
                | crate::resolved::Prim::SymbolTy
        )
    )
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
    // RE-APPLY the param-level match-based `@requires` constraints on the DECODE side too, matching the
    // generation-side (`plan_for_item`): drop the forbidden variants AND narrow a kept ctor's Int payload to
    // its recognized range, so the decoder consumes the pool with the SAME `% k` selector arity + the SAME
    // in-range payload the generator drew. WITHOUT this, decode re-classifies over the FULL/unconstrained set
    // and the `% k` selector or payload width desyncs → a wrong-variant / raw-int counterexample render.
    if let (GenTy::Sum { variants, .. }, Some(pname)) = (
        &mut gt,
        db.ast
            .as_form(param, ":")
            .and_then(|t| t.first())
            .and_then(|&n| db.ast.as_name(n)),
    ) {
        constrain_sum_variants(&db.ast, variants, db.requires_of(def), pname);
    }
    // RE-APPLY the param-level `@requires` scalar RANGE on the DECODE side, matching the generation-side
    // narrowing (`plan_for_item`): a scalar `Int` wrapper param under a recognized `@requires(k >= 0)` becomes
    // an `IntRange`, so the decoder maps the shrunk pool int into the SAME window the generator drew — a bare
    // `Int` decode would render the raw out-of-window pool int for a failing counterexample.
    if let (GenTy::Int, Some(pname)) = (
        &gt,
        db.ast
            .as_form(param, ":")
            .and_then(|t| t.first())
            .and_then(|&n| db.ast.as_name(n)),
    ) && let Some((lo, hi)) = db
        .requires_of(def)
        .iter()
        .find_map(|&q| int_range_over(&db.ast, q, pname))
    {
        gt = GenTy::IntRange { lo, hi };
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
            // A record-TYPE field is EITHER the canonical `(: name T)` ascription (the shared binder node,
            // DESIGN-record-type-syntax Phase A / RT3) OR the legacy `(name T)` head-app pair. Read both to
            // `(name_occ, ty_occ)`, mirroring the widening `reduce_ctor`/`decode_ty` (eval.rs/resolve.rs) took
            // on trunk 797e06857: this recognizer is a CONSUMER of record-type syntax, so it must accept the
            // ascription BEFORE the encoder flips to emit it (OQ-C) — else a `(: f T)` field fails the pair
            // check → this arm returns `None` → a record we should generate is silently DECLINED (a coverage
            // regression). Strictly widening: an ascription previously failed the `len == 2` check and returned
            // `None`, so no currently-classified input changes. (`field_pair` name kept so it does not SHADOW
            // the top-level `items` param this arm's recursion threads — PR #419.)
            let (name_occ, ty_occ) = if let Some(asc) = ast.as_form(field, ":") {
                match asc {
                    [name, t] => (*name, *t),
                    _ => return None,
                }
            } else {
                match ast.get(field) {
                    crate::ast::Struct::List(field_pair) if field_pair.len() == 2 => {
                        (field_pair[0], field_pair[1])
                    }
                    _ => return None,
                }
            };
            let fname = ast.as_name(name_occ)?.to_string();
            let fty = classify_ty_at(ast, ty_occ, items, depth + 1)?;
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
    int_range_over(ast, pred, crate::invariant_establish::VALUE_BINDER)
}

/// Recognize an inclusive integer RANGE `[lo, hi]` from a predicate over the BINDER name `binder` — the
/// binder-parameterized core of [`invariant_int_range`]. `invariant_int_range` calls this with the
/// operator-canonical `@invariant` value binder (`self`); the sum-payload-guard recognizer calls it with a
/// match-arm's PATTERN BINDER (e.g. `n` in `((Opt.Some n) (>= n 0))`), so a guard body that bounds the
/// payload int maps to an [`GenTy::IntRange`] on that constructor's payload. Same shape vocabulary as before
/// — `(and (>= b LO) (<= b HI))`, mirrors, a lone bound, `(= b K)`, with one-sided bounds closed by
/// [`ONE_SIDED_INVARIANT_WINDOW`] and a contradictory two-sided range (`lo > hi`) rejected → unconstrained.
fn int_range_over(ast: &Arenas, pred: StructId, binder: &str) -> Option<(i64, i64)> {
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
            let it_is = |n: StructId| ast.as_name(n) == Some(binder);
            let lit = |n: StructId| ast.as_int(n).and_then(|v| v.to_i64());
            // `(op self LIT)` or the mirror `(op LIT self)` — normalize to `self OP' LIT`.
            let (val, mirrored) = if it_is(t[0]) {
                (lit(t[1])?, false)
            } else if it_is(t[1]) {
                (lit(t[0])?, true)
            } else {
                // A comparison about ANOTHER binder (`(>= b 100)` while recognizing `a`) — SKIP it, do not
                // abandon the whole predicate. A multi-param `@requires` conjoins each param's bounds in one
                // predicate (`(and (and (>= a 0) (<= a 9)) (and (>= b 100) (<= b 109)))`); `int_range_over` is
                // called once per param, and each call must ignore the OTHER params' conjuncts rather than
                // `return None` on the first foreign one (which discarded THIS binder's bounds → the param was
                // drawn unconstrained → spurious (D)-trap). A comparison about `binder` with a NON-literal
                // operand still bails via the `lit(…)?` above (genuinely unrecognizable for this binder →
                // conservative unconstrained fallback, per the documented relational-bound limitation).
                break;
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

/// The set of SUM-CONSTRUCTOR names a match-based `@requires` on `param` FORBIDS — i.e. the precondition is
/// `(match param (PAT BODY) …)` and an arm dispatching on constructor `C` has body literal `false`, meaning
/// "a value built with `C` violates the precondition". Returns the forbidden constructor SHORT names (e.g.
/// `"None"` from a `(Opt.None)` pattern). The generator then declines to draw those constructors, so it never
/// produces a value the enforced (D) precondition rejects (a spurious `f(None)` failure). An arm whose body is
/// NOT literal `false` (a real guard like `(>= n 0)`) does NOT forbid its constructor — that arm's values may
/// still be valid, and a payload-level guard is not a constructor-level one (left to reject-free fallback).
/// Empty if `pred` is not a `(match param …)` over this param, or no arm bodies are literal `false`.
/// Constrain a `Sum` param's generator variants from the match-based `@requires` predicates over `pname`,
/// applied IDENTICALLY at generation (`plan_for_item`) and counterexample-decode (`gen_ty_of_wrapper_param`)
/// so the `% k` variant selector and the payload draw stay in sync between the two sides. Two constraints:
/// (1) DROP any constructor an arm forbids (body literal `false`) — unless that would empty the sum (an
/// unsatisfiable precondition keeps every variant so generation trips the honest (D) trap rather than
/// building an empty sum); (2) NARROW a kept constructor's `Int` payload to the [`GenTy::IntRange`] a
/// same-arm payload guard imposes (`((T.Some n) (>= n 0))`), so the drawn payload is in-domain and never a
/// spurious `Some(-1)`. A payload range only narrows an `Int` payload (a non-Int payload keeps its shape;
/// an unrecognized guard is left unconstrained — reject-free fallback, never wrong).
fn constrain_sum_variants(
    ast: &Arenas,
    variants: &mut Vec<(String, Option<GenTy>)>,
    preds: &[StructId],
    pname: &str,
) {
    let forbidden: std::collections::HashSet<String> = preds
        .iter()
        .flat_map(|&q| sum_ctors_forbidden_by_match(ast, q, pname))
        .collect();
    if !forbidden.is_empty() {
        let kept: Vec<_> = variants
            .iter()
            .filter(|(vname, _)| !forbidden.contains(vname))
            .cloned()
            .collect();
        if !kept.is_empty() {
            *variants = kept;
        }
    }
    // Narrow a KEPT constructor's Int payload to a recognized same-arm payload range. Keyed by ctor name so
    // it applies to whichever variant survived the forbidden filter.
    let ranges: std::collections::HashMap<String, (i64, i64)> = preds
        .iter()
        .flat_map(|&q| sum_ctor_payload_ranges(ast, q, pname))
        .collect();
    if !ranges.is_empty() {
        for (vname, payload) in variants.iter_mut() {
            if let (Some(&(lo, hi)), Some(GenTy::Int)) = (ranges.get(vname), payload.as_ref()) {
                *payload = Some(GenTy::IntRange { lo, hi });
            }
        }
    }
    // Floor a KEPT constructor's List payload to a recognized same-arm min-length guard (`((Box.Full xs) (< 0
    // (List.len xs)))` → the `Full` payload list is drawn non-empty). Same ctor-keyed application as the Int
    // range above; the max floor wins if several conjuncts/preds bound the same constructor.
    let min_lens: std::collections::HashMap<String, usize> = preds
        .iter()
        .flat_map(|&q| sum_ctor_payload_min_lens(ast, q, pname))
        .fold(std::collections::HashMap::new(), |mut acc, (c, m)| {
            let e = acc.entry(c).or_insert(0);
            *e = (*e).max(m);
            acc
        });
    if !min_lens.is_empty() {
        for (vname, payload) in variants.iter_mut() {
            if let (Some(&m), Some(GenTy::List(_, ml))) = (min_lens.get(vname), payload.as_mut()) {
                *ml = (*ml).max(m);
            }
        }
    }
}

/// The ARMS of every `(match param …)` precondition on `param`, SEEING THROUGH a top-level `(and …)`/`(& …)`
/// conjunction — the sum-match analogue of the conjunct descent in [`invariant_int_range`]/[`min_len_for_param`].
/// A `@requires` may spell the sum constraint bare (`(match o …)`) OR conjoined with other preconditions
/// (`(and (match o …) (>= k 0))` — a multi-param signature, or a sum constraint beside a scalar one), and it
/// may even carry SEVERAL match conjuncts on the same param whose constraints all apply. Without this descent
/// the three sum recognizers matched only a BARE `(match …)` and silently dropped the constraint inside an
/// `(and …)`, so the generator drew a forbidden constructor and the (D) precondition spuriously tripped (e.g.
/// `f(None)`). Returns the arms of ALL matching conjuncts CONCATENATED (an empty slice if none) — the callers
/// union across arms (a forbidden ctor from any arm is forbidden; a payload bound from any arm applies), so
/// collecting every match's arms is correct and order-independent. `arena` backs the returned slice.
fn match_arms_for_param<'a>(
    ast: &Arenas,
    pred: StructId,
    param: &str,
    arena: &'a mut Vec<StructId>,
) -> &'a [StructId] {
    // Descend a top-level `(and …)`/`(& …)` into its conjuncts (same vocabulary the scalar recognizers use),
    // collecting the arms of EVERY `(match param …)` conjunct on this exact param.
    let mut stack = vec![pred];
    while let Some(p) = stack.pop() {
        if let Some(t) = ast.as_form(p, "and").or_else(|| ast.as_form(p, "&")) {
            stack.extend(t.iter().copied());
            continue;
        }
        // A `(match SCRUT ARMS…)` on THIS param — head `match`, first tail item the scrutinee, rest the arms.
        if let Some(tail) = ast.as_form(p, "match")
            && let Some((&scrut, arms)) = tail.split_first()
            && ast.as_name(scrut) == Some(param)
        {
            arena.extend_from_slice(arms);
        }
    }
    arena.as_slice()
}

fn sum_ctors_forbidden_by_match(ast: &Arenas, pred: StructId, param: &str) -> Vec<String> {
    // The `(match param …)` arms, seeing through a top-level `(and …)` conjunction.
    let mut arena = Vec::new();
    let arms = match_arms_for_param(ast, pred, param, &mut arena);
    let mut forbidden = Vec::new();
    for &arm in arms {
        // An arm is `(PAT BODY)`: a 2-element list.
        let crate::ast::Struct::List(items) = ast.get(arm) else {
            continue;
        };
        let [pat, body] = items.as_slice() else {
            continue;
        };
        // The arm forbids its constructor only when its BODY is the literal `false` (a `Leaf::Bool`, not a
        // name — `as_bool`, not `as_name`).
        if ast.as_bool(*body) != Some(false) {
            continue;
        }
        // The PAT names a constructor: either `(. TYPE Ctor)` / `((. TYPE Ctor) binds…)` (member-access head)
        // or a bare `Ctor` name. Extract the short constructor name.
        if let Some(c) = ctor_name_of_pattern(ast, *pat) {
            forbidden.push(c);
        }
    }
    forbidden
}

/// The set of `(constructor, [lo,hi])` PAYLOAD RANGES a match-based `@requires` on `param` imposes — the
/// payload-level twin of [`sum_ctors_forbidden_by_match`]. Where that function drops a constructor whose arm
/// body is literal `false`, this recognizes an arm `((T.Ctor n) GUARD)` whose body is a recognized integer
/// RANGE over the single pattern BINDER `n` (e.g. `(>= n 0)`, `(and (>= n 0) (<= n 9))`), meaning "a value
/// built with `Ctor` is in-domain only when its payload satisfies GUARD". The generator then draws that
/// constructor's `Int` payload IN-RANGE rather than uniformly, so it never produces `Some(-1)` that the
/// enforced (D) precondition would reject as a spurious counterexample. Returns nothing for an arm whose body
/// is `false` (that's a forbidden CONSTRUCTOR — `sum_ctors_forbidden_by_match`'s job), `true`/an unrecognized
/// guard shape (opaque → unconstrained fallback, never wrong), or a pattern binding other than exactly one
/// name (a nullary or multi-bind pattern has no single payload binder to bound). Only a match on THIS param
/// constrains its generation.
fn sum_ctor_payload_ranges(ast: &Arenas, pred: StructId, param: &str) -> Vec<(String, (i64, i64))> {
    let mut arena = Vec::new();
    let arms = match_arms_for_param(ast, pred, param, &mut arena);
    let mut ranges = Vec::new();
    for &arm in arms {
        let crate::ast::Struct::List(items) = ast.get(arm) else {
            continue;
        };
        let [pat, body] = items.as_slice() else {
            continue;
        };
        // A literal-bool body is a constructor-level verdict (`false` = forbidden, `true` = allow-all), not a
        // payload bound — leave those to `sum_ctors_forbidden_by_match`.
        if ast.as_bool(*body).is_some() {
            continue;
        }
        // The pattern must bind EXACTLY one payload name (the int this guard bounds): `((T.Ctor n) …)`.
        let (Some(ctor), Some(binder)) = (
            ctor_name_of_pattern(ast, *pat),
            single_payload_binder(ast, *pat),
        ) else {
            continue;
        };
        if let Some((lo, hi)) = int_range_over(ast, *body, &binder) {
            ranges.push((ctor, (lo, hi)));
        }
    }
    ranges
}

/// The set of `(constructor, min_len)` LIST-LENGTH FLOORS a match-based `@requires` on `param` imposes — the
/// LIST-payload twin of [`sum_ctor_payload_ranges`] (which handles an Int payload). An arm `((Box.Full xs)
/// (< 0 (List.len xs)))` allows the `Full` constructor but requires its `(List …)` payload be non-empty, so
/// the generator must FLOOR that constructor's drawn list length rather than draw the empty list the enforced
/// (D) precondition rejects as a spurious `f(Full([]))`. Reuses [`min_len_for_param`] over the pattern's
/// single payload binder (the same recognizer the param-level and type-level min-length paths use), so the
/// vocabulary (`(< K (List.len xs))`, mirrors, conjunctions) is identical. Skips a `false`/`true` arm (a
/// constructor verdict), a nullary/multi-bind pattern, and an unrecognized guard (unconstrained fallback).
fn sum_ctor_payload_min_lens(ast: &Arenas, pred: StructId, param: &str) -> Vec<(String, usize)> {
    let mut arena = Vec::new();
    let arms = match_arms_for_param(ast, pred, param, &mut arena);
    let mut mins = Vec::new();
    for &arm in arms {
        let crate::ast::Struct::List(items) = ast.get(arm) else {
            continue;
        };
        let [pat, body] = items.as_slice() else {
            continue;
        };
        if ast.as_bool(*body).is_some() {
            continue;
        }
        let (Some(ctor), Some(binder)) = (
            ctor_name_of_pattern(ast, *pat),
            single_payload_binder(ast, *pat),
        ) else {
            continue;
        };
        if let Some(m) = min_len_for_param(ast, *body, &binder) {
            mins.push((ctor, m));
        }
    }
    mins
}

/// The single payload BINDER name of a constructor pattern `((. TYPE Ctor) n)` / `(Ctor n)` → `n`. `None` if
/// the pattern is nullary (`(. TYPE Ctor)` / bare `Ctor`), binds more than one payload, or binds a non-name
/// (a nested pattern) — those have no single int binder for a payload range to bound.
fn single_payload_binder(ast: &Arenas, pat: StructId) -> Option<String> {
    let crate::ast::Struct::List(items) = ast.get(pat) else {
        return None; // a bare name / member-access — nullary, no payload bind
    };
    // `[ HEAD, bind ]` — HEAD is the constructor (member-access or bare name), the one tail item the binder.
    let [_, bind] = items.as_slice() else {
        return None;
    };
    ast.as_name(*bind).map(str::to_string)
}

/// The short constructor name a match PATTERN dispatches on: `(. TYPE Ctor)` → `Ctor`; a payload pattern
/// `((. TYPE Ctor) x …)` → `Ctor` (the head is the member access); a bare `Ctor` name → `Ctor`. `None` for a
/// wildcard/binding/other pattern (which doesn't name a single constructor).
fn ctor_name_of_pattern(ast: &Arenas, pat: StructId) -> Option<String> {
    // A `(. TYPE Ctor)` member access directly.
    if let Some(m) = ast.as_form(pat, ".")
        && m.len() == 2
    {
        return ast.as_name(m[1]).map(str::to_string);
    }
    // A payload pattern `(HEAD binds…)` — recurse on HEAD (the constructor position).
    if let crate::ast::Struct::List(items) = ast.get(pat)
        && let Some(&head) = items.first()
    {
        return ctor_name_of_pattern(ast, head);
    }
    // A bare constructor name.
    ast.as_name(pat).map(str::to_string)
}

/// True if `ty` is a BARE NAME that resolves to a user `(type NAME …)` declaration in `items` (seeing
/// through an annotation wrapper, like `classify_sum`). Used by `plan_for_item` to distinguish a param
/// typed by a KNOWN user type whose shape the generator can't produce (a recursive sum, a multi-payload
/// variant, or a mixed nullary+payload sum classify_sum declines) — which should get a DECLINING wrapper
/// so siblings survive — from a genuinely-unresolvable/inference-typed name, which should keep its CDZ0101
/// boundary diagnostic. Without this, a `@test def p(x: T)` over such a `T` fell through to the boundary
/// and ABORTED THE WHOLE `cdz test` file, killing every sibling test — the exact failure the
/// declining-wrapper mechanism prevents for a non-generatable COMPOUND-FORM leaf (`(List Char)`).
fn name_resolves_to_user_type(ast: &Arenas, ty: StructId, items: &[StructId]) -> bool {
    let Some(type_name) = ast.as_name(ty) else {
        return false; // not a bare name — a compound FORM is handled by `is_compound_form`
    };
    items.iter().copied().any(|it| {
        type_decl_form(ast, it).is_some_and(|tail| {
            tail.first()
                .is_some_and(|&n| ast.type_decl_head_name(n) == Some(type_name))
        })
    })
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
                .is_some_and(|&n| ast.type_decl_head_name(n) == Some(type_name))
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
        // A variant is either a bare NAME (a nullary variant — `B` in `type T = A(Int64) | B`, which the
        // ML surface lowers to a bare-name sexpr atom, NOT a `(B)` list) or a list `(VNAME PAYLOAD?)`. A
        // bare-name nullary variant is generatable (the selector just picks it; no payload to draw), so it
        // must NOT decline the whole sum — else a MIXED payload+nullary sum (or a plain all-nullary enum
        // like `Red | Green | Blue`) fails to generate at all, when both are common, fully-generatable
        // shapes. (`build_sum_gen` already emits a nullary variant as the bare ctor `(. TYPE V)`.)
        if let Some(vname) = ast.as_name(vf) {
            variants.push((vname.to_string(), None));
            continue;
        }
        let vitems = match ast.get(vf) {
            crate::ast::Struct::List(v) if !v.is_empty() => v.as_slice(),
            _ => return None,
        };
        let vname = ast.as_name(vitems[0])?.to_string();
        let payload = match vitems.get(1) {
            None => None, // nullary variant written in list form `(V)`
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
                Leaf::Str(
                    format!(
                        "{}: a parameter's type has no property-test-generatable form yet \
                     (Char/Rational/BigInt/String/Symbol, a compound with such a leaf, or an empty \
                     (Tuple)/(Record) with nothing to generate) — not property-testable; use a \
                     boundary-representable type or drop the @test",
                        plan.def_name
                    )
                    .into(),
                ),
            );
            push_list(ast, vec![member, msg])
        };
        let trap = {
            let t = name(ast, "trap");
            let m = push_atom(ast, Leaf::Str("not property-testable".into()));
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
        // `(record (= f <gen:T>) …)` — one generated value per named field, each a canonical `(= name value)`
        // ascription triple (record-type-syntax Phase B, trunk ab42bfb83: record fields spell `(= name value)`
        // in EVERY position — literal, pattern, value-output — for read==print symmetry). The readers still
        // TOLERATE the legacy `(name value)` pair, but the generator emits the canonical triple so a synthesized
        // record value prints back byte-identically AND stays valid if a later phase drops legacy-pair tolerance
        // (mirrors the RT3 type-field widen this vertical landed ahead of its encode flip).
        GenTy::Record(fields) => {
            let head = name(ast, "record");
            let mut children = vec![head];
            for (fname, fty) in fields {
                let fval = build_gen(ast, fty, binds);
                let fnm = name(ast, fname);
                let eq = name(ast, "=");
                let triple = push_list(ast, vec![eq, fnm, fval]);
                children.push(triple);
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
        // A VARIABLE-cardinality set (`0..=G1_LIST_LEN` distinct elements) via a `Set.insert` fold over the
        // constant empty set — so the EMPTY + singleton sets are reachable (a fixed 3-element `Set.of (list …)`
        // never reached them for a wide element type). See `build_var_set_gen`.
        GenTy::Set(elem) => build_var_set_gen(ast, elem, binds),
        // A VARIABLE-size map (`0..=G1_LIST_LEN` entries) via a `Map.insert` fold over `(Map.empty)` — so the
        // EMPTY + small maps are reachable (a fixed `G1_LIST_LEN`-insert fold never reached them for a wide key
        // type — keys never collide, so the map was always exactly `G1_LIST_LEN` entries). See
        // `build_var_map_gen`. The Map analogue of the variable-cardinality Set fix.
        GenTy::Map(kty, vty) => build_var_map_gen(ast, kty, vty, binds),
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

/// Build a VARIABLE-cardinality `Set` generator (`0..=G1_LIST_LEN` distinct elements) — the Set analogue of
/// [`build_var_list_gen`]. A fixed-cardinality `(Set.of (list e0 e1 e2))` made the EMPTY + singleton sets
/// unreachable for a wide element type (Int64 never collides), so a "Set is never empty" / "Set.len < 3"
/// property spuriously passed. This draws a count `c` (like the var-list gen) then folds `c` `Set.insert`s over
/// the CONSTANT empty set `(Set.of (list))` — `Set.insert` exists in the prelude and dedups, so the result is
/// a set of AT MOST `c` elements (a collision yields a smaller one, same as `Set.of`). `(Set.of (list))` is a
/// constant-list literal (the ONLY list shape `Set.of` accepts), so the seed compiles; the fold builds the
/// variable part. Mirrors the Map generator's `Map.insert`-over-`Map.empty` fold, but with a variable count.
fn build_var_set_gen(
    ast: &mut Arenas,
    elem: &GenTy,
    binds: &mut Vec<(StructId, StructId)>,
) -> StructId {
    let span = (G1_LIST_LEN + 1) as i64;
    // Hoist the count `c = (% (& gen i64::MAX) (LEN+1))` ∈ [0, LEN] — same non-negative-mask derivation as the
    // var-list count (a raw negative `%` would make `c` negative and the `(<= c 0)` guard pick the empty set).
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
        let var = name(ast, &count_name);
        binds.push((var, modded));
    }
    // Hoist the LEN candidate elements (each its own `gN`), so the prefix sets reference names, never sharing
    // an expression node across parents. Same ordering as `build_var_list_gen`, so the DECODER (which draws the
    // count then LEN elements then dedups a `c`-prefix) stays in lockstep.
    let elem_names: Vec<String> = (0..G1_LIST_LEN)
        .map(|_| {
            let e = build_gen(ast, elem, binds);
            let nm = format!("g{}", binds.len());
            let var = name(ast, &nm);
            binds.push((var, e));
            nm
        })
        .collect();
    // The constant empty set `(Set.of (list))` — the fold seed (a constant-list literal, so `Set.of` accepts).
    let empty_set = |ast: &mut Arenas| -> StructId {
        let set_of = {
            let dot = name(ast, ".");
            let set = name(ast, "Set");
            let of = name(ast, "of");
            push_list(ast, vec![dot, set, of])
        };
        let empty_list = {
            let list_head = name(ast, "list");
            push_list(ast, vec![list_head])
        };
        push_list(ast, vec![set_of, empty_list])
    };
    // Build the prefix SETS: prefix0 = `(Set.of (list))`, prefix_k = `(Set.insert … (Set.insert (Set.of (list))
    // e0) …) e_{k-1}` — `k` nested inserts of the first `k` candidate elements.
    let prefixes: Vec<StructId> = (0..=G1_LIST_LEN)
        .map(|k| {
            let mut acc = empty_set(ast);
            for enm in elem_names.iter().take(k) {
                let insert = {
                    let dot = name(ast, ".");
                    let set = name(ast, "Set");
                    let ins = name(ast, "insert");
                    push_list(ast, vec![dot, set, ins])
                };
                let e = name(ast, enm);
                acc = push_list(ast, vec![insert, acc, e]);
            }
            acc
        })
        .collect();
    // Fold into `(if (<= c 0) prefix0 (if (<= c 1) prefix1 … prefixLEN))` — the last prefix is the else.
    let mut chain = prefixes[G1_LIST_LEN];
    for k in (0..G1_LIST_LEN).rev() {
        let cond = {
            let le = name(ast, "<=");
            let c_use = name(ast, &count_name);
            let iv = int_lit(ast, k as i64);
            push_list(ast, vec![le, c_use, iv])
        };
        let if_head = name(ast, "if");
        chain = push_list(ast, vec![if_head, cond, prefixes[k], chain]);
    }
    chain
}

/// Build a VARIABLE-size `Map` generator (`0..=G1_LIST_LEN` entries) — the Map analogue of
/// [`build_var_set_gen`]. A fixed-`G1_LIST_LEN`-insert fold made the EMPTY + small maps unreachable for a wide
/// key type (keys never collide → always exactly `G1_LIST_LEN` entries), so a "Map is never empty" property
/// spuriously passed. This draws a count `c` then folds `c` `Map.insert`s of the first `c` candidate key/value
/// pairs over `(Map.empty)` — same variable-count if-chain as `build_var_set_gen`, over the Map's existing
/// `Map.empty`/`Map.insert` prelude ops. Last-write-wins on a repeated key yields a smaller map (as before).
fn build_var_map_gen(
    ast: &mut Arenas,
    kty: &GenTy,
    vty: &GenTy,
    binds: &mut Vec<(StructId, StructId)>,
) -> StructId {
    let span = (G1_LIST_LEN + 1) as i64;
    // Hoist the count `c = (% (& gen i64::MAX) (LEN+1))` ∈ [0, LEN] — same non-negative-mask derivation as the
    // var-set/var-list count (a raw negative `%` would make `c` negative → the `(<= c 0)` guard picks empty).
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
        let var = name(ast, &count_name);
        binds.push((var, modded));
    }
    // Hoist the LEN candidate key/value pairs, each its own `gN`, in draw order (key then value per pair) so the
    // DECODER (which draws the count then LEN (k,v) pairs then last-write-wins a `c`-prefix) stays in lockstep.
    let pair_names: Vec<(String, String)> = (0..G1_LIST_LEN)
        .map(|_| {
            let ke = build_gen(ast, kty, binds);
            let knm = format!("g{}", binds.len());
            let kvar = name(ast, &knm);
            binds.push((kvar, ke));
            let ve = build_gen(ast, vty, binds);
            let vnm = format!("g{}", binds.len());
            let vvar = name(ast, &vnm);
            binds.push((vvar, ve));
            (knm, vnm)
        })
        .collect();
    // The empty map `(Map.empty)` — the fold seed.
    let empty_map = |ast: &mut Arenas| -> StructId {
        let dot = name(ast, ".");
        let mapn = name(ast, "Map");
        let empty = name(ast, "empty");
        let member = push_list(ast, vec![dot, mapn, empty]);
        push_list(ast, vec![member])
    };
    // Build the prefix MAPS: prefix0 = `(Map.empty)`, prefix_k = `k` nested inserts of the first `k` pairs.
    let prefixes: Vec<StructId> = (0..=G1_LIST_LEN)
        .map(|k| {
            let mut acc = empty_map(ast);
            for (knm, vnm) in pair_names.iter().take(k) {
                let insert = {
                    let dot = name(ast, ".");
                    let mapn = name(ast, "Map");
                    let ins = name(ast, "insert");
                    push_list(ast, vec![dot, mapn, ins])
                };
                let kref = name(ast, knm);
                let vref = name(ast, vnm);
                acc = push_list(ast, vec![insert, acc, kref, vref]);
            }
            acc
        })
        .collect();
    // Fold into `(if (<= c 0) prefix0 (if (<= c 1) prefix1 … prefixLEN))` — the last prefix is the else.
    let mut chain = prefixes[G1_LIST_LEN];
    for k in (0..G1_LIST_LEN).rev() {
        let cond = {
            let le = name(ast, "<=");
            let c_use = name(ast, &count_name);
            let iv = int_lit(ast, k as i64);
            push_list(ast, vec![le, c_use, iv])
        };
        let if_head = name(ast, "if");
        chain = push_list(ast, vec![if_head, cond, prefixes[k], chain]);
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

    /// A record-TYPE whose fields use the CANONICAL `(: name T)` ascription spelling (DESIGN-record-type-
    /// syntax Phase A / RT3) — NOT the legacy `(name T)` head-app pair — must still classify as a `Record`
    /// and gain its `-gen` wrapper. `classify_ty_at`'s Record arm reads BOTH spellings (mirroring the widening
    /// `reduce_ctor`/`decode_ty` took), so this recognizer accepts the ascription BEFORE the encoder flips to
    /// emit it (OQ-C) — without this arm the `(: n T)` field fails the `len == 2` pair check → the record is
    /// silently DECLINED (a coverage regression). DISCRIMINATING: the fields are ascription-spelled (`(: x
    /// Int64)`), so a recognizer that only read the legacy pair would produce NO wrapper here. Nested +
    /// user-sum-field faces too, so the both-spellings read holds through the field-classify recursion.
    #[test]
    fn a_record_type_with_ascription_spelled_fields_still_generates() {
        for (src, def, wrapper) in [
            (
                "(do (@ test (def (r (: v (Record (: x Int64) (: y Bool)))) 0)) (def (o) 1))",
                "r",
                "r-gen",
            ),
            (
                "(do (@ test (def (lr (: xs (List (Record (: a Int64) (: b Bool)))))  (List.len xs))) (def (o) 1))",
                "lr",
                "lr-gen",
            ),
            // A user-sum field under the ascription spelling — the field-classify recursion must keep passing
            // the TOP-LEVEL `items` so `Ty` resolves the `(type …)` decl, exactly as the legacy-pair face does.
            (
                "(do (type Ty (A Int64) (B Bool)) \
                   (@ test (def (rs (: v (Record (: t Ty) (: n Int64)))) 0)) (def (o) 1))",
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
                "{def} (ascription-spelled record fields): expected wrapper {wrapper}, got {names:?}"
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

    /// A SCALAR `Int` param that rides inside a synthesized wrapper (because a SIBLING param is compound) is
    /// narrowed to an `IntRange` by a param-level `@requires` bound — was drawn UNCONSTRAINED, so `@requires(k
    /// >= 0)` on `f(xs: List Int64, k: Int64)` let the wrapper draw `k < 0` and the enforced (D) precondition
    /// spuriously tripped. Pins that `plan_for_item` narrows the scalar leaf (not only the List/Sum payloads).
    /// The compound sibling (`xs`) forces the wrapper; the scalar (`k`) must come out `IntRange`, not `Int`.
    #[test]
    fn a_scalar_wrapper_param_is_narrowed_by_a_param_level_requires_range() {
        let ast = crate::testkit::parse(
            "(do (@ test (@ (requires (and (>= k 0) (<= k 9))) \
               (def (f (: xs (List Int64)) (: k Int64)) (List.len xs)))) (def (o) 1))",
        );
        let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
        let ann = *items
            .iter()
            .find(|&&it| ast.as_form(it, "@").is_some())
            .expect("the @test item");
        let plan =
            super::plan_for_item(&ast, ann, &items).expect("a wrapper plan (xs is compound)");
        // gen_tys is [List(Int, 0), IntRange{0,9}] — the scalar `k` narrowed, the List sibling untouched.
        match plan.gen_tys.as_slice() {
            [
                super::GenTy::List(..),
                super::GenTy::IntRange { lo: 0, hi: 9 },
            ] => {}
            other => panic!("expected [List, IntRange{{0,9}}] for (xs, k), got {other:?}"),
        }
    }

    /// The dual guard: an ALL-SCALAR signature with a `@requires` range must NOT synthesize a wrapper — the
    /// boundary-arg route generates each scalar. A narrowed `IntRange` scalar stays boundary-representable, so
    /// `plan_for_item` returns `None` (no compound param), leaving the def to the runner's `--arg` generation.
    #[test]
    fn an_all_scalar_requires_range_signature_gets_no_wrapper() {
        let ast = crate::testkit::parse(
            "(do (@ test (@ (requires (and (>= k 10) (<= k 20))) \
               (def (f (: k Int64)) k))) (def (o) 1))",
        );
        let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
        let ann = *items
            .iter()
            .find(|&&it| ast.as_form(it, "@").is_some())
            .expect("the @test item");
        assert!(
            super::plan_for_item(&ast, ann, &items).is_none(),
            "an all-scalar @requires-range signature stays on the boundary route (no wrapper synthesized)"
        );
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

    /// `int_range_over` must SKIP a conjunct that compares a DIFFERENT binder, not abandon the whole predicate.
    /// A multi-param `@requires` conjoins each param's bounds in ONE predicate; the recognizer runs once per
    /// param, so recognizing `a`'s range must ignore `b`'s conjuncts. Before the fix, the first foreign conjunct
    /// hit `return None`, discarding `a`'s bounds → `a` drew unconstrained → spurious (D)-trap. Pins that each
    /// binder's bounds survive alongside another binder's, AND that a same-binder RELATIONAL (non-literal) bound
    /// still bails conservatively (the documented relational limitation) — the two must not be conflated.
    #[test]
    fn int_range_over_skips_a_cross_binder_conjunct() {
        // Extract the `@requires` predicate node, then hand it to the binder-parameterized recognizer.
        let pred_of = |src: &str| -> (crate::ast::Arenas, crate::ast::StructId) {
            let ast = crate::testkit::parse(src);
            let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
            let ann = *items
                .iter()
                .find(|&&it| ast.as_form(it, "@").is_some())
                .expect("an @ annotation");
            let head = ast.as_form(ann, "@").unwrap()[0];
            let pred = ast.as_form(head, "requires").expect("(requires Q)")[0];
            (ast, pred)
        };
        // `(and (and (>= a 0) (<= a 9)) (and (>= b 100) (<= b 109)))`: recognizing `a` yields [0,9] (b's
        // conjuncts skipped, not a bail), and recognizing `b` yields [100,109] (a's conjuncts skipped).
        let (ast, pred) = pred_of(
            "(do (@ (requires (and (and (>= a 0) (<= a 9)) (and (>= b 100) (<= b 109)))) \
               (def (f (: a Int64) (: b Int64)) 0)) (def (z) 1))",
        );
        assert_eq!(super::int_range_over(&ast, pred, "a"), Some((0, 9)));
        assert_eq!(super::int_range_over(&ast, pred, "b"), Some((100, 109)));
        // A binder with NO bound of its own among only-foreign conjuncts ⇒ None (unconstrained).
        assert_eq!(super::int_range_over(&ast, pred, "c"), None);
        // A RELATIONAL (non-literal) bound `(>= a b)` bails for BOTH the binders it mentions — it constrains
        // `a` (a >= b) and `b` (b <= a) but neither can be inverted to a LITERAL window (the documented
        // relational limitation), so each returns None → conservative unconstrained fallback (never wrong, may
        // fail honestly via the (D)-trap). A cross-binder LITERAL conjunct (the case above) is skipped; a
        // same-or-mentioned-binder NON-LITERAL bound bails. The two must stay distinct.
        let (ast2, pred2) = pred_of(
            "(do (@ (requires (and (>= a b) (>= a 0))) \
               (def (f (: a Int64) (: b Int64)) 0)) (def (z) 1))",
        );
        // `a` is mentioned in the non-literal `(>= a b)` → bails (relational, can't invert), even though it
        // also has a literal `(>= a 0)`: a non-literal bound on the binder is the conservative signal.
        assert_eq!(super::int_range_over(&ast2, pred2, "a"), None);
        // `b` appears ONLY in `(>= a b)` (as the non-literal operand of a bound naming `a` first) → for `b`
        // that conjunct is `(op a b)` with `it_is(t[1])` true and `lit(t[0])` = None → bails → None.
        assert_eq!(super::int_range_over(&ast2, pred2, "b"), None);
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
            // A sum whose variant PAYLOADS are themselves COMPOUNDS (a `(Tuple …)` and a `(List …)`), mixed
            // with a bare-name NULLARY variant — classify_sum recurses through each payload into the nested
            // compound (depth-bounded) AND accepts the nullary. A common real shape (a tagged message union);
            // exercises payload-recursion + mixed-arity together, beyond the scalar-payload cases above.
            (
                "(do (type Msg (Pair (Tuple Int64 Bool)) (Items (List Int64)) Empty) \
                   (@ test (def (m (: v Msg)) 0)) (def (o) 1))",
                "m",
                "m-gen",
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

    /// A sum with a BARE-NAME nullary variant generates. The ML surface lowers a nullary variant to a bare
    /// NAME atom, not a `(V)` list: `type T = A(Int64) | B` → sexpr `(type T (A Int64) B)`. classify_sum
    /// must accept the bare-name variant (`B`) as nullary, else a MIXED payload+nullary sum, a 3-variant
    /// mixed sum, AND a plain all-nullary enum (`Red | Green | Blue` → `(type C Red Green Blue)`) all fail
    /// to generate — common, fully-generatable shapes. A REAL generator wrapper is synthesized (not the
    /// declining one), and the original def is neutralized. (Previously only an all-payloaded sum, or one
    /// whose nullary variants happened to be written in `(V)` list form, generated — so the ML-authored
    /// mixed/enum sums the operator flagged declined.)
    #[test]
    fn a_sum_with_bare_name_nullary_variants_generates() {
        for (src, def, wrapper) in [
            // mixed payload + bare-name nullary
            (
                "(do (type T (A Int64) B) (@ test (def (p (: x T)) 0)) (def (o) 1))",
                "p",
                "p-gen",
            ),
            // 3-variant mixed, bare-name nullary
            (
                "(do (type T (Var Int64) (Con Bool) Nil) (@ test (def (p (: x T)) 0)) (def (o) 1))",
                "p",
                "p-gen",
            ),
            // plain all-nullary enum (every variant a bare name)
            (
                "(do (type C Red Green Blue) (@ test (def (p (: x C)) 0)) (def (o) 1))",
                "p",
                "p-gen",
            ),
        ] {
            // `Db::load` runs proptest_gen::synthesize and exposes the synthesized AST as `db.ast`, so the
            // wrapper's BODY is inspectable directly (no separate `super::synthesize`). Both a REAL generator
            // wrapper and a DECLINING wrapper are nullary and rename the def, so nullary+rename alone does
            // NOT discriminate; the trap-freedom of the body is what pins that classify_sum produced a REAL
            // generator (a `let`-chain, NO trap), not the declining fallback (`(do (Test.fail …) (trap …))`)
            // — Copilot PR#1199 review.
            let db = Db::load(crate::testkit::parse(src));
            let names: Vec<String> = db
                .test_defs()
                .into_iter()
                .map(|i| db.defs[i].name.clone())
                .collect();
            assert!(
                names.iter().any(|n| n == wrapper) && !names.iter().any(|n| n == def),
                "{def}: a bare-name-nullary sum generates a REAL wrapper {wrapper} (not declining), \
                 original neutralized; got {names:?}"
            );
            let w = db.defs.iter().find(|d| d.name == wrapper).unwrap();
            assert!(
                w.params.is_empty(),
                "the generator wrapper is nullary (generates the sum param internally): {wrapper}"
            );
            // DISCRIMINATING assertion: the real generator's body has NO `trap` form. A DECLINING wrapper
            // (what a NON-generatable sum would produce, and what this test would wrongly pass without the
            // classify_sum bare-name-nullary fix) traps. Walk the wrapper def's body subtree for any `trap`.
            let body = w.body.expect("the generator wrapper has a body");
            let mut stack = vec![body];
            let mut has_trap = false;
            while let Some(id) = stack.pop() {
                if db.ast.as_form(id, "trap").is_some() {
                    has_trap = true;
                    break;
                }
                if let crate::ast::Struct::List(kids) = db.ast.get(id) {
                    stack.extend(kids.iter().copied());
                }
            }
            assert!(
                !has_trap,
                "{wrapper}: a REAL sum generator's body must NOT trap (a DECLINING wrapper traps) — this \
                 discriminates the classify_sum bare-name-nullary fix from the declining fallback"
            );
        }
    }

    /// The synthesized RECORD generator emits each field as the CANONICAL `(= name value)` ascription triple
    /// (record-type-syntax Phase B, trunk ab42bfb83 — record fields spell `(= name value)` in every position
    /// for read==print symmetry), NOT the legacy `(name value)` pair. Readers still TOLERATE the pair, so a
    /// gate/roundtrip would pass either way — this DISCRIMINATES the emit form directly by walking the wrapper
    /// body for a `(record …)` and asserting EVERY field child is an `(= …)` form (head `=`), none a bare pair.
    /// Guards against my generator drifting behind the canonical spelling (the same stay-ahead-of-the-flip
    /// discipline as the RT3 record-TYPE-field widen).
    #[test]
    fn the_synthesized_record_generator_emits_the_ascription_triple_per_field() {
        // Record-TYPE fields use the pair/`:` spelling (RT3); the check below is on the emitted VALUE literal,
        // which Phase B canonicalizes to `(= name value)`.
        let src = "(do (@ test (def (r (: v (Record (x Int64) (y Bool)))) 0)) (def (o) 1))";
        let db = Db::load(crate::testkit::parse(src));
        let w = db
            .defs
            .iter()
            .find(|d| d.name == "r-gen")
            .expect("the record test synthesizes an r-gen wrapper");
        let body = w.body.expect("the generator wrapper has a body");
        // Find the `(record …)` literal the generator builds, then assert each field child is `(= name value)`.
        let mut stack = vec![body];
        let mut checked_a_record = false;
        while let Some(id) = stack.pop() {
            if let Some(rec_tail) = db.ast.as_form(id, "record") {
                checked_a_record = true;
                for &field in rec_tail {
                    let asc = db.ast.as_form(field, "=");
                    assert!(
                        asc.map(|a| a.len() == 2).unwrap_or(false),
                        "each synthesized record field must be the canonical `(= name value)` triple, \
                         not a legacy `(name value)` pair; offending field is not `(= _ _)`"
                    );
                }
            }
            if let crate::ast::Struct::List(kids) = db.ast.get(id) {
                stack.extend(kids.iter().copied());
            }
        }
        assert!(
            checked_a_record,
            "the r-gen wrapper body must build at least one `(record …)` literal to check"
        );
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
        // A recursive sum (unbounded generator) declines at the depth guard → classify_sum returns None.
        // Because `Tree` RESOLVES to a user `(type …)`, `plan_for_item` gives it a DECLINING wrapper
        // (`tr-gen`, trapping nullary; original `tr` neutralized) so the runner reports a per-test FAIL
        // while a sibling still runs — rather than escaping to the export boundary and ABORTING the whole
        // `cdz test` file (which is what "no wrapper" meant at the CLI level: `a Tree sum crosses the host
        // boundary only as a single nullary export's result`, exit 1, siblings killed). Without hanging.
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
            names.iter().any(|n| n == "tr-gen") && !names.iter().any(|n| n == "tr"),
            "a recursive sum gets a DECLINING wrapper (tr-gen), original tr neutralized — siblings survive \
             instead of a file-abort: {names:?}"
        );
        let tr_gen = db
            .defs
            .iter()
            .find(|d| d.name == "tr-gen")
            .expect("tr-gen def exists");
        assert!(
            tr_gen.params.is_empty(),
            "the declining wrapper is nullary (neutralizes the sum param → never hits the boundary)"
        );
    }

    /// `classify_sum` models a variant as `(VNAME PAYLOAD?)` — zero or ONE payload occurrence (several
    /// fields are a single `(Tuple …)`/`(Record …)` payload). A variant with TWO+ payload occurrences
    /// (`(Var Int64 Bool)`) is not that shape, so the whole sum declines (classify_sum → None). Because
    /// `Bad` RESOLVES to a user `(type …)`, `plan_for_item` gives it a DECLINING wrapper (`b-gen`, trapping
    /// nullary; original `b` neutralized) so the runner reports a per-test FAIL while a sibling still runs
    /// — rather than escaping to the export boundary and ABORTING the whole `cdz test` file (what "no
    /// wrapper" meant at the CLI: `a Bad sum crosses the host boundary…`, exit 1, siblings killed).
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
            names.iter().any(|n| n == "b-gen") && !names.iter().any(|n| n == "b"),
            "a multi-payload variant gets a DECLINING wrapper (b-gen), original b neutralized — siblings \
             survive instead of a file-abort: {names:?}"
        );
        let b_gen = db
            .defs
            .iter()
            .find(|d| d.name == "b-gen")
            .expect("b-gen def exists");
        assert!(
            b_gen.params.is_empty(),
            "the declining wrapper is nullary (neutralizes the sum param → never hits the boundary)"
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
        let ast = crate::testkit::parse(
            "(do (@ test (def (r (: xs (List Char))) (List.len xs))) (def (other) 1))",
        );
        // `Db::load` runs proptest_gen::synthesize, producing the declining `r-gen` wrapper (a trapping
        // nullary def) and neutralizing the original `r`; the assertions below inspect `db.defs` for it.
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

    /// A `@test` over a BARE-NAME non-generatable concrete scalar (`Rational` — a heap scalar with no host
    /// boundary form, per spec 26-runtime-params) gets the SAME declining wrapper: `p-gen` is synthesized
    /// (a trapping nullary def), the original `p` neutralized. Before this, such a param fell through to the
    /// boundary and ABORTED THE WHOLE `cdz test` file (killing sibling tests) — at layout for `Char`
    /// (valtype None) or at serialize for `Rational`/`BigInt`/`String`/`Symbol` (valtype = a heap handle).
    /// Now it declines per-test, symmetric with the non-generatable-leaf COMPOUND case above.
    #[test]
    fn a_bare_name_nongeneratable_scalar_param_gets_a_declining_wrapper() {
        let ast = crate::testkit::parse(
            "(do (@ test (def (p (: r Rational)) (if (= r r) unit (trap \"neq\")))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            test_names.iter().any(|n| n == "p-gen") && !test_names.iter().any(|n| n == "p"),
            "a bare-name non-generatable scalar (Rational) gets a DECLINING wrapper (p-gen), original p \
             neutralized: {test_names:?}"
        );
        let p_gen = db
            .defs
            .iter()
            .find(|d| d.name == "p-gen")
            .expect("p-gen def exists");
        assert!(
            p_gen.params.is_empty(),
            "the declining wrapper is nullary (neutralizes the scalar param → never hits the boundary)"
        );
    }

    /// The declining path is NARROW: a genuinely UNKNOWN bare-name type (`Nonexistent`) must NOT be masked
    /// as a per-test decline — it stays a real type error (no `-gen` wrapper synthesized), so the boundary/
    /// layout reports the actionable CDZ0101 "unknown type". `plan_for_item` returns None for it (not our
    /// concern), leaving the diagnosis intact. Guards against `is_ungeneratable_concrete_scalar` over-matching.
    #[test]
    fn an_unknown_bare_name_type_param_is_not_masked_as_a_declining_wrapper() {
        let ast = crate::testkit::parse(
            "(do (@ test (def (p (: x Nonexistent)) (if (= x x) unit (trap \"neq\")))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            !test_names.iter().any(|n| n == "p-gen"),
            "an unknown-type param gets NO declining wrapper — the real type error is left to the boundary: \
             {test_names:?}"
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

    /// `sum_ctors_forbidden_by_match` recognizes a match-based `@requires` and returns the constructors whose
    /// arm body is the literal `false` (forbidden by the precondition). An arm with a real guard body (not
    /// `false`) is NOT forbidden; a match on a DIFFERENT param constrains nothing.
    #[test]
    fn sum_ctors_forbidden_by_match_reads_false_arms() {
        // Extract the `@requires` predicate node from a fixture, then hand it to the recognizer.
        let forbidden_of = |src: &str, param: &str| -> Vec<String> {
            let ast = crate::testkit::parse(src);
            // The requires predicate is the first child of the `(requires Q)` head of the `(@ (requires …) …)`.
            let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
            let ann = *items
                .iter()
                .find(|&&it| ast.as_form(it, "@").is_some())
                .expect("an @ annotation");
            let layer = ast.as_form(ann, "@").unwrap();
            let head = layer[0];
            let pred = ast.as_form(head, "requires").expect("(requires Q)")[0];
            super::sum_ctors_forbidden_by_match(&ast, pred, param)
        };
        // `None -> false` forbids `None`; the `Some` arm (guard body) is not forbidden.
        assert_eq!(
            forbidden_of(
                "(do (type Opt (None) (Some Int64)) \
                 (@ (requires (match o ((Opt.Some n) (>= n 0)) ((Opt.None) false))) \
                   (def (f (: o Opt)) 0)) (def (z) 1))",
                "o"
            ),
            vec!["None".to_string()]
        );
        // Both a `false` arm AND a `true` arm: only the `false` one is forbidden.
        assert_eq!(
            forbidden_of(
                "(do (type T (A) (B) (C Int64)) \
                 (@ (requires (match o ((T.A) false) ((T.B) true) ((T.C n) (>= n 0)))) \
                   (def (f (: o T)) 0)) (def (z) 1))",
                "o"
            ),
            vec!["A".to_string()]
        );
        // A match on a DIFFERENT param than the one asked about constrains nothing.
        assert!(
            forbidden_of(
                "(do (type Opt (None) (Some Int64)) \
                 (@ (requires (match o ((Opt.None) false) ((Opt.Some n) true))) \
                   (def (f (: o Opt) (: p Opt)) 0)) (def (z) 1))",
                "p"
            )
            .is_empty()
        );
        // SEEING THROUGH a top-level `(and …)`: the match constraint is recognized even when conjoined with a
        // scalar precondition on another param (a multi-param signature) — was silently dropped before the
        // conjunct descent (`match_arms_for_param`), causing a spurious `f(None)`.
        assert_eq!(
            forbidden_of(
                "(do (type Opt (None) (Some Int64)) \
                 (@ (requires (and (match o ((Opt.Some n) true) ((Opt.None) false)) (>= k 0))) \
                   (def (f (: o Opt) (: k Int64)) 0)) (def (z) 1))",
                "o"
            ),
            vec!["None".to_string()]
        );
        // TWO match conjuncts on the same param union their forbidden constructors (order-independent — sort
        // before comparing, since conjunct traversal order is not part of the contract).
        let mut both = forbidden_of(
            "(do (type T (A) (B) (C Int64)) \
             (@ (requires (and (match o ((T.A) false) ((T.B) true) ((T.C n) true)) \
                               (match o ((T.A) true) ((T.B) false) ((T.C n) true)))) \
               (def (f (: o T)) 0)) (def (z) 1))",
            "o",
        );
        both.sort();
        assert_eq!(both, vec!["A".to_string(), "B".to_string()]);
    }

    /// `sum_ctor_payload_ranges` is the PAYLOAD-level twin of `sum_ctors_forbidden_by_match`: an arm whose
    /// body is a recognized integer range over the pattern's single payload binder (`((Opt.Some n) (>= n 0))`)
    /// yields `(Ctor, [lo,hi])`, so the generator draws that constructor's payload IN-RANGE (no spurious
    /// `Some(-1)`). A `false`/`true` arm body is a CONSTRUCTOR verdict, not a payload range; an unrecognized
    /// guard, a nullary/multi-bind pattern, or a match on another param yields nothing.
    #[test]
    fn sum_ctor_payload_ranges_reads_guard_arms() {
        let ranges_of = |src: &str, param: &str| -> Vec<(String, (i64, i64))> {
            let ast = crate::testkit::parse(src);
            let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
            let ann = *items
                .iter()
                .find(|&&it| ast.as_form(it, "@").is_some())
                .expect("an @ annotation");
            let head = ast.as_form(ann, "@").unwrap()[0];
            let pred = ast.as_form(head, "requires").expect("(requires Q)")[0];
            super::sum_ctor_payload_ranges(&ast, pred, param)
        };
        // A one-sided lower bound on the `Some` payload closes to a window; `None -> false` is NOT a range.
        assert_eq!(
            ranges_of(
                "(do (type Opt (None) (Some Int64)) \
                 (@ (requires (match o ((Opt.Some n) (>= n 0)) ((Opt.None) false))) \
                   (def (f (: o Opt)) 0)) (def (z) 1))",
                "o"
            ),
            vec![("Some".to_string(), (0, super::ONE_SIDED_INVARIANT_WINDOW))]
        );
        // A two-sided range maps in directly.
        assert_eq!(
            ranges_of(
                "(do (type Opt (None) (Some Int64)) \
                 (@ (requires (match o ((Opt.Some n) (and (>= n 0) (<= n 9))) ((Opt.None) false))) \
                   (def (f (: o Opt)) 0)) (def (z) 1))",
                "o"
            ),
            vec![("Some".to_string(), (0, 9))]
        );
        // A nullary pattern (no payload binder) and a `true` allow-all arm yield no payload range.
        assert!(
            ranges_of(
                "(do (type T (A) (B Int64)) \
                 (@ (requires (match o ((T.A) true) ((T.B m) true))) \
                   (def (f (: o T)) 0)) (def (z) 1))",
                "o"
            )
            .is_empty()
        );
        // A match on a DIFFERENT param constrains nothing.
        assert!(
            ranges_of(
                "(do (type Opt (None) (Some Int64)) \
                 (@ (requires (match o ((Opt.Some n) (>= n 0)) ((Opt.None) false))) \
                   (def (f (: o Opt) (: p Opt)) 0)) (def (z) 1))",
                "p"
            )
            .is_empty()
        );
    }

    /// `sum_ctor_payload_min_lens` is the LIST-payload twin of `sum_ctor_payload_ranges`: an arm whose body is
    /// a recognized min-length guard over the pattern's single `(List …)` payload binder (`((Box.Full xs) (< 0
    /// (List.len xs)))`) yields `(Ctor, min_len)`, so the generator floors that constructor's drawn list length
    /// (no spurious `Full([])`). A `false`/`true` arm body, a nullary/multi-bind pattern, an upper-bound-only
    /// guard, or a match on another param yields nothing.
    #[test]
    fn sum_ctor_payload_min_lens_reads_length_guard_arms() {
        let mins_of = |src: &str, param: &str| -> Vec<(String, usize)> {
            let ast = crate::testkit::parse(src);
            let items: Vec<_> = ast.as_form(ast.root, "do").unwrap().to_vec();
            let ann = *items
                .iter()
                .find(|&&it| ast.as_form(it, "@").is_some())
                .expect("an @ annotation");
            let head = ast.as_form(ann, "@").unwrap()[0];
            let pred = ast.as_form(head, "requires").expect("(requires Q)")[0];
            super::sum_ctor_payload_min_lens(&ast, pred, param)
        };
        // `(< 0 (List.len xs))` → floor 1 (non-empty); `Empty -> false` is not a length guard.
        assert_eq!(
            mins_of(
                "(do (type Box (Empty) (Full (List Int64))) \
                 (@ (requires (match o ((Box.Full xs) (< 0 (List.len xs))) ((Box.Empty) false))) \
                   (def (f (: o Box)) 0)) (def (z) 1))",
                "o"
            ),
            vec![("Full".to_string(), 1)]
        );
        // `(<= 2 (List.len xs))` → floor 2.
        assert_eq!(
            mins_of(
                "(do (type Box (Empty) (Full (List Int64))) \
                 (@ (requires (match o ((Box.Full xs) (<= 2 (List.len xs))) ((Box.Empty) false))) \
                   (def (f (: o Box)) 0)) (def (z) 1))",
                "o"
            ),
            vec![("Full".to_string(), 2)]
        );
        // A `true` allow-all arm (no length guard) yields no floor.
        assert!(
            mins_of(
                "(do (type Box (Empty) (Full (List Int64))) \
                 (@ (requires (match o ((Box.Full xs) true) ((Box.Empty) true))) \
                   (def (f (: o Box)) 0)) (def (z) 1))",
                "o"
            )
            .is_empty()
        );
    }

    /// A `@test` over an EMPTY `(Tuple)` (and, symmetrically, an empty `(Record)`) param gets a DECLINING
    /// wrapper: `classify_ty_at`'s zero-slot/zero-field guards (`tup_tail.is_empty()`/`rec_tail.is_empty()`
    /// → `None`) make the compound un-generatable, so `plan_for_item`'s `None if is_compound_form` arm
    /// synthesizes a trapping nullary `-gen` (neutralizing the original) rather than letting the empty
    /// compound reach the export boundary and abort the whole `cdz test` file. Pins these two decline guards
    /// (previously untested, unlike the recursive-sum / multi-payload / non-generatable-leaf siblings): an
    /// empty tuple/record has nothing to draw, so a per-test `FAIL NAME-gen` is the correct clean decline.
    #[test]
    fn an_empty_tuple_or_record_compound_gets_a_declining_wrapper() {
        for (src, wrapper) in [
            ("(do (@ test (def (e (: t (Tuple)))) (def (o) 1)))", "e-gen"),
            (
                "(do (@ test (def (r (: v (Record)))) (def (o) 1)))",
                "r-gen",
            ),
        ] {
            let ast = crate::testkit::parse(src);
            let db = Db::load(ast);
            let orig = wrapper.trim_end_matches("-gen").to_string();
            let test_names: Vec<String> = db
                .test_defs()
                .into_iter()
                .map(|i| db.defs[i].name.clone())
                .collect();
            assert!(
                test_names.iter().any(|n| n == wrapper) && !test_names.iter().any(|n| n == &orig),
                "an empty-compound param gets a DECLINING wrapper ({wrapper}), original {orig} \
                 neutralized: {test_names:?}"
            );
            let gen_def = db
                .defs
                .iter()
                .find(|d| d.name == wrapper)
                .unwrap_or_else(|| panic!("{wrapper} def exists"));
            assert!(
                gen_def.params.is_empty(),
                "the declining wrapper is nullary (neutralizes the empty-compound param → never hits the \
                 boundary)"
            );
        }
    }

    /// A `@test` over a USER-SUM param whose PAYLOAD is non-generatable (`type T = A(Char) | B` — `Char`
    /// unsupported) gets a DECLINING wrapper (`p-gen`, trapping nullary; original `p` neutralized), NOT a
    /// file-abort. classify_sum returns None (the `Char` payload isn't generatable), and because `T`
    /// RESOLVES to a user `(type …)` the `name_resolves_to_user_type` guard routes it to the declining
    /// wrapper — so a sibling test still runs, rather than `p` escaping to the export boundary and aborting
    /// the whole `cdz test` file (`a T sum crosses the host boundary…`, exit 1, siblings killed). The
    /// user-sum counterpart to `a_nongeneratable_leaf_compound_gets_a_declining_wrapper` (a COMPOUND-FORM
    /// leaf). GUARDS AGAINST OVER-CAPTURE: an UNRESOLVABLE bare name still returns None (keeps its CDZ0101),
    /// pinned by `an_unknown_bare_name_type_param_is_not_masked_as_a_declining_wrapper`; and a GENERATABLE
    /// sum still gets a REAL wrapper, pinned by `synthesizes_a_generator_wrapper_for_a_sum_test`.
    #[test]
    fn a_user_sum_with_a_nongeneratable_payload_gets_a_declining_wrapper() {
        let ast = crate::testkit::parse(
            "(do (type T (A Char) (B)) \
               (@ test (def (p (: x T)) unit)) (def (o) 1))",
        );
        let db = Db::load(ast);
        let names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "p-gen") && !names.iter().any(|n| n == "p"),
            "a user-sum-with-nongeneratable-payload gets a DECLINING wrapper (p-gen), original p \
             neutralized — siblings survive instead of a file-abort: {names:?}"
        );
        let p_gen = db
            .defs
            .iter()
            .find(|d| d.name == "p-gen")
            .expect("p-gen def exists");
        assert!(
            p_gen.params.is_empty(),
            "the declining wrapper is nullary (neutralizes the sum param → never hits the boundary)"
        );
    }
}
