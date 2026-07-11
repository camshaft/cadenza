//! A readable debug view of the AST: the arena structure as an indented tree.
//!
//! Where `sexpr::print` reconstructs the PROGRAM text, this shows the raw SHAPE the compiler sees —
//! each structure occurrence on its own line, indented by depth, tagged with its `StructId` and,
//! for an atom, its leaf kind and value. It is a read-only view (not re-readable back to binary); it
//! exists to answer "what does this binary AST actually decode to?" at a glance.
//!
//! Example, for `(do (def (main) 42) (export main))`:
//! ```text
//! #12 List
//!   #0 Atom Name do
//!   #6 List
//!     #1 Atom Name def
//!     #3 List
//!       #2 Atom Name main
//!     #4 Atom Int 42 (dec)
//!   #11 List
//!     #7 Atom Name export
//!     #8 Atom Name main
//! ```
//! The `#N` is the occurrence's index in the structure arena — note it is post-order (children are
//! built before their parent), so a parent's id is larger than its children's.

use crate::ast::{Arenas, Leaf, Radix, Struct, StructId};

/// Render `arenas` as an indented debug tree, rooted at its root occurrence.
pub fn print(arenas: &Arenas) -> String {
    let mut out = String::new();
    node(arenas, arenas.root, 0, &mut out);
    out
}

fn node(a: &Arenas, id: StructId, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    match a.get(id) {
        Struct::Atom(leaf_id) => {
            out.push_str(&format!("#{} Atom {}\n", id.0, leaf(a.leaf(*leaf_id))));
        }
        Struct::List(children) => {
            out.push_str(&format!("#{} List\n", id.0));
            for &child in children {
                node(a, child, depth + 1, out);
            }
        }
    }
}

/// A leaf's kind and value in a compact, unambiguous form.
fn leaf(l: &Leaf) -> String {
    match l {
        Leaf::Int { value, radix } => format!("Int {value} ({})", radix_name(*radix)),
        Leaf::Float(d) => format!("Float {}", crate::literal::render_decimal(d)),
        Leaf::Str(s) => format!("Str {s:?}"),
        Leaf::Bool(b) => format!("Bool {b}"),
        Leaf::Name(n) => format!("Name {n}"),
    }
}

fn radix_name(r: Radix) -> &'static str {
    match r {
        Radix::Dec => "dec",
        Radix::Hex => "hex",
        Radix::Bin => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sexpr;

    #[test]
    fn renders_the_arena_tree() {
        let arenas = sexpr::read("(def (main) 42)").unwrap();
        let out = print(&arenas);
        // Every occurrence appears with its id + kind; the head names and the literal are shown.
        assert!(out.contains("Atom Name def"), "{out}");
        assert!(out.contains("Atom Name main"), "{out}");
        assert!(out.contains("Atom Int 42 (dec)"), "{out}");
        // Nesting: the outer list is at depth 0, its atom children indented.
        assert!(out.starts_with('#'), "{out}");
        assert!(out.contains("\n  #"), "expected indented children in:\n{out}");
    }

    #[test]
    fn hex_radix_is_shown() {
        let arenas = sexpr::read("0x2A").unwrap();
        assert!(print(&arenas).contains("Int 42 (hex)"));
    }
}
