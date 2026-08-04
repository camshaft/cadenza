//! The binary-AST DICTIONARY transport plane — types for the non-canonical `cdzast\x00\x02` wire.
//!
//! A dictionary lets a byte blob reference a shared subtree by (dict, node) INDEX instead of inlining
//! it — a compact TRANSPORT encoding (operator seq 119–121; see `implementation/design/
//! DESIGN-binary-ast-dictionary.md`). Per the operator's ruling (option A), a dict-bearing artifact is
//! NON-CANONICAL: the one canonical byte form of any AST stays the inline `cdzast\x00\x01` form, so the
//! frozen `ast-encoding.md` bijection + content-addressing are UNTOUCHED. These types live in their own
//! module, deliberately OUT of the canonical [`crate::ast::Struct`]/[`crate::ast::Arenas`], so the
//! identity `encode`/`canon` paths can never accidentally emit a dict reference — the transport plane is
//! reachable only through [`crate::codec::decode_with_dicts`] (and, later, `encode_with_dict`).
//!
//! A dictionary IS just another binary AST: a normal inline-canonical (`cdzast\x00\x01`, dict-free)
//! `Arenas`. Its identity is its content hash (the same 32-byte content-address used elsewhere). An
//! importable node is a `StructId` of the dictionary's own `structure` arena; a transport dict-ref
//! `{dict, node}` resolves to the subtree rooted at that node of the named dictionary.
//!
//! v1 dictionaries are FLAT: a dictionary's bytes MUST themselves be dict-free `cdzast\x00\x01`, so the
//! resolver is a single bounded expand pass with no cycle possible. Layered dictionaries are a clean
//! additive v2 extension (design §8).

use crate::ast::Arenas;
use std::collections::HashMap;

/// A 32-byte content-address — the identity of a dictionary (and the codebase's one content-hash width).
///
/// This is a VALUE type: it holds the raw 32 digest bytes an import names; it carries NO hashing
/// machinery (`cadenza-ast`, the bottom crate, never COMPUTES a hash — it only stores + compares the
/// bytes). The upper `cdz-kernel` crate's `Hash([u8; 32])` is byte-identical and re-exports / converts
/// to this; the type is rooted HERE because the wire format in the bottom crate must reference it and
/// `cdz-kernel` depends on `cadenza-ast`, not the reverse (design §9.1, corrected for dep direction —
/// the value is the identical 32-byte content-address, only the type's crate home moved).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// The raw 32 content-address bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A resolved set of importable dictionaries, keyed by content [`Hash`]. Supplied to the decoder as an
/// INPUT ARTIFACT (design/seq-120): [`crate::codec::decode_with_dicts`] makes NO external calls — a hash
/// a transport artifact imports but that is not present here is a hard [`crate::codec::DecodeError::
/// MissingDict`], never a fetch. Each value is a decoded, inline-canonical (flat, dict-free) `Arenas`;
/// resolving a dict-ref `{dict, node}` grafts the subtree rooted at `structure[node]` of the named dict.
#[derive(Clone, Debug, Default)]
pub struct DictSet {
    dicts: HashMap<Hash, Arenas>,
}

impl DictSet {
    /// An empty dict-set — resolving any import against it yields `MissingDict`. (A dict-FREE
    /// `cdzast\x00\x01` artifact decodes fine against an empty set, since it imports nothing.)
    pub fn new() -> DictSet {
        DictSet {
            dicts: HashMap::new(),
        }
    }

    /// Register a dictionary under its content hash. The caller is responsible for the hash being the
    /// content-address of `dict`'s canonical bytes and for `dict` being flat (dict-free) — the model
    /// layer (`v-metaprogramming`'s I3) validates those when building a `DictSet` from input artifacts;
    /// this bottom-crate container just stores the mapping the wire resolver looks up.
    pub fn insert(&mut self, hash: Hash, dict: Arenas) {
        self.dicts.insert(hash, dict);
    }

    /// The dictionary registered under `hash`, if any. `decode_with_dicts` uses this to resolve each
    /// import; a `None` is the hermetic-resolution failure (`MissingDict`).
    pub fn get(&self, hash: &Hash) -> Option<&Arenas> {
        self.dicts.get(hash)
    }

    /// Iterate `(hash, dict)` pairs — used by the transport ENCODER (`encode_with_dict`) to build its
    /// subtree-match table over every importable node of every supplied dictionary.
    pub fn iter(&self) -> impl Iterator<Item = (&Hash, &Arenas)> {
        self.dicts.iter()
    }

    /// How many dictionaries are registered.
    pub fn len(&self) -> usize {
        self.dicts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dicts.is_empty()
    }
}
