//! Structure-aware AST mutation (S5) — the heart of "use the binary AST as the entropy, it generates
//! a lot better".
//!
//! Byte-flipping a seed's binary-AST bytes almost always yields garbage the strict codec rejects (the
//! decode-gate skips it), so a byte-level mutator spends its budget producing undecodable inputs.
//! Mutating at the STRUCTURE level instead — on the decoded node tree — keeps the result a well-formed
//! AST *by construction*, so every mutant reaches the compiler. Combined with the semantics-corpus
//! seeds (S2), this drives dense, valid-ish programs into the backend, where the bug clusters live; the
//! crash / invalid-wasm oracle (S1, [`crate::oracle::compile_catching_ast`]) judges each mutant.
//!
//! The mutations here are GENERIC (they operate on the `Struct` arena, not on any specific construct),
//! so they compose with any seed and never need to understand the language: dropping or duplicating a
//! child of a form reshapes arity, argument lists, `do`-sequences, match arms, etc. — exactly the
//! structural edges a hand-written corpus never enumerates. Each mutation is driven by an entropy
//! `&[u8]`, so the same bytes reproduce the same mutant (libFuzzer / a PRNG driver can steer them).

use cadenza_syntax::ast::{Arenas, Builder, Struct, StructId};

/// A deterministic byte cursor: yields 0 once the entropy is spent, so a short seed still terminates
/// and every choice is reproducible from the bytes.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Cursor<'a> {
        Cursor { bytes, pos: 0 }
    }
    fn byte(&mut self) -> u8 {
        let v = self.bytes.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        v
    }
    /// A choice in `0..n` (0 when `n == 0`), using two bytes so it spans arenas larger than 256.
    fn pick(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let hi = self.byte() as usize;
        let lo = self.byte() as usize;
        ((hi << 8) | lo) % n
    }
}

/// Which structural edit to apply to the chosen node's children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// Remove one child (after the head) — reshapes arity / shortens a sequence or arg list.
    DropChild,
    /// Duplicate one child (after the head) — repeats an argument / sequence element / arm.
    DupChild,
}

/// A node is a mutation CANDIDATE iff it is a `List` with at least one child beyond the head (index 0),
/// i.e. `len >= 2` — a `(head x …)` form we can drop-from or duplicate-within.
fn is_candidate(arenas: &Arenas, id: StructId) -> bool {
    matches!(arenas.get(id), Struct::List(kids) if kids.len() >= 2)
}

/// Apply ONE structural mutation to `seed`, chosen and located by `entropy`, returning the mutated AST.
///
/// Returns `None` when the seed has no mutable node (e.g. a bare atom, or a form with only a head) — the
/// caller treats that as "this seed is not mutable" and moves on. The result is always a well-formed
/// tree (it is rebuilt through a [`Builder`]), so it round-trips the codec and reaches the compiler.
pub fn mutate(seed: &Arenas, entropy: &[u8]) -> Option<Arenas> {
    // Enumerate candidate nodes in a stable order (ascending id), so `entropy` selects reproducibly.
    let candidates: Vec<StructId> = (0..seed.structure.len())
        .map(|i| StructId(i as u32))
        .filter(|&id| is_candidate(seed, id))
        .collect();
    if candidates.is_empty() {
        return None;
    }

    let mut cur = Cursor::new(entropy);
    let op = if cur.byte().is_multiple_of(2) {
        Op::DropChild
    } else {
        Op::DupChild
    };
    let target = candidates[cur.pick(candidates.len())];

    // Choose which non-head child (index in 1..len) to act on — read BEFORE the rebuild so it is a pure
    // function of `entropy` regardless of traversal order.
    let target_len = match seed.get(target) {
        Struct::List(kids) => kids.len(),
        Struct::Atom(_) => unreachable!("candidates are lists"),
    };
    let child_slot = 1 + cur.pick(target_len - 1); // in 1..target_len

    let mut b = Builder::new();
    let root = copy_apply(seed, seed.root, target, op, child_slot, &mut b);
    Some(b.finish(root))
}

