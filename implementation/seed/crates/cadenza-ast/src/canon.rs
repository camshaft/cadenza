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
        id_map: Vec::new(), // not tracked on this path — see `canonicalize_with_map`
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
pub fn is_canonical(arenas: &Arenas) -> bool {
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

/// Canonicalize `arenas` AND return the OLD→NEW structure-id map — `map[old.0] == Some(new_id)` for
/// every reachable node, `None` for an unreachable one (dropped by the normal form). A caller holding a
/// side table keyed by the OLD ids (a SPAN TABLE) remaps it through this so its ids match the canonical
/// arena — which is what [`crate::codec::encode`] serializes, so they then match the COMPILER's decoded
/// node ids. This is the fix for the ML surface: `read_ml` builds nodes in a non-canonical order, so
/// without the remap a span-table lookup by a compiler-reported node id lands on the wrong node
/// (`ml-parser-node-order`). Always rebuilds (returns owned) — the map IS the point; an already-canonical
/// arena just gets the identity map.
pub fn canonicalize_with_map(arenas: &Arenas) -> (Arenas, Vec<Option<StructId>>) {
    let mut c = Canon {
        src: arenas,
        leaves: Vec::new(),
        leaf_map: FxHashMap::default(),
        structure: Vec::new(),
        id_map: vec![None; arenas.structure.len()],
    };
    let root = c.visit(arenas.root);
    (
        Arenas {
            leaves: c.leaves,
            structure: c.structure,
            root,
        },
        c.id_map,
    )
}

struct Canon<'a> {
    src: &'a Arenas,
    leaves: Vec<Leaf>,
    leaf_map: FxHashMap<LeafId, LeafId>, // old leaf id -> new (first-encounter) leaf id
    structure: Vec<Struct>,
    /// old structure id -> new id, recorded as each node is emitted (for span-table remap). Empty when
    /// the caller does not need it (`canonicalize`), non-empty for `canonicalize_with_map`.
    id_map: Vec<Option<StructId>>,
}

impl Canon<'_> {
    /// Visit an occurrence, emitting it (and its subtree) into the new arena, and return its new id.
    /// Children are visited before the parent is appended, so the parent's `StructId` is always
    /// greater than its children's — a post-order layout, deterministic from the tree shape alone.
    ///
    /// EXPLICIT stack, not native recursion: canonicalize runs on arenas from ANY source — a decoded
    /// binary AST in particular, which `codec::decode` accepts at ARBITRARY nesting depth (no cap,
    /// unlike the reader's `MAX_NESTING_DEPTH`). It runs on essentially EVERY decode (the compiler's
    /// load path, and cdz-wasm's `parse_spanned` → `canonicalize_with_map`), so a recursive walk would
    /// overflow the native stack (SIGABRT) on a deep-but-valid tree — crashing the process. A `Job`
    /// work-stack plus a `results` stack preserves the recursive version's EXACT observable order (leaf
    /// interning in pre-order left-to-right; structure push in post-order; id_map recorded per node), so
    /// the canonical bytes are byte-identical — which `is_canonical`'s replay-walk depends on.
    fn visit(&mut self, root: StructId) -> StructId {
        enum Job {
            Visit(StructId),
            // Emit a `List` for `old` once its `n` children's new ids sit atop `results`.
            EmitList(StructId, usize),
        }
        let mut jobs: Vec<Job> = vec![Job::Visit(root)];
        let mut results: Vec<StructId> = Vec::new();
        while let Some(job) = jobs.pop() {
            match job {
                Job::Visit(old) => match self.src.get(old) {
                    Struct::Atom(old_leaf) => {
                        // Atom: intern (first-encounter order) + push + record + result — immediately.
                        let leaf = self.intern(*old_leaf);
                        let new = self.push(Struct::Atom(leaf));
                        if let Some(slot) = self.id_map.get_mut(old.0 as usize) {
                            *slot = Some(new);
                        }
                        results.push(new);
                    }
                    Struct::List(children) => {
                        // Defer the parent's emit until after its children; push children in REVERSE so
                        // they pop (and thus intern/emit) left-to-right — matching the recursive order.
                        jobs.push(Job::EmitList(old, children.len()));
                        for &ch in children.iter().rev() {
                            jobs.push(Job::Visit(ch));
                        }
                    }
                },
                Job::EmitList(old, n) => {
                    // The n child results sit on top in reverse (last child deepest-pushed pops last →
                    // ends up last in `results`); split them off and they are already in source order.
                    let kids = results.split_off(results.len() - n);
                    let new = self.push(Struct::List(kids));
                    if let Some(slot) = self.id_map.get_mut(old.0 as usize) {
                        *slot = Some(new);
                    }
                    results.push(new);
                }
            }
        }
        // Exactly the root's new id remains.
        results.pop().expect("visit leaves the root's new id")
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
