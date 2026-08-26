//! The WIRE types of the compiler's sidecar QUERY protocol — the `Query` request vocabulary, the
//! `KIND_*` result-artifact names, and the LEB128 request codec — with NO compiler dependency.
//!
//! Extracted from `rcdzc::sidecar` (`design/DESIGN-cdz-delegate-compile.md`) so a consumer can BUILD a
//! query request and NAME its result artifact without linking the compiler: `rcdzc::sidecar` re-exports
//! [`Query`] and the `KIND_*` constants from here (its engine code is unchanged) and delegates the
//! query-request codec arms to [`encode_query`]/[`decode_query`], so this crate is the SINGLE SOURCE of
//! the query wire form. The `cdz` binary uses [`encode_query_requests`] to build the sidecar input it
//! hands `cdz-compile`, and the `KIND_*` names to read the result artifact back.
//!
//! The byte layout MUST stay identical to `rcdzc::sidecar`'s historical `Request::Query` encoding (a
//! one-byte tag then LEB128-framed operands); a round-trip diff pins it. The LEB128 varint is the
//! standard unsigned encoding, carried here self-contained (it is a format, not a copied contract).

// ── Result-artifact kinds ────────────────────────────────────────────────────────────────────────
// The artifact KIND a sidecar request materializes its answer under (the name the consumer reads back).
// Kept here as the single source so both the compiler (which writes them) and a delegating driver (which
// reads them) agree. `rcdzc::sidecar` re-exports every one, so its uses are unchanged.

/// The INPUT kind: a request-list blob (this crate's [`encode_query_requests`]) handed to the compiler.
pub const KIND_SIDECAR: &str = "sidecar";
/// A definition's rendered type (`TypeOf`).
pub const KIND_TYPE_INFO: &str = "type-info";
/// The reference occurrences of a definition (`UsesOf`).
pub const KIND_USES: &str = "uses";
/// The rendered type of a node (`TypeAt`).
pub const KIND_TYPE_AT: &str = "type-at";
/// Every well-formedness fault (`Diagnostics`).
pub const KIND_DIAGNOSTICS: &str = "diagnostics";
/// The defining occurrence a reference resolves to (`ResolveOf`).
pub const KIND_RESOLVE: &str = "resolve";
/// The bindings visible at a node (`ScopeAt`).
pub const KIND_SCOPE: &str = "scope";
/// The module's exported names + types (`Exports`).
pub const KIND_EXPORTS: &str = "exports";
/// Semantic syntax-highlight classification per node (`Highlight`).
pub const KIND_HIGHLIGHT: &str = "highlight";
/// A definition's documentation (`DocOf`/`DocAt`).
pub const KIND_DOC: &str = "doc";
/// A definition's disposition + instantiations (`Instantiations`).
pub const KIND_INSTANTIATIONS: &str = "instantiations";
/// Every top-level declaration, classified (`Symbols`).
pub const KIND_SYMBOLS: &str = "symbols";
/// The `@param` widget manifest (`ParamManifest`).
pub const KIND_PARAM_MANIFEST: &str = "param-manifest";
/// The Option-C shared-closure content hash (`ClosureHash`).
pub const KIND_CLOSURE_HASH: &str = "closure-hash";
/// The emitted-function layout: func-index ↔ content-hash (`FuncLayout`).
pub const KIND_FUNC_LAYOUT: &str = "func-layout";
/// Each exported def's resolved type as a structured cdzast sub-AST (`ExportedTypes`).
pub const KIND_EXPORT_TYPES: &str = "export-types";

// ── The query vocabulary ─────────────────────────────────────────────────────────────────────────

