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
use crate::codec::{self, DecodeError};
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

    /// Build a `DictSet` from SUPPLIED input artifacts — the MODEL-layer entry point (design §5, I3) the
    /// compiler front uses to turn a bag of `(content-hash, bytes)` dictionary artifacts into the resolved
    /// set [`crate::codec::decode_with_dicts`] reads.
    ///
    /// HERMETIC (design/seq-120): the builder validates ONLY the bytes it is GIVEN — it never reads a path,
    /// opens a store, or fetches a hash. A `TAG_DICT_REF` that a later `resolve` cannot satisfy from this
    /// set is a clean [`DecodeError::MissingDict`], never an external lookup.
    ///
    /// Each artifact is validated to be a FLAT, inline-canonical dictionary before it is registered:
    /// - **flat** — a v1 dictionary's bytes MUST themselves be dict-free canonical `cdzast\x00\x01`; a
    ///   dict-BEARING (`cdzast\x00\x02`) artifact is rejected [`DictError::DictNotFlat`]. This is enforced
    ///   for free by decoding through [`crate::codec::decode`] / `decode_detailed`, which REFUSE the
    ///   transport header — so the resolver stays a single bounded expand pass with no cycle possible.
    /// - **decodable** — a byte string that is not a well-formed canonical AST is [`DictError::BadArtifact`]
    ///   (the underlying [`DecodeError`] is carried for diagnostics).
    ///
    /// The `hash` is the artifact's content address; the CALLER computes it (this bottom crate stores +
    /// compares hashes but never COMPUTES one — the digest lives in exactly one place, `cdz-kernel`, §9.1).
    /// [`resolve`] grafts a dict-ref by this exact hash, so the caller MUST pass the content-address of
    /// the SAME canonical bytes it (or `encode_with_dict`) will reference. A duplicate hash keeps the FIRST artifact
    /// (later duplicates of an already-registered hash are ignored — content-addressed, so equal-hash
    /// artifacts are equal by construction).
    pub fn from_artifacts<'a, I>(artifacts: I) -> Result<DictSet, DictError>
    where
        I: IntoIterator<Item = (Hash, &'a [u8])>,
    {
        let mut set = DictSet::new();
        for (hash, bytes) in artifacts {
            if set.dicts.contains_key(&hash) {
                continue; // content-addressed: an equal hash is an equal artifact — keep the first.
            }
            // `decode_detailed` accepts ONLY the canonical `\x00\x01` plane and REFUSES the `\x00\x02`
            // transport header with `BadHeader`. A dict-bearing artifact therefore fails here — which is
            // exactly the v1 "dictionaries must be flat" rule, enforced without a bespoke check. To give a
            // PRECISE diagnostic, distinguish a genuine transport artifact (its own header) from corruption.
            match codec::decode_detailed(bytes) {
                Ok(arenas) => set.insert(hash, arenas),
                Err(DecodeError::BadHeader) if is_transport_header(bytes) => {
                    return Err(DictError::DictNotFlat(hash));
                }
                Err(e) => return Err(DictError::BadArtifact(hash, e)),
            }
        }
        Ok(set)
    }
}

/// Whether `bytes` begins with the DICTIONARY TRANSPORT header (`cdzast\x00\x02`) — a dict-bearing,
/// non-canonical artifact. Used by [`DictSet::from_artifacts`] to tell a v1-flat VIOLATION (a real
/// transport artifact supplied where a flat dict is required) apart from ordinary corruption, so the
/// caller gets [`DictError::DictNotFlat`] rather than a generic bad-artifact error.
fn is_transport_header(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && &bytes[..8] == b"cdzast\x00\x02"
}

/// Why [`DictSet::from_artifacts`] rejected a supplied dictionary artifact. Each variant names the
/// offending artifact by its content [`Hash`] so a caller can report WHICH input was at fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DictError {
    /// The artifact is itself dict-BEARING (`cdzast\x00\x02`), but v1 dictionaries MUST be flat
    /// (dict-free `cdzast\x00\x01`) so resolution is a single bounded pass with no possible cycle
    /// (design §5, §8). Layered dictionaries are a future additive v2 extension.
    DictNotFlat(Hash),
    /// The artifact's bytes are not a well-formed canonical AST — the carried [`DecodeError`] says how
    /// (truncated, bad tag, out-of-range id, not-a-tree, trailing bytes, …).
    BadArtifact(Hash, DecodeError),
}

