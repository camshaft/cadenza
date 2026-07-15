//! Compiler-directed generators for property tests over COLLECTION types (F1 / approach A —
//! compiler-synthesis; see `implementation/design/DESIGN-property-test-collection-generators-rcdzc.md`).
//!
//! `cdz test` already property-tests a `@test` def with SCALAR parameters: the runner generates each
//! scalar and (for a guest that performs `Test.gen : Unit -> Int64`) drives a seeded int pool with
//! shrinking. A `@test` with a COMPOUND parameter (`List Int64`, …) has no boundary representation, so it
//! could not be property-tested — it declined at the export boundary.
//!
//! This pass closes that gap by SYNTHESIS. For a `@test` whose single parameter is a generatable
//! compound type (a `(List ELEM)` or `(Tuple T…)` over integer / `Bool` leaves, nested arbitrarily), e.g.
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
//! consumes one `Test.gen` int (`Bool` = `(= gen 0)`), a `(List ELEM)` builds a fixed-length list of
//! recursively-generated elements, a `(Tuple T…)` builds `(tuple <gen:T> …)`. Every `Test.gen` is hoisted
//! into a `let` (an inlined one under a constructor is not seen within the `host` scope). The existing
//! gen-driven runner detects the wrapper (it pulls `Test.gen` ints), runs `--trials` trials, and shrinks
//! over the int pool — no ABI change, no runner change. Increments so far cover G1 `List<Int>`, G2
//! `List<Bool>` + element recursion, and G3 `Tuple` + arbitrary nesting; still to come are record and sum
//! elements, `Set`/`Map`, variable-length lists, `Float`/`Char`, multi-parameter tests, and a lone
//! single-form file (which currently needs a `do`-block root).
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
    // Only a top-level `(do …)` block can carry both the test defs and the appended effect/wrapper; a
    // bare single-form program (or a `(module …)`) is left untouched by G1 (a compound-param `@test`
    // there still declines, as before — G1 targets the common `(do …)` test file).
    let root = ast.root;
    let Some(items) = ast.as_form(root, "do").map(<[_]>::to_vec) else {
        return;
    };

    // Find each `(@ test (def SIG BODY))` item whose SIG has exactly one `(: name (List ELEM))` param
    // with ELEM an integer type. Record (item index, def-name occ text, param count) to synthesize for.
    let mut plans: Vec<TestPlan> = Vec::new();
    for (idx, &item) in items.iter().enumerate() {
        if let Some(plan) = plan_for_item(ast, idx, item) {
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
            ast.structure[item.0 as usize] = crate::ast::Struct::List(inner_children);
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
/// node (the `@test` unwrapped), the def name (the wrapper calls `(NAME arg)`), and the `GenTy` for the
/// single parameter (which drives the `<gen:T>` the wrapper builds and passes).
struct TestPlan {
    item_idx: usize,
    inner_def: StructId,
    /// The def's name as text (e.g. `"p"`) — the wrapper calls `(p <gen-arg>)`.
    def_name: String,
    /// The synthesized wrapper's name (`"<def_name>-gen"`).
    wrapper_name: String,
    /// The parameter's generatable type — decides the `<gen:T>` expression the wrapper builds + passes.
    gen_ty: GenTy,
}

/// A type this pass can GENERATE, and the recursive shape of its `<gen:T>` expression (each built from
/// one or more `Test.gen` ints). This is the compiler-directed "Arbitrary-like" derivation over the type
/// structure: a scalar consumes one gen int; a `List`/`Tuple` recurses into its element/slot types. A
/// type outside this set (`Float`/`Char`, record, sum, `Set`/`Map`, a bare/unresolved type) is not (yet)
/// generatable — `classify_ty` returns `None`, so the `@test` declines at the boundary as before.
#[derive(Clone)]
enum GenTy {
    /// An integer type (`Int8`…`UInt64`): `<gen>` = `((. Test gen))` (the int at the element width).
    Int,
    /// `Bool`: `<gen>` = `(= ((. Test gen)) 0)` (a gen int read as a boolean).
    Bool,
    /// `(List ELEM)`: `<gen>` = a fixed-length `(list <gen:ELEM> …)` of `G1_LIST_LEN` elements.
    List(Box<GenTy>),
    /// `(Tuple T…)`: `<gen>` = `(tuple <gen:T> …)`, one generated value per slot.
    Tuple(Vec<GenTy>),
}

/// Recognize `(@ test (def (NAME (: PARAM (List ELEM))) BODY))` with ELEM an integer type; return its
/// plan, or `None` if the item is not such a test.
fn plan_for_item(ast: &Arenas, item_idx: usize, item: StructId) -> Option<TestPlan> {
    // `(@ test INNER)` — the annotation must be the bare name `test`.
    let ann = ast.as_form(item, "@")?;
    let (&name_occ, &inner) = (ann.first()?, ann.get(1)?);
    if ast.as_name(name_occ) != Some("test") {
        return None;
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
    // EXACTLY ONE parameter, annotated `(: PARAM (List ELEM))` with ELEM an integer type. (Multi-param
    // and richer element types are later increments; a single `List <Int>` is G1.)
    if params.len() != 1 {
        return None;
    }
    let ann_param = ast.as_form(params[0], ":")?; // `(: name TYPE)`
    let &ty = ann_param.get(1)?;
    // The parameter must be a COMPOUND type this pass generates (a `(List …)` or `(Tuple …)` — a bare
    // scalar is left to the existing boundary-arg route, which needs no wrapper). Classify recursively;
    // `None` (a non-generatable or bare-scalar type) declines the synthesis, leaving the def as-is.
    let gen_ty = classify_ty(ast, ty)?;
    if !matches!(gen_ty, GenTy::List(_) | GenTy::Tuple(_)) {
        return None; // a scalar param — the boundary-arg route handles it; no wrapper needed
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
        gen_ty,
    })
}

/// Recursively classify a parameter TYPE occurrence into the [`GenTy`] whose `<gen:T>` this pass builds,
/// or `None` if it is not (yet) generatable. Handles integer scalars, `Bool`, `(List ELEM)` (recursing
/// into ELEM), and `(Tuple T…)` (recursing into each slot). A `Float`/`Char`, record, sum, `Set`/`Map`,
/// or a bare/unresolved type is `None` — the caller then declines the synthesis.
fn classify_ty(ast: &Arenas, ty: StructId) -> Option<GenTy> {
    // A bare NAME type — a scalar (`Int64`/`Bool`/…).
    if let Some(n) = ast.as_name(ty) {
        return match n {
            "Int8" | "Int16" | "Int32" | "Int64" | "UInt8" | "UInt16" | "UInt32" | "UInt64" => {
                Some(GenTy::Int)
            }
            "Bool" => Some(GenTy::Bool),
            _ => None,
        };
    }
    // `(List ELEM)` — recurse into the element type.
    if let Some(list_tail) = ast.as_form(ty, "List") {
        let &elem = list_tail.first()?;
        return Some(GenTy::List(Box::new(classify_ty(ast, elem)?)));
    }
    // `(Tuple T…)` — recurse into each slot; every slot must be generatable.
    if let Some(tup_tail) = ast.as_form(ty, "Tuple") {
        if tup_tail.is_empty() {
            return None; // a zero-slot tuple — nothing to generate; not modeled
        }
        let mut slots = Vec::with_capacity(tup_tail.len());
        for &slot in tup_tail {
            slots.push(classify_ty(ast, slot)?);
        }
        return Some(GenTy::Tuple(slots));
    }
    None
}

/// Whether the program already declares an effect named `Test` carrying a `gen` operation — so the pass
/// does not append a second, colliding declaration. A shallow check over the top-level items.
fn declares_test_gen(ast: &Arenas, items: &[StructId]) -> bool {
    for &item in items {
        if let Some(eff) = ast.as_form(item, "effect")
            && eff.first().and_then(|&n| ast.as_name(n)) == Some("Test")
        {
            return true;
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
    let gen_arg = build_gen(ast, &plan.gen_ty, &mut binds);
    // `(NAME <gen-arg>)` — call the original test with the generated value.
    let call = {
        let callee = name(ast, &plan.def_name);
        push_list(ast, vec![callee, gen_arg])
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
    // `(@ test def)` — mark the wrapper as the test to hoist.
    let at = name(ast, "@");
    let test_ann = name(ast, "test");
    push_list(ast, vec![at, test_ann, def])
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
        // `(= ((. Test gen)) 0)` — a gen int read as a boolean. Any total int→Bool map works; equality
        // with zero is the simplest and covers both values across the generated ints.
        GenTy::Bool => hoist_scalar(ast, binds, |ast| {
            let g = gen_call(ast);
            let eq = name(ast, "=");
            let zero = push_atom(
                ast,
                Leaf::Int {
                    value: crate::ast::IntValue::zero(),
                    radix: crate::ast::Radix::Dec,
                },
            );
            push_list(ast, vec![eq, g, zero])
        }),
        // `(list <gen:ELEM> …)` — a fixed-length list (G1_LIST_LEN elements), each recursively generated.
        GenTy::List(elem) => {
            let head = name(ast, "list");
            let mut children = vec![head];
            for _ in 0..G1_LIST_LEN {
                children.push(build_gen(ast, elem, binds));
            }
            push_list(ast, children)
        }
        // `(tuple <gen:T> …)` — one generated value per slot.
        GenTy::Tuple(slots) => {
            let head = name(ast, "tuple");
            let mut children = vec![head];
            for slot in slots {
                children.push(build_gen(ast, slot, binds));
            }
            push_list(ast, children)
        }
    }
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

    /// G2: a `(List Bool)` element is also generatable (the wrapper builds each element as `= gen 0`), so
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

    /// A `@test` over a genuinely NON-generatable element (`(List Float64)` — floats are not yet
    /// generated) is left alone: no wrapper, so it declines at the boundary as before. (Nested
    /// `List`/`Tuple` over int/Bool leaves ARE generatable now — the non-generatable leaf is what stops it.)
    #[test]
    fn leaves_a_nongeneratable_element_alone() {
        let ast = crate::testkit::parse(
            "(do (@ test (def (r (: xs (List Float64))) (List.len xs))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        assert!(
            !test_names.iter().any(|n| n == "r-gen"),
            "a non-generatable (Float) element gets no wrapper: {test_names:?}"
        );
    }
}