/// A read of a fact column — the query half of the compiler's sidecar request vocabulary. Each arm
/// names the column it reads; the compiler's answer is TOTAL (a name/node with no answer yields a
/// defined "unknown"/empty result, never a crash). Node-id-keyed arms carry a `StructId.0` (`u32`); the
/// consumer holds the span table and maps the id to a source range. The detailed per-arm behavior lives
/// with the engine (`rcdzc::sidecar::run_query`); this is the wire vocabulary that selects it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Query {
    /// The solved type of a top-level definition, BY NAME (rendered). → `KIND_TYPE_INFO`.
    TypeOf { name: String },
    /// Every occurrence that RESOLVES to the named definition/sum type (referencing node ids). → `KIND_USES`.
    UsesOf { name: String },
    /// The solved type of a specific NODE, by `StructId` — "type at cursor". → `KIND_TYPE_AT`.
    TypeAt { node: u32 },
    /// The defining occurrence a reference node resolves to — go-to-definition. → `KIND_RESOLVE`.
    ResolveOf { node: u32 },
    /// Every well-formedness fault, without requiring export/emit — "diagnostics as you type". → `KIND_DIAGNOSTICS`.
    Diagnostics,
    /// The bindings visible at a node — variable scope tracking. → `KIND_SCOPE`.
    ScopeAt { node: u32 },
    /// Semantic syntax highlighting — every user node classified by role. → `KIND_HIGHLIGHT`.
    Highlight,
    /// The module's exports, each name paired with its type. → `KIND_EXPORTS`.
    Exports,
    /// Each exported definition's resolved type as a STRUCTURED cdzast sub-AST. → `KIND_EXPORT_TYPES`.
    ExportedTypes,
    /// The documentation of a definition/built-in, BY NAME. → `KIND_DOC`.
    DocOf { name: String },
    /// The documentation of the definition a node belongs to/references — "doc at cursor". → `KIND_DOC`.
    DocAt { node: u32 },
    /// A definition's disposition (specialized/inlined/emitted/…) + instantiations, BY NAME. → `KIND_INSTANTIATIONS`.
    Instantiations { name: String },
    /// The document outline — every top-level declaration, classified. → `KIND_SYMBOLS`.
    Symbols,
    /// The `@param` widget manifest — one record per well-typed `@param` site. → `KIND_PARAM_MANIFEST`.
    ParamManifest,
    /// The emitted-function layout — func-index ↔ AST content-hash, in func-index order. → `KIND_FUNC_LAYOUT`.
    FuncLayout,
    /// The Option-C shared-closure content hash for a `@test` dir — the provider-cache decision key. → `KIND_CLOSURE_HASH`.
    ClosureHash,
}

/// The one-byte query request tags. Stable + additive (a new query is a new tag); the values MATCH
/// `rcdzc::sidecar`'s historical `tag` module exactly (0x10.. is the query range; 0x00..=0x08 is the
/// compiler-owned Emit range, which stays in `rcdzc` and is never produced here).
mod tag {
    pub const QUERY_TYPE_OF: u8 = 0x10;
    pub const QUERY_USES_OF: u8 = 0x11;
    pub const QUERY_TYPE_AT: u8 = 0x12;
    pub const QUERY_DIAGNOSTICS: u8 = 0x13;
    pub const QUERY_RESOLVE_OF: u8 = 0x14;
    pub const QUERY_SCOPE_AT: u8 = 0x15;
    pub const QUERY_EXPORTS: u8 = 0x16;
    pub const QUERY_HIGHLIGHT: u8 = 0x17;
    pub const QUERY_DOC_OF: u8 = 0x18;
    pub const QUERY_DOC_AT: u8 = 0x19;
    pub const QUERY_INSTANTIATIONS: u8 = 0x1a;
    pub const QUERY_SYMBOLS: u8 = 0x1b;
    pub const QUERY_PARAM_MANIFEST: u8 = 0x1c;
    pub const QUERY_FUNC_LAYOUT: u8 = 0x1d;
    pub const QUERY_CLOSURE_HASH: u8 = 0x1e;
    pub const QUERY_EXPORTED_TYPES: u8 = 0x1f;
}

// ── Encode ───────────────────────────────────────────────────────────────────────────────────────

/// Encode a whole request list consisting of these queries to its sidecar-input wire bytes — the blob a
/// consumer hands the compiler as a `KIND_SIDECAR` input. Byte-identical to `rcdzc::sidecar::encode` of
/// the same queries wrapped as `Request::Query(_)`: a LEB128 count, then each query's [`encode_query`].
pub fn encode_query_requests(queries: &[Query]) -> Vec<u8> {
    let mut out = Vec::new();
    write_varu64(&mut out, queries.len() as u64);
    for q in queries {
        encode_query(&mut out, q);
    }
    out
}

