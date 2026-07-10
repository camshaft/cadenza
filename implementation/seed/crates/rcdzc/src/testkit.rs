//! Shared test fixtures — builders for the tiny Stage-0 programs the query modules assert over.
//!
//! Kept in one place so `db`, `resolve`, `infer`, and `lower` tests all build the SAME subject AST
//! (a change to the slice's shape updates one builder, not four). Compiled only under `#[cfg(test)]`.

#![cfg(test)]

use crate::ast::{Arenas, Builder, IntValue, Leaf, Radix, StructId};

/// Build `(module m (def (main) 42) (export main))` and return `(arenas, the-42-literal-node-id)`.
/// The literal id is what the `type_of` / `core_of` queries are asked about.
pub fn scalar_program() -> (Arenas, StructId) {
    let mut b = Builder::new();
    let module = b.name("module");
    let m = b.name("m");
    // (def (main) 42)
    let def = b.name("def");
    let main_sig_name = b.name("main");
    let sig = b.list(vec![main_sig_name]);
    let body = b.atom_leaf(Leaf::Int { value: IntValue::from_i64(42), radix: Radix::Dec });
    let def_form = b.list(vec![def, sig, body]);
    // (export main)
    let export = b.name("export");
    let main_ref = b.name("main");
    let export_form = b.list(vec![export, main_ref]);
    let root = b.list(vec![module, m, def_form, export_form]);
    let ast = b.finish(root);
    (ast, body)
}

/// Build `(module m (def (main) (if false 1 2)) (export main))` and return `(arenas, if-node-id)`.
/// The two-way branch case — its id is the `if` node.
pub fn if_program() -> (Arenas, StructId) {
    let mut b = Builder::new();
    let module = b.name("module");
    let m = b.name("m");
    let def = b.name("def");
    let main_sig_name = b.name("main");
    let sig = b.list(vec![main_sig_name]);
    // (if false 1 2)
    let if_head = b.name("if");
    let cond = b.atom_leaf(Leaf::Bool(false));
    let then_ = b.atom_leaf(Leaf::Int { value: IntValue::from_i64(1), radix: Radix::Dec });
    let else_ = b.atom_leaf(Leaf::Int { value: IntValue::from_i64(2), radix: Radix::Dec });
    let if_form = b.list(vec![if_head, cond, then_, else_]);
    let def_form = b.list(vec![def, sig, if_form]);
    let export = b.name("export");
    let main_ref = b.name("main");
    let export_form = b.list(vec![export, main_ref]);
    let root = b.list(vec![module, m, def_form, export_form]);
    let ast = b.finish(root);
    (ast, if_form)
}
