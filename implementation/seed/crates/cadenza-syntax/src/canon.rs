//! Canonicalization: re-index the arenas into a deterministic normal form so that equal PROGRAMS
//! encode to identical bytes, regardless of the order the nodes happened to be built in.
//!
//! Why this is needed. The two surfaces build the same tree in different occurrence orders — e.g.
//! the s-expr reader builds `(+ a b)` head-first (`+`, `a`, `b`), while the ML parser parses the
//! left operand `a` before synthesizing the `+` head. Same tree, different `StructId` assignment,
//! so a raw `codec::encode` produces different bytes even though the programs are equal. Byte
//! identity is therefore a property of a NORMAL FORM reached by this pass (the same shape as NFC
//! for strings), not of the codec — the codec faithfully serializes whatever arena it is handed.
//!
//! The normal form. Walk the structure tree from `root` in a fixed order and re-number every
//! occurrence and leaf by FIRST-ENCOUNTER during that walk. Because both surfaces yield the same
//! tree, the walk visits the same nodes in the same order, so the renumbering is identical — the
//! build order is erased. The walk is pre-order over the root's reachable structure; leaves are
//! interned in the order their atoms are first visited. Unreachable nodes (the reader never
//! produces them, but a hand-built arena might) are dropped, which is correct for a normal form.

use crate::ast::{Arenas, Leaf, LeafId, Struct, StructId};
use crate::fxhash::FxHashMap;

/// Return the canonical form of `arenas`: the same program, re-indexed so that any arena denoting
/// this tree yields byte-identical output from [`crate::codec::encode`]. Idempotent.
///
/// Returns a [`Cow`]: `Borrowed` when `arenas` is ALREADY canonical (the common case — the s-expr
/// reader builds structure in pre-order and interns leaves in first-encounter order, so a fresh parse
/// is already normal-form), so the caller serializes it in place with NO clone and NO rebuild; `Owned`
/// only when a genuine renumbering is needed (e.g. the ML surface, which parses operands before
/// synthesizing heads). The full rebuild below would otherwise clone every leaf + structure node — a
/// second full pass over a large arena — and throw the identical result away.
pub fn canonicalize(arenas: &Arenas) -> std::borrow::Cow<'_, Arenas> {
    if is_canonical(arenas) {
        return std::borrow::Cow::Borrowed(arenas);
    }
    let mut c = Canon {
        src: arenas,
        leaves: Vec::new(),
        leaf_map: FxHashMap::default(),
        structure: Vec::new(),
    };
    let root = c.visit(arenas.root);
    std::borrow::Cow::Owned(Arenas {
        leaves: c.leaves,
        structure: c.structure,
        root,
    })
}

/// Whether `arenas` is ALREADY in canonical form — i.e. [`canonicalize`] would rebuild a structurally
/// identical arena, so serializing `arenas` in place is byte-equal. The normal form numbers structure
/// nodes AND leaves by FIRST-ENCOUNTER in the PRE-ORDER walk from `root` (`canonicalize`'s `visit`), so
/// this replays exactly that walk and checks the numbering is the identity: each node visited is the
/// next sequential `StructId`, each leaf first-seen is the next `LeafId`, and every node/leaf is
/// reached. A structure-array-order scan is NOT equivalent — the rebuild's leaf order follows the
/// pre-order VISIT, which a repeated leaf under a different subtree can reorder relative to the array —
/// so the walk is load-bearing (a linear scan gave false positives on cross-surface programs and broke
/// the byte-equality tests). Any deviation returns `false` → the full rebuild is the sound fallback (a
/// false negative only costs the rebuild we already do, never a wrong result). No native recursion (an
/// explicit stack), so deep input can't overflow.
fn is_canonical(arenas: &Arenas) -> bool {
    if arenas.structure.is_empty() {
        return false;
    }
    let mut next_leaf: u32 = 0;
    let mut seen_leaf = vec![false; arenas.leaves.len()];
    let mut next_struct: u32 = 0;
    let mut entered = vec![false; arenas.structure.len()];
    // (node, child-cursor): a `List` is visited (assigned its id) AFTER its children — matching
    // `canonicalize`'s post-order `push`; an `Atom` is assigned immediately.
    let mut stack: Vec<(StructId, usize)> = vec![(arenas.root, 0)];
    while let Some((node, cursor)) = stack.pop() {
        match arenas.get(node) {
            Struct::Atom(leaf) => {
                let lid = leaf.0 as usize;
                if lid >= seen_leaf.len() {
                    return false;
                }
                if !seen_leaf[lid] {
                    if leaf.0 != next_leaf {
                        return false;
                    }
                    seen_leaf[lid] = true;
                    next_leaf += 1;
                }
                if node.0 != next_struct {
                    return false;
                }
                next_struct += 1;
            }
            Struct::List(children) => {
                if cursor == 0 {
                    if entered[node.0 as usize] {
                        // A node reached twice = a shared (non-tree) arena; the reader never produces
                        // one, so bail to the safe rebuild.
                        return false;
                    }
                    entered[node.0 as usize] = true;
                }
                if cursor < children.len() {
                    stack.push((node, cursor + 1));
                    stack.push((children[cursor], 0));
                } else {
                    if node.0 != next_struct {
                        return false;
                    }
                    next_struct += 1;
                }
            }
        }
    }
    next_struct as usize == arenas.structure.len() && next_leaf as usize == arenas.leaves.len()
}