/// Append ONE query's wire bytes (its tag, then LEB128-framed operands) to `out`. This is the single
/// source of the per-query encoding; `rcdzc::sidecar`'s `encode_one` calls it for the `Request::Query`
/// arm so the compiler and a delegating driver never drift.
pub fn encode_query(out: &mut Vec<u8>, q: &Query) {
    match q {
        Query::TypeOf { name } => {
            out.push(tag::QUERY_TYPE_OF);
            write_string(out, name);
        }
        Query::UsesOf { name } => {
            out.push(tag::QUERY_USES_OF);
            write_string(out, name);
        }
        Query::TypeAt { node } => {
            out.push(tag::QUERY_TYPE_AT);
            write_varu64(out, *node as u64);
        }
        Query::ResolveOf { node } => {
            out.push(tag::QUERY_RESOLVE_OF);
            write_varu64(out, *node as u64);
        }
        Query::Diagnostics => out.push(tag::QUERY_DIAGNOSTICS),
        Query::ScopeAt { node } => {
            out.push(tag::QUERY_SCOPE_AT);
            write_varu64(out, *node as u64);
        }
        Query::Highlight => out.push(tag::QUERY_HIGHLIGHT),
        Query::Exports => out.push(tag::QUERY_EXPORTS),
        Query::ExportedTypes => out.push(tag::QUERY_EXPORTED_TYPES),
        Query::DocOf { name } => {
            out.push(tag::QUERY_DOC_OF);
            write_string(out, name);
        }
        Query::DocAt { node } => {
            out.push(tag::QUERY_DOC_AT);
            write_varu64(out, *node as u64);
        }
        Query::Instantiations { name } => {
            out.push(tag::QUERY_INSTANTIATIONS);
            write_string(out, name);
        }
        Query::Symbols => out.push(tag::QUERY_SYMBOLS),
        Query::ParamManifest => out.push(tag::QUERY_PARAM_MANIFEST),
        Query::FuncLayout => out.push(tag::QUERY_FUNC_LAYOUT),
        Query::ClosureHash => out.push(tag::QUERY_CLOSURE_HASH),
    }
}

// ── Decode ───────────────────────────────────────────────────────────────────────────────────────

/// Decode ONE query given its already-read `tag` byte and a [`Reader`] positioned at its operands.
/// Returns `None` if `tag` is not a query tag (so `rcdzc::sidecar`'s `decode_one` can try its Emit tags
/// first, then delegate the rest here) or the operands are truncated. Total — never panics.
pub fn decode_query(tag: u8, r: &mut Reader) -> Option<Query> {
    Some(match tag {
        tag::QUERY_TYPE_OF => Query::TypeOf {
            name: read_string(r)?,
        },
        tag::QUERY_USES_OF => Query::UsesOf {
            name: read_string(r)?,
        },
        tag::QUERY_TYPE_AT => Query::TypeAt {
            node: u32::try_from(r.read_varu64()?).ok()?,
        },
        tag::QUERY_DIAGNOSTICS => Query::Diagnostics,
        tag::QUERY_RESOLVE_OF => Query::ResolveOf {
            node: u32::try_from(r.read_varu64()?).ok()?,
        },
        tag::QUERY_SCOPE_AT => Query::ScopeAt {
            node: u32::try_from(r.read_varu64()?).ok()?,
        },
        tag::QUERY_EXPORTS => Query::Exports,
        tag::QUERY_HIGHLIGHT => Query::Highlight,
        tag::QUERY_DOC_OF => Query::DocOf {
            name: read_string(r)?,
        },
        tag::QUERY_DOC_AT => Query::DocAt {
            node: u32::try_from(r.read_varu64()?).ok()?,
        },
        tag::QUERY_INSTANTIATIONS => Query::Instantiations {
            name: read_string(r)?,
        },
        tag::QUERY_SYMBOLS => Query::Symbols,
        tag::QUERY_PARAM_MANIFEST => Query::ParamManifest,
        tag::QUERY_FUNC_LAYOUT => Query::FuncLayout,
        tag::QUERY_CLOSURE_HASH => Query::ClosureHash,
        tag::QUERY_EXPORTED_TYPES => Query::ExportedTypes,
        _ => return None,
    })
}

// ── LEB128 varint + framing (self-contained; the standard unsigned encoding) ───────────────────────

