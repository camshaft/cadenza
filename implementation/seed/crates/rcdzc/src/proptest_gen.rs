//! Compiler-directed generators for property tests over COLLECTION types (F1 / approach A —
//! compiler-synthesis; see `implementation/design/DESIGN-property-test-collection-generators-rcdzc.md`).
//!
//! `cdz test` already property-tests a `@test` def with SCALAR parameters: the runner generates each
//! scalar and (for a guest that performs `Test.gen : Unit -> Int64`) drives a seeded int pool with
//! shrinking. A `@test` with a COMPOUND parameter (`List Int64`, …) has no boundary representation, so it
//! could not be property-tested — it declined at the export boundary.
//!
//! This pass closes that gap for `List <Int>` by SYNTHESIS: for
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
//!     (let ((x0 ((. Test gen)))) (let ((x1 …)) (let ((x2 …))
//!       (p (list x0 x1 x2))))))))
//! ```
//!
//! The wrapper performs `Test.gen` `K` times to build a fixed-length list, then calls the real test. The
//! existing gen-driven runner detects it (it pulls `Test.gen` ints), runs `--trials` trials, and shrinks
//! over the int pool — no ABI change, no runner change. This is the FIXED-LENGTH first increment (G1);
//! variable length and richer element types (`List Bool`, tuple/record/sum, `Set`/`Map`) are later
//! increments over the same `<gen:T>` recursion.
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
/// node (the `@test` unwrapped), the def's NAME occurrence (to call it from the wrapper), and how many
/// list elements to generate (the element type is an integer, so `Test.gen` builds each directly).
struct TestPlan {
    item_idx: usize,
    inner_def: StructId,
    /// The def's name as text (e.g. `"p"`) — the wrapper calls `(p (list …))`.
    def_name: String,
    /// The synthesized wrapper's name (`"<def_name>-gen"`).
    wrapper_name: String,
    /// The list's ELEMENT kind — decides the `<gen:ELEM>` expression each list slot is built from.
    elem: ElemKind,
}

/// The element type of a `(List ELEM)` a G1/G2 wrapper can generate, and the shape of its `<gen:ELEM>`
/// expression built from a `Test.gen` int. An INTEGER element uses the raw int directly; a BOOL element
/// reads its low bit (`= gen 0`). Richer elements (nested `List`, tuple/record/sum, `Float`/`Char`) are
/// later increments — `plan_for_item` returns `None` for them, so the def declines as before.
#[derive(Clone, Copy)]
enum ElemKind {
    /// An integer type (`Int8`…`UInt64`): the element IS `((. Test gen))` (the int at the element width).
    Int,
    /// `Bool`: the element is `(= ((. Test gen)) 0)` (the gen int's low-bit-ish parity → a boolean).
    Bool,
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
    // TYPE must be `(List ELEM)` with ELEM a type this pass can generate (an integer or `Bool`).
    let list_tail = ast.as_form(ty, "List")?;
    let &elem = list_tail.first()?;
    let elem_name = ast.as_name(elem)?;
    let elem = elem_kind(elem_name)?; // non-generatable element (nested list, float, …) → decline as before
    Some(TestPlan {
        item_idx,
        inner_def: inner,
        // Suffix `-gen`: a hyphen-delimited segment that begins with a letter, so the wrapper name is a
        // valid component extern name (an extern name's `-`-separated segments must each start with a
        // letter — a `$` or a digit-led segment fails boundary-name validation). The wrapper is what
        // `cdz test` reports, so the name stays readable (`p` → `p-gen`).
        wrapper_name: format!("{def_name}-gen"),
        def_name,
        elem,
    })
}

