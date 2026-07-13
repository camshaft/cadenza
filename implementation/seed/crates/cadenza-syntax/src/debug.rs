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

/// Render `arenas` FLAT — the two arenas dumped literally, exactly as they sit in memory (and on
/// the wire): the leaf pool, then the structure vector, then the root id. Unlike the tree view, this
/// shows the storage directly — leaf INTERNING (each distinct leaf once; a `List` references leaves
/// by id) and the post-order structure layout (a child's id precedes its parent's). Useful for
/// seeing what the codec actually serializes.
///
/// Example, for `(def (main) 42)`:
/// ```text
/// leaves (4):
///   L0  Name def
///   L1  Name main
///   L2  Int 42 (dec)
///   (…)
/// structure (5):
///   S0  Atom L0
///   S1  Atom L1
///   S2  List [S1]
///   S3  Atom L2
///   S4  List [S0 S2 S3]
/// root: S4
/// ```
pub fn print_flat(arenas: &Arenas) -> String {
    let mut out = String::new();

    out.push_str(&format!("leaves ({}):\n", arenas.leaves.len()));
    for (i, l) in arenas.leaves.iter().enumerate() {
        out.push_str(&format!("  L{i}  {}\n", leaf(l)));
    }

    out.push_str(&format!("structure ({}):\n", arenas.structure.len()));
    for (i, s) in arenas.structure.iter().enumerate() {
        match s {
            Struct::Atom(leaf_id) => out.push_str(&format!("  S{i}  Atom L{}\n", leaf_id.0)),
            Struct::List(children) => {
                let ids: Vec<String> = children.iter().map(|c| format!("S{}", c.0)).collect();
                out.push_str(&format!("  S{i}  List [{}]\n", ids.join(" ")));
            }
        }
    }

    out.push_str(&format!("root: S{}\n", arenas.root.0));
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
        Leaf::Bytes(b) => format!("Bytes {b:?}"),
        Leaf::Bool(b) => format!("Bool {b}"),
        Leaf::Sym(s) => format!("Sym {s:?}"),
        Leaf::Name(n) => format!("Name {n}"),
        Leaf::BadEscape(c) => format!("BadEscape {c:?}"),
        Leaf::Char(c) => format!("Char {c:?}"),
        Leaf::BadChar(s) => format!("BadChar {s:?}"),
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
        assert!(
            out.contains("\n  #"),
            "expected indented children in:\n{out}"
        );
    }

    #[test]
    fn hex_radix_is_shown() {
        let arenas = sexpr::read("0x2A").unwrap();
        assert!(print(&arenas).contains("Int 42 (hex)"));
    }

    #[test]
    fn flat_dumps_the_arenas() {
        let arenas = sexpr::read("(def (main) 42)").unwrap();
        let out = print_flat(&arenas);
        // The three sections, sized, with the root pointing at the top structure entry.
        assert!(
            out.contains(&format!("leaves ({}):", arenas.leaves.len())),
            "{out}"
        );
        assert!(
            out.contains(&format!("structure ({}):", arenas.structure.len())),
            "{out}"
        );
        assert!(out.contains(&format!("root: S{}", arenas.root.0)), "{out}");
        // Leaves are addressed L#, and a List references its children by S#.
        assert!(out.contains("L0  Name def"), "{out}");
        assert!(out.contains("List ["), "{out}");
    }

    #[test]
    fn flat_shows_leaf_interning() {
        // `main` occurs TWICE in the source but is interned to ONE leaf; two Atom occurrences point
        // at the same L#. Confirm the leaf pool holds a single `Name main`.
        let arenas = sexpr::read("(do (def (main) 1) (export main))").unwrap();
        let out = print_flat(&arenas);
        let mains = out
            .lines()
            .filter(|l| l.trim_end().ends_with("Name main"))
            .count();
        assert_eq!(mains, 1, "expected `main` interned to one leaf:\n{out}");
    }
}