/// Append the unsigned LEB128 encoding of `value` to `out`.
pub fn write_varu64(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// A length-prefixed UTF-8 string: a LEB128 byte length, then the bytes.
fn write_string(out: &mut Vec<u8>, s: &str) {
    write_varu64(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn read_string(r: &mut Reader) -> Option<String> {
    let len = r.read_var_len()?;
    let bytes = r.take(len)?;
    String::from_utf8(bytes.to_vec()).ok()
}

/// A cursor reading out of a byte slice, tracking position. Reads are total — never panic (decode
/// operates on untrusted external bytes). The subset of `rcdzc::leb128::Reader` the query codec needs.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, pos: 0 }
    }

    /// True once every byte has been consumed.
    pub fn at_end(&self) -> bool {
        self.pos == self.bytes.len()
    }

    /// Read one raw byte, or `None` at end of input.
    pub fn byte(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    /// Read `n` raw bytes as a slice, or `None` if fewer than `n` remain.
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    /// Read a `VarU64` (unsigned LEB128). `None` on truncation or a value wider than 64 bits.
    pub fn read_varu64(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.byte()?;
            if shift >= 64 {
                return None;
            }
            let payload = (byte & 0x7f) as u64;
            if shift == 63 && payload > 1 {
                return None;
            }
            result |= payload << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
        }
    }

    /// Read a `VarU64` and narrow to `usize`. `None` if it exceeds `usize`.
    pub fn read_var_len(&mut self) -> Option<usize> {
        usize::try_from(self.read_varu64()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One of each query variant, exercising every operand shape (string / node / nullary).
    fn every_query() -> Vec<Query> {
        vec![
            Query::TypeOf { name: "foo".into() },
            Query::UsesOf { name: "bar".into() },
            Query::TypeAt { node: 42 },
            Query::ResolveOf { node: 7 },
            Query::Diagnostics,
            Query::ScopeAt { node: 300 },
            Query::Highlight,
            Query::Exports,
            Query::ExportedTypes,
            Query::DocOf { name: "baz".into() },
            Query::DocAt { node: 0 },
            Query::Instantiations { name: "qux".into() },
            Query::Symbols,
            Query::ParamManifest,
            Query::FuncLayout,
            Query::ClosureHash,
        ]
    }

    /// Decode a request list produced by [`encode_query_requests`] back to its queries — the driver
    /// never decodes (it only sends), but the compiler side does, so this mirrors that path for the test.
    fn decode_query_requests(bytes: &[u8]) -> Option<Vec<Query>> {
        let mut r = Reader::new(bytes);
        let count = r.read_var_len()?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let t = r.byte()?;
            out.push(decode_query(t, &mut r)?);
        }
        if !r.at_end() {
            return None;
        }
        Some(out)
    }

    #[test]
    fn every_query_round_trips_through_the_request_list() {
        let qs = every_query();
        let bytes = encode_query_requests(&qs);
        assert_eq!(
            decode_query_requests(&bytes).as_deref(),
            Some(qs.as_slice())
        );
    }

    #[test]
    fn per_query_encode_matches_the_documented_tag_and_framing() {
        // Pin the exact bytes for a representative of each operand shape, so a drift from
        // rcdzc::sidecar's historical layout fails here (the byte-identity invariant v-inference asked
        // to keep). A node varint is LEB128; a string is LEB128-len then UTF-8; a nullary is just a tag.
        let mut b = Vec::new();
        encode_query(&mut b, &Query::TypeAt { node: 300 });
        assert_eq!(b, vec![0x12, 0xac, 0x02]); // tag 0x12, then LEB128(300) = [0xac, 0x02]

        let mut b = Vec::new();
        encode_query(&mut b, &Query::TypeOf { name: "hi".into() });
        assert_eq!(b, vec![0x10, 0x02, b'h', b'i']); // tag 0x10, len 2, "hi"

        let mut b = Vec::new();
        encode_query(&mut b, &Query::Diagnostics);
        assert_eq!(b, vec![0x13]); // nullary: tag only
    }

    #[test]
    fn decode_of_a_non_query_tag_is_none() {
        // An Emit tag (compiler-owned, never produced here) is not a query — decode declines it so the
        // compiler's decoder can handle its own range first.
        assert_eq!(decode_query(0x00, &mut Reader::new(&[])), None);
        assert_eq!(decode_query(0x05, &mut Reader::new(&[])), None);
    }

    #[test]
    fn truncated_operand_is_none_not_panic() {
        // tag says TypeOf (needs a string) but no length/bytes follow.
        assert_eq!(
            decode_query(tag::QUERY_TYPE_OF, &mut Reader::new(&[])),
            None
        );
    }
}