/// Recursively copy `src`'s subtree at `id` into `b`. At the `target` node, apply `op` to its child
/// list (drop or duplicate the child at `child_slot`) — every other node is copied verbatim.
fn copy_apply(
    src: &Arenas,
    id: StructId,
    target: StructId,
    op: Op,
    child_slot: usize,
    b: &mut Builder,
) -> StructId {
    match src.get(id) {
        Struct::Atom(leaf) => b.atom_leaf(src.leaf(*leaf).clone()),
        Struct::List(kids) => {
            let mut new: Vec<StructId> = kids
                .clone()
                .into_iter()
                .map(|k| copy_apply(src, k, target, op, child_slot, b))
                .collect();
            if id == target {
                match op {
                    Op::DropChild => {
                        new.remove(child_slot);
                    }
                    Op::DupChild => {
                        // Re-copy the chosen child so the duplicate is a distinct occurrence (the AST is
                        // a tree: each node is referenced once).
                        let dup = copy_apply(src, kids[child_slot], target, op, child_slot, b);
                        new.insert(child_slot, dup);
                    }
                }
            }
            b.list(new)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ast(source: &str) -> Arenas {
        cadenza_syntax::sexpr::read(source).expect("test source parses")
    }

    /// The mutant is always a well-formed AST that round-trips the codec (mutation stays in-grammar).
    fn assert_roundtrips(a: &Arenas) {
        let bytes = cadenza_syntax::codec::encode(a);
        assert!(
            cadenza_syntax::codec::decode(&bytes).is_some(),
            "mutant must re-encode to a decodable AST"
        );
    }

    #[test]
    fn drop_child_removes_one_child_and_stays_well_formed() {
        // `(+ 1 2 3)` has head `+` and 3 children; entropy picks DropChild (even first byte).
        let seed = ast("(+ 1 2 3)");
        // first byte even → DropChild; remaining bytes select the target node + child slot.
        let m = mutate(&seed, &[0, 0, 0, 0, 0, 0]).expect("mutable");
        assert_roundtrips(&m);
        // The root list should now have one fewer child than the seed's root.
        let Struct::List(seed_kids) = seed.get(seed.root) else {
            panic!()
        };
        let Struct::List(m_kids) = m.get(m.root) else {
            panic!()
        };
        assert_eq!(m_kids.len(), seed_kids.len() - 1);
    }

    #[test]
    fn dup_child_adds_one_child_and_stays_well_formed() {
        let seed = ast("(+ 1 2 3)");
        // first byte odd → DupChild.
        let m = mutate(&seed, &[1, 0, 0, 0, 0, 0]).expect("mutable");
        assert_roundtrips(&m);
        let Struct::List(seed_kids) = seed.get(seed.root) else {
            panic!()
        };
        let Struct::List(m_kids) = m.get(m.root) else {
            panic!()
        };
        assert_eq!(m_kids.len(), seed_kids.len() + 1);
    }

    #[test]
    fn mutation_is_deterministic_in_the_entropy() {
        let seed = ast("(do (def (main) (+ 1 2)) (export main))");
        let e = [3u8, 7, 9, 11, 13, 21, 5];
        let a = mutate(&seed, &e).expect("mutable");
        let b = mutate(&seed, &e).expect("mutable");
        assert_eq!(
            cadenza_syntax::codec::encode(&a),
            cadenza_syntax::codec::encode(&b),
            "same seed + same entropy → identical mutant"
        );
    }

    #[test]
    fn a_bare_atom_seed_has_nothing_to_mutate() {
        // A single leaf (no list with children) → no candidate → None.
        assert!(mutate(&ast("42"), &[0, 0, 0]).is_none());
    }

    #[test]
    fn a_head_only_form_is_not_a_candidate() {
        // `(main)` is a list of length 1 (head only) → not mutable on its own; but a nested seed with a
        // longer form IS mutable. `(do (main))` → `do` has 1 child `(main)`; `(main)` has 0 children;
        // `do` list len 2 IS a candidate (drop/dup its single child).
        assert!(mutate(&ast("(main)"), &[0, 0, 0]).is_none());
        assert!(mutate(&ast("(do (main))"), &[0, 0, 0]).is_some());
    }

    /// A real corpus-shaped program mutates into a still-decodable program (the whole point: mutants
    /// reach the compiler, unlike byte-flips of the strict codec).
    #[test]
    fn a_realistic_program_mutates_into_a_decodable_program() {
        let seed = ast(
            "(do (def (f n) (if (<= n 0) 0 (+ n (f (- n 1))))) (def (main) (f 5)) (export main))",
        );
        for e in [
            [0u8, 0, 1, 0, 2, 0],
            [1, 0, 3, 0, 1, 0],
            [0, 0, 5, 0, 4, 0],
            [1, 1, 2, 3, 4, 5],
        ] {
            let m = mutate(&seed, &e).expect("mutable");
            assert_roundtrips(&m);
        }
    }
}
