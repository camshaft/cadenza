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

/// Emit the pre-order tree, one occurrence per line indented by depth. Uses an EXPLICIT stack rather
/// than native recursion: `codec::decode` accepts arbitrarily-deep valid-tree arenas (it builds flat
/// and validates tree-ness with its own explicit stack — no depth cap), so a legitimately-decoded
/// arena can nest deeper than the native call stack survives. A recursive walk here overflowed the
/// stack (SIGABRT) on such input, which would crash the process on a debug view of an untrusted binary
/// AST — the crate's readers/printers must stay total. The stack holds `(id, depth)`; children are
/// pushed in REVERSE so they pop left-to-right, preserving source order.
fn node(a: &Arenas, root: StructId, depth: usize, out: &mut String) {
    let mut stack: Vec<(StructId, usize)> = vec![(root, depth)];
    while let Some((id, depth)) = stack.pop() {
        for _ in 0..depth {
            out.push_str("  ");
        }
        match a.get(id) {
            Struct::Atom(leaf_id) => {
                out.push_str(&format!("#{} Atom {}\n", id.0, leaf(a.leaf(*leaf_id))));
            }
            Struct::List(children) => {
                out.push_str(&format!("#{} List\n", id.0));
                for &child in children.iter().rev() {
                    stack.push((child, depth + 1));
                }
            }
        }
    }
}