/// Resolve a possibly-dict-bearing TRANSPORT artifact against a supplied [`DictSet`], handing back a
/// NORMAL dict-free canonical [`Arenas`] the rest of the compiler consumes unchanged — the model-layer
/// "resolve then hand the compiler a plain AST" entry point (design §5, I3).
///
/// This is a thin, intention-revealing wrapper over [`crate::codec::decode_with_dicts`]: a canonical
/// `cdzast\x00\x01` input decodes identically (dicts unused); a `cdzast\x00\x02` transport input has
/// every `TAG_DICT_REF` grafted from `dicts` and the result is a plain inline `Arenas` — the SAME arena
/// the fully-inlined form would decode to (`decode_with_dicts(encode_with_dict(a, d), d) ==
/// canonicalize(a)`; hence `encode(resolve(encode_with_dict(a, d), d)) == encode(a)`, byte-identical). A
/// `TAG_DICT_REF` naming a hash absent from `dicts` is a clean [`DecodeError::MissingDict`] — HERMETIC,
/// never a fetch.
pub fn resolve(bytes: &[u8], dicts: &DictSet) -> Result<Arenas, DecodeError> {
    codec::decode_with_dicts(bytes, dicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Builder;
    use crate::codec::{encode, encode_with_dict};

    /// The shared subtree a dictionary exports: `(pair a b)`.
    fn dict_arena() -> Arenas {
        let mut b = Builder::new();
        let pair = b.name("pair");
        let sym_a = b.name("a");
        let sym_b = b.name("b");
        let root = b.list(vec![pair, sym_a, sym_b]);
        b.finish(root)
    }

    /// A tree that CONTAINS the dict's `(pair a b)` subtree, so `encode_with_dict` will compact it to a
    /// dict-ref: `(f (pair a b))`.
    fn program_using_dict() -> Arenas {
        let mut b = Builder::new();
        let f = b.name("f");
        let pair = b.name("pair");
        let sym_a = b.name("a");
        let sym_b = b.name("b");
        let inner = b.list(vec![pair, sym_a, sym_b]);
        let root = b.list(vec![f, inner]);
        b.finish(root)
    }

    #[test]
    fn from_artifacts_registers_a_flat_dictionary_keyed_by_its_supplied_hash() {
        // The happy path: a flat, inline-canonical `\x00\x01` dictionary artifact is accepted and stored
        // under the caller-supplied content hash, ready for `resolve` to graft from.
        let dict = dict_arena();
        let bytes = encode(&dict);
        let hash = Hash([0x11u8; 32]);
        let set = DictSet::from_artifacts([(hash, bytes.as_slice())]).expect("flat dict accepted");
        assert_eq!(set.len(), 1);
        assert!(
            set.get(&hash)
                .expect("registered under its hash")
                .structurally_eq(&dict),
            "the registered arena must equal the decoded dictionary"
        );
    }

    #[test]
    fn from_artifacts_rejects_a_dict_bearing_dictionary_as_not_flat() {
        // v1 dictionaries MUST be flat (design §5): a supplied artifact that is itself dict-BEARING
        // (`\x00\x02`, carrying its own dict-refs) is rejected `DictNotFlat`, naming the offending hash —
        // it is NOT silently registered, so the resolver can never face a layered dict / cycle.
        // Build a real `\x00\x02` transport artifact via the honest encoder.
        let dict = dict_arena();
        let dict_hash = Hash([0x11u8; 32]);
        let base = DictSet::from_artifacts([(dict_hash, encode(&dict).as_slice())]).unwrap();
        let transport = encode_with_dict(&program_using_dict(), &base);
        assert!(
            is_transport_header(&transport),
            "sanity: encode_with_dict produced a transport artifact"
        );

        // Now try to register THAT dict-bearing artifact as a dictionary — must be rejected.
        let bad_hash = Hash([0x22u8; 32]);
        assert_eq!(
            DictSet::from_artifacts([(bad_hash, transport.as_slice())]).err(),
            Some(DictError::DictNotFlat(bad_hash)),
            "a dict-bearing artifact must be rejected as not-flat, naming its hash"
        );
    }

    #[test]
    fn from_artifacts_rejects_a_malformed_artifact_as_bad() {
        // A byte string that is not a well-formed canonical AST is BadArtifact (carrying the DecodeError),
        // distinct from the not-flat case: too-short bytes are Truncated.
        let bad_hash = Hash([0x33u8; 32]);
        let err = DictSet::from_artifacts([(bad_hash, b"nope".as_slice())])
            .expect_err("garbage bytes are not a dictionary");
        match err {
            DictError::BadArtifact(h, _) => assert_eq!(h, bad_hash, "names the bad artifact"),
            other => panic!("expected BadArtifact, got {other:?}"),
        }
    }

    #[test]
    fn from_artifacts_keeps_the_first_of_a_duplicate_hash() {
        // Content-addressed: a repeated hash is a repeated (equal) artifact — the builder keeps the first
        // and does not error, so a caller can pass overlapping artifact bags harmlessly.
        let dict = dict_arena();
        let bytes = encode(&dict);
        let hash = Hash([0x11u8; 32]);
        let set = DictSet::from_artifacts([(hash, bytes.as_slice()), (hash, bytes.as_slice())])
            .expect("duplicate hash is fine");
        assert_eq!(set.len(), 1, "the duplicate is folded, not doubled");
    }

    #[test]
    fn resolve_yields_an_arena_re_encoding_byte_identical_to_the_inline_form() {
        // THE model-layer identity gate (design §5): resolving a dict-bearing transport artifact against
        // the dicts it was encoded with hands back a plain `Arenas` whose canonical re-encoding is
        // BYTE-IDENTICAL to the fully-inline encoding of the original program. Dict indirection erases at
        // resolve — a resolved program is indistinguishable from one that never used a dictionary.
        let program = program_using_dict();
        let dict = dict_arena();
        let dict_hash = Hash([0x11u8; 32]);

        let dicts = DictSet::from_artifacts([(dict_hash, encode(&dict).as_slice())]).unwrap();
        let transport = encode_with_dict(&program, &dicts);
        assert!(
            is_transport_header(&transport),
            "sanity: is a transport artifact"
        );

        let resolved = resolve(&transport, &dicts).expect("resolve grafts the dict-refs");
        assert_eq!(
            encode(&resolved),
            encode(&program),
            "resolve(encode_with_dict(a, d), d) must re-encode byte-identical to encode(a)"
        );
        assert!(
            resolve(&transport, &dicts)
                .unwrap()
                .structurally_eq(&program),
            "and be structurally equal to the inline program"
        );
    }

    #[test]
    fn resolve_of_a_missing_import_is_missing_dict() {
        // Hermetic (design/seq-120): if the dicts an artifact references are NOT supplied, resolve is a
        // clean MissingDict(hash) — never a fetch, never a panic. Encode against a dict, then resolve
        // against an EMPTY set.
        let dict = dict_arena();
        let dict_hash = Hash([0x11u8; 32]);
        let dicts = DictSet::from_artifacts([(dict_hash, encode(&dict).as_slice())]).unwrap();
        let transport = encode_with_dict(&program_using_dict(), &dicts);

        assert_eq!(
            resolve(&transport, &DictSet::new()),
            Err(DecodeError::MissingDict(dict_hash)),
            "an unsatisfied import must be MissingDict naming the absent hash"
        );
    }

    #[test]
    fn resolve_of_a_canonical_artifact_is_just_decode() {
        // A plain `\x00\x01` artifact resolves identically with no dicts — the entry point is a superset
        // of ordinary decode.
        let program = program_using_dict();
        let v1 = encode(&program);
        let resolved = resolve(&v1, &DictSet::new()).expect("v1 resolves");
        assert!(resolved.structurally_eq(&program));
        assert_eq!(encode(&resolved), v1, "canonical in, byte-identical out");
    }
}