struct Canon<'a> {
    src: &'a Arenas,
    leaves: Vec<Leaf>,
    leaf_map: FxHashMap<LeafId, LeafId>, // old leaf id -> new (first-encounter) leaf id
    structure: Vec<Struct>,
}

impl Canon<'_> {
    /// Visit an occurrence, emitting it (and its subtree) into the new arena, and return its new id.
    /// Children are visited before the parent is appended, so the parent's `StructId` is always
    /// greater than its children's — a post-order layout, deterministic from the tree shape alone.
    fn visit(&mut self, old: StructId) -> StructId {
        match self.src.get(old) {
            Struct::Atom(old_leaf) => {
                let leaf = self.intern(*old_leaf);
                self.push(Struct::Atom(leaf))
            }
            Struct::List(children) => {
                let kids: Vec<StructId> = children.iter().map(|&ch| self.visit(ch)).collect();
                self.push(Struct::List(kids))
            }
        }
    }

    /// Intern a leaf by first-encounter order during the walk.
    fn intern(&mut self, old: LeafId) -> LeafId {
        if let Some(&new) = self.leaf_map.get(&old) {
            return new;
        }
        let new = LeafId(self.leaves.len() as u32);
        self.leaves.push(self.src.leaf(old).clone());
        self.leaf_map.insert(old, new);
        new
    }

    fn push(&mut self, s: Struct) -> StructId {
        let id = StructId(self.structure.len() as u32);
        self.structure.push(s);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Builder;
    use crate::{codec, parser, sexpr};

    /// Two arenas built in DIFFERENT occurrence order but denoting the same tree encode to the same
    /// bytes (because `codec::encode` canonicalizes). Their canonical structure arenas are equal.
    #[test]
    fn build_order_independent() {
        // Build `(+ a b)` head-first (like the s-expr reader).
        let mut b1 = Builder::new();
        let plus = b1.name("+");
        let a = b1.name("a");
        let bb = b1.name("b");
        let root1 = b1.list(vec![plus, a, bb]);
        let head_first = b1.finish(root1);

        // Build `(+ a b)` left-operand-first (like the ML parser: a, then +, then b).
        let mut b2 = Builder::new();
        let a2 = b2.name("a");
        let plus2 = b2.name("+");
        let b2b = b2.name("b");
        let root2 = b2.list(vec![plus2, a2, b2b]);
        let operand_first = b2.finish(root2);

        // The raw structure arenas differ (different occurrence order)...
        assert_ne!(head_first.structure, operand_first.structure);
        // ...but their canonical forms — and hence their encoded bytes — are identical.
        assert_eq!(
            canonicalize(&head_first).structure,
            canonicalize(&operand_first).structure
        );
        assert_eq!(codec::encode(&head_first), codec::encode(&operand_first));
    }

    #[test]
    fn idempotent() {
        let a = sexpr::read("(let ((x 1) (y (+ x 1))) y)").unwrap();
        let c1 = canonicalize(&a);
        let c2 = canonicalize(&c1);
        assert_eq!(codec::encode(&c1), codec::encode(&c2));
    }

    #[test]
    fn preserves_structure() {
        // canonicalization must not change what the tree denotes.
        let a = sexpr::read("(f (+ a b) (g 1 2))").unwrap();
        assert!(canonicalize(&a).structurally_eq(&a));
    }

    #[test]
    fn sexpr_and_ml_agree_after_canon() {
        // The bug this fixes: an infix expression via the two surfaces.
        let src = "(let ((x 1) (y (+ x 1))) y)";
        let from_sexpr = sexpr::read(src).unwrap();
        let ml = crate::printer::print(&from_sexpr, 100);
        let from_ml = parser::read_ml(&ml).arenas;
        assert_eq!(
            codec::encode(&canonicalize(&from_sexpr)),
            codec::encode(&canonicalize(&from_ml)),
        );
    }
}