/// Classify a `(List ELEM)` element type name into the [`ElemKind`] whose `<gen:ELEM>` this pass builds,
/// or `None` if it is not (yet) generatable (a nested `List`, `Float`, `Char`, tuple/record/sum — later
/// increments). An integer type of any admitted width, or `Bool`.
fn elem_kind(name: &str) -> Option<ElemKind> {
    match name {
        "Int8" | "Int16" | "Int32" | "Int64" | "UInt8" | "UInt16" | "UInt32" | "UInt64" => {
            Some(ElemKind::Int)
        }
        "Bool" => Some(ElemKind::Bool),
        _ => None,
    }
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
/// `(@ test (def (NAME-gen) (host (Test) (let ((x0 ((. Test gen)))) … (CALL (list x0 …))))))`.
fn build_wrapper(ast: &mut Arenas, plan: &TestPlan) -> StructId {
    // The `K` generated element bindings `x0..xK`, each `((. Test gen))` (an APPLICATION of the member
    // access `Test.gen` to no args — a nullary call).
    let elem_names: Vec<String> = (0..G1_LIST_LEN).map(|k| format!("x{k}")).collect();

    // `(list x0 x1 …)` — the generated list the wrapper passes to the real test.
    let list_expr = {
        let head = name(ast, "list");
        let mut children = vec![head];
        for nm in &elem_names {
            children.push(name(ast, nm));
        }
        push_list(ast, children)
    };
    // `(NAME (list …))` — call the original test with the generated list.
    let call = {
        let callee = name(ast, &plan.def_name);
        push_list(ast, vec![callee, list_expr])
    };
    // Wrap the call in nested `let`s binding each `xk = <gen:ELEM>`, innermost first so the final `let`
    // body is `call`.
    let mut body = call;
    for nm in elem_names.iter().rev() {
        let gen_expr = build_elem_gen(ast, plan.elem);
        // `(xk gen_expr)` binding, in a single-binding list `((xk gen_expr))`.
        let binder = {
            let x = name(ast, nm);
            push_list(ast, vec![x, gen_expr])
        };
        let binds = push_list(ast, vec![binder]);
        let let_head = name(ast, "let");
        body = push_list(ast, vec![let_head, binds, body]);
    }
    // `(host (Test) body)` — delegate the Test effect to the boundary.
    let host = {
        let head = name(ast, "host");
        let test = name(ast, "Test");
        let effs = push_list(ast, vec![test]);
        push_list(ast, vec![head, effs, body])
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

/// Build one `<gen:ELEM>` expression — the value a single list slot is generated from, per the element
/// kind. Both build on the same `((. Test gen))` nullary call (a `Test.gen` performance); an integer
/// element uses it directly, a `Bool` element reads it as `(= ((. Test gen)) 0)`. A richer element type
/// (nested list, tuple/record/sum, float/char) would recurse here in a later increment.
fn build_elem_gen(ast: &mut Arenas, elem: ElemKind) -> StructId {
    // `((. Test gen))` — the nullary application of the member access `Test.gen`.
    let gen_call = {
        let dot = name(ast, ".");
        let test = name(ast, "Test");
        let gen_nm = name(ast, "gen");
        let member = push_list(ast, vec![dot, test, gen_nm]);
        push_list(ast, vec![member])
    };
    match elem {
        ElemKind::Int => gen_call,
        // `(= gen_call 0)` — the gen int as a boolean (whether it equals zero). Any total int→Bool map
        // works; equality-with-zero is the simplest and covers both values across the generated ints.
        ElemKind::Bool => {
            let eq = name(ast, "=");
            let zero = push_atom(
                ast,
                Leaf::Int {
                    value: crate::ast::IntValue::zero(),
                    radix: crate::ast::Radix::Dec,
                },
            );
            push_list(ast, vec![eq, gen_call, zero])
        }
    }
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

    /// A `@test` over a `(List <non-generatable>)` (e.g. a nested `(List (List Int64))`) is left alone —
    /// it declines at the boundary as before, rather than synthesizing a wrapper it cannot build yet.
    #[test]
    fn leaves_a_nongeneratable_element_alone() {
        let ast = crate::testkit::parse(
            "(do (@ test (def (r (: xs (List (List Int64)))) (List.len xs))) (def (other) 1))",
        );
        let db = Db::load(ast);
        let test_names: Vec<String> = db
            .test_defs()
            .into_iter()
            .map(|i| db.defs[i].name.clone())
            .collect();
        // No wrapper synthesized; the original `r` is still the (compound-param) test — it will decline
        // at the boundary downstream, which is the correct "not yet" behavior for a nested-list element.
        assert!(
            !test_names.iter().any(|n| n == "r-gen"),
            "a non-generatable element gets no wrapper: {test_names:?}"
        );
    }
}