/// A leaf's kind and value in a compact, unambiguous form.
fn leaf(l: &Leaf) -> String {
    match l {
        Leaf::Int { value, radix } => {
            format!("Int {} ({})", value.to_decimal_string(), radix_name(*radix))
        }
        Leaf::Float(d) => format!("Float {}", crate::literal::render_decimal(d)),
        Leaf::FloatNan => "FloatNan".to_string(),
        Leaf::FloatInf { negative } => {
            format!("FloatInf {}", if *negative { "-inf" } else { "inf" })
        }
        Leaf::Str(s) => format!("Str {s:?}"),
        Leaf::Bytes(b) => format!("Bytes {b:?}"),
        Leaf::Bool(b) => format!("Bool {b}"),
        Leaf::Sym(s) => format!("Sym {s:?}"),
        Leaf::Name(n) => format!("Name {n}"),
        Leaf::BadEscape(c) => format!("BadEscape {c:?}"),
        Leaf::Char(c) => format!("Char {c:?}"),
        Leaf::BadChar(s) => format!("BadChar {s:?}"),
        Leaf::Suffixed { value, kind } => {
            format!(
                "Suffixed {} ({})",
                crate::literal::render_suffixed(value, *kind),
                kind.type_name()
            )
        }
        // Native compound HEAD leaves (M2).
        Leaf::Ctor(c) => format!("Ctor {c:?}"),
        Leaf::FieldPair => "FieldPair".to_string(),
        Leaf::Member => "Member".to_string(),
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
    use crate::ast::{Builder, Decimal, SuffixBody, SuffixKind};
    use crate::sexpr;

    /// Build an `n`-deep single-child `List` chain wrapping one atom — a minimal-width, maximal-depth
    /// arena (the shape a crafted-but-valid binary AST uses to attack a recursive walker).
    fn deep_chain(n: usize) -> Arenas {
        let mut b = Builder::new();
        let mut cur = b.atom_leaf(Leaf::Name("x".into()));
        for _ in 0..n {
            cur = b.list(vec![cur]);
        }
        b.finish(cur)
    }

    #[test]
    fn print_tree_is_iterative_not_recursive() {
        // `print`'s tree walk must be ITERATIVE. Before the fix it recursed natively — one frame per
        // level — and OVERFLOWED the stack (SIGABRT) on a deep arena. `codec::decode` accepts
        // arbitrarily-deep valid trees (no depth cap, unlike the s-expr reader's MAX_NESTING_DEPTH), so a
        // debug view of an untrusted binary AST could crash the process. Use a depth (12k) well past the
        // native recursion limit the old code died at, but modest enough that the tree view's inherently
        // QUADRATIC output (cumulative indent, ~depth²/2 chars) stays bounded. Completing at all is the
        // assertion — a recursive walker never returns here.
        let depth = 12_000usize;
        let tree = print(&deep_chain(depth)); // recursive code overflowed; iterative completes
        assert!(tree.contains("Atom Name x"), "the deep leaf is rendered");
        // The deepest atom carries the full indent (2 spaces × depth).
        assert!(
            tree.contains(&format!("{}#", "  ".repeat(depth))),
            "the deepest node carries its full indent"
        );
    }

    #[test]
    fn print_flat_stays_total_on_a_very_deep_arena() {
        // `print_flat` is O(n) (no cumulative indent), so it takes an even deeper chain — the realistic
        // "dump a decoded untrusted arena" path. 200k levels, far past any native-stack limit; it is
        // already iterative (a flat loop over the structure vector) and must render leaf + root.
        let a = deep_chain(200_000);
        let flat = print_flat(&a);
        assert!(flat.contains("Name x"), "the leaf pool holds the atom");
        assert!(
            flat.contains(&format!("root: S{}", a.root.0)),
            "the root line is present"
        );
    }

    #[test]
    fn every_leaf_kind_renders_distinctly() {
        // The debug view is what a compiler engineer reads to answer "what did this binary AST decode
        // to?" — so `leaf()` must render EVERY `Leaf` variant with its kind tag + value, and the marker
        // leaves (`BadEscape`/`BadChar`) must be visibly distinct from their well-formed cousins
        // (`Str`/`Char`). Build one arena holding an atom of each kind and assert each line.
        let leaves = [
            (
                Leaf::Int {
                    value: crate::ast::IntValue::from_i64(42),
                    radix: Radix::Hex,
                },
                "Int 42 (hex)",
            ),
            (
                Leaf::Float(Decimal {
                    negative: false,
                    significand: crate::ast::IntValue::from_i64(15).magnitude,
                    exponent: -1,
                }),
                "Float 1.5",
            ),
            (Leaf::Str("hi\n".into()), "Str \"hi\\n\""),
            (Leaf::Bytes(vec![0, 255].into()), "Bytes [0, 255]"),
            (Leaf::Bool(true), "Bool true"),
            (Leaf::Sym("meter".into()), "Sym \"meter\""),
            (Leaf::Name("foo".into()), "Name foo"),
            (Leaf::BadEscape('q'), "BadEscape 'q'"),
            (Leaf::Char('a'), "Char 'a'"),
            (Leaf::BadChar("u+D800".into()), "BadChar \"u+D800\""),
            (
                Leaf::Suffixed {
                    value: SuffixBody::Int {
                        value: crate::ast::IntValue::from_i64(100),
                        radix: Radix::Dec,
                    },
                    kind: SuffixKind::BigInt,
                },
                "Suffixed 100N (BigInt)",
            ),
        ];
        for (leaf_val, expected) in leaves {
            // A single-atom arena for this leaf.
            let mut b = Builder::new();
            let root = b.atom_leaf(leaf_val.clone());
            let a = b.finish(root);
            let out = print(&a);
            assert!(
                out.contains(expected),
                "leaf {leaf_val:?} should render as {expected:?}; got:\n{out}"
            );
        }
    }

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

    /// A tiny deterministic PRNG (SplitMix64) — reproducible generation without a dependency (mirrors
    /// the unit-test PRNGs in `codec.rs`/`lexer.rs`/`canon.rs`).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Build a random arena node (bounded by `depth`): either an atom of some leaf kind, or a `List`
    /// with 0..=3 random children. Multi-child, varied-width — the shape that catches a walker that
    /// visits children in the WRONG order (a single-child chain cannot).
    fn gen_node(rng: &mut Rng, b: &mut Builder, depth: usize) -> StructId {
        if depth == 0 || rng.below(3) == 0 {
            let leaf = match rng.below(4) {
                0 => Leaf::Name(["a", "b", "cc", "x"][rng.below(4)].into()),
                1 => Leaf::Int {
                    value: crate::ast::IntValue::from_i64(rng.below(1000) as i64),
                    radix: Radix::Dec,
                },
                2 => Leaf::Bool(rng.below(2) == 0),
                _ => Leaf::Str(["", "hi", "a b"][rng.below(3)].into()),
            };
            return b.atom_leaf(leaf);
        }
        let n = rng.below(4); // 0..=3 children
        let kids: Vec<StructId> = (0..n).map(|_| gen_node(rng, b, depth - 1)).collect();
        b.list(kids)
    }

    /// An INDEPENDENT recursive oracle for `print`'s tree view — the obvious native-recursion form the
    /// production walker replaced with an explicit stack (to survive deep untrusted arenas). If the
    /// iterative rewrite visits children out of order, mis-indents a level, or drops the reverse-push
    /// bookkeeping, this reference and `print` disagree.
    fn oracle(a: &Arenas, id: StructId, depth: usize, out: &mut String) {
        for _ in 0..depth {
            out.push_str("  ");
        }
        match a.get(id) {
            Struct::Atom(leaf_id) => {
                out.push_str(&format!("#{} Atom {}\n", id.0, leaf(a.leaf(*leaf_id))));
            }
            Struct::List(children) => {
                out.push_str(&format!("#{} List\n", id.0));
                for &child in children.iter() {
                    oracle(a, child, depth + 1, out);
                }
            }
        }
    }

    #[test]
    fn print_matches_an_independent_recursive_oracle_over_generated_arenas() {
        // `print`'s tree walk is an explicit-stack rewrite of a recursion (so a deep untrusted arena
        // can't overflow the native stack). The deep-chain test proves it doesn't crash — but a single-
        // child chain can't detect a walker that visits multi-child siblings in the WRONG order or off by
        // one indent level (the reverse-push is exactly the bookkeeping most likely to drift on a rewrite).
        // Sweep random MULTI-CHILD arenas and assert the iterative `print` is byte-identical to a plain
        // recursive oracle: same pre-order, same left-to-right child order, same per-depth indent.
        let mut rng = Rng(0xde6b_c0de_1a7e_5eed);
        for _ in 0..4000 {
            let depth = 1 + rng.below(4);
            let mut b = Builder::new();
            let root = gen_node(&mut rng, &mut b, depth);
            let a = b.finish(root);
            let got = print(&a);
            let mut want = String::new();
            oracle(&a, a.root, 0, &mut want);
            assert_eq!(
                got, want,
                "iterative `print` diverged from the recursive oracle for arena rooted at #{}",
                a.root.0
            );
        }
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
