//! Shared test fixtures — builders for the tiny Stage-0 programs the query modules assert over.
//!
//! Kept in one place so `db`, `resolve`, `infer`, and `lower` tests all build the SAME subject AST
//! (a change to the slice's shape updates one builder, not four). Compiled only under `#[cfg(test)]`.

#![cfg(test)]

use crate::ast::{Arenas, Builder, IntValue, Leaf, Radix, StructId};

/// A tiny s-expression reader for TESTS ONLY — turns a readable string like
/// `(module m (def (main) (let ((p (record (x 1) (y 2)))) (. p x))) (export main))` into `Arenas`, so
/// a test case is one line rather than a dozen builder calls. This is NOT the compiler's reader (the
/// compiler takes binary AST); it just spares the tests the manual `Builder` plumbing. Classifies a
/// token as int / bool / name via the same rules the real reader would (radix-free ints + `true`/
/// `false`); anything with a leading digit that isn't a clean integer stays a name (harmless in tests).
pub fn parse(src: &str) -> Arenas {
    let mut b = Builder::new();
    let bytes: Vec<char> = src.chars().collect();
    let mut pos = 0;
    let root = read_node(&bytes, &mut pos, &mut b);
    skip_ws(&bytes, &mut pos);
    assert_eq!(
        pos,
        bytes.len(),
        "trailing input in test s-expr at {pos}: {src}"
    );
    b.finish(root)
}

fn skip_ws(b: &[char], pos: &mut usize) {
    while *pos < b.len() && b[*pos].is_whitespace() {
        *pos += 1;
    }
}

fn read_node(b: &[char], pos: &mut usize, out: &mut Builder) -> StructId {
    skip_ws(b, pos);
    if *pos < b.len() && b[*pos] == '(' {
        *pos += 1; // '('
        let mut children = Vec::new();
        loop {
            skip_ws(b, pos);
            assert!(*pos < b.len(), "unterminated list");
            if b[*pos] == ')' {
                *pos += 1;
                break;
            }
            children.push(read_node(b, pos, out));
        }
        out.list(children)
    } else {
        // A token up to whitespace or a paren.
        let start = *pos;
        while *pos < b.len() && !b[*pos].is_whitespace() && b[*pos] != '(' && b[*pos] != ')' {
            *pos += 1;
        }
        let tok: String = b[start..*pos].iter().collect();
        out.atom_leaf(classify(&tok))
    }
}

fn classify(tok: &str) -> Leaf {
    match tok {
        "true" => Leaf::Bool(true),
        "false" => Leaf::Bool(false),
        _ => {
            // A clean signed decimal integer → Int; else a Name.
            let body = tok.strip_prefix('-').unwrap_or(tok);
            if !body.is_empty() && body.chars().all(|c| c.is_ascii_digit()) {
                let n: i64 = tok.parse().expect("test int fits i64");
                Leaf::Int {
                    value: IntValue::from_i64(n),
                    radix: Radix::Dec,
                }
            } else {
                Leaf::Name(tok.to_string())
            }
        }
    }
}

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
    let body = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(42),
        radix: Radix::Dec,
    });
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
    let then_ = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(1),
        radix: Radix::Dec,
    });
    let else_ = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(2),
        radix: Radix::Dec,
    });
    let if_form = b.list(vec![if_head, cond, then_, else_]);
    let def_form = b.list(vec![def, sig, if_form]);
    let export = b.name("export");
    let main_ref = b.name("main");
    let export_form = b.list(vec![export, main_ref]);
    let root = b.list(vec![module, m, def_form, export_form]);
    let ast = b.finish(root);
    (ast, if_form)
}
