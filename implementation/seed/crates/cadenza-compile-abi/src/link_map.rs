//! The link-map RESULT wire — the diagnostics DEMUX table a linked package emits (`KIND_LINK_MAP`
//! artifact) + its `FileSpan` table and `encode`/`decode` codec. A compile-BOUNDARY concern: a
//! consumer (`cdz check`, the error reporter) reads it to demux a GLOBAL merged `StructId` back to
//! `(file, local id)` for source mapping, so it must speak this format WITHOUT linking `rcdzc`. The
//! LINKER itself (`link()`, `Linkage`, `LinkedProgram`) stays in `rcdzc::link`, which `pub use`s these
//! so `crate::link::{KIND_LINK_MAP, FileSpan, encode_link_map, decode_link_map}` stay byte-stable.

use cadenza_ast::ast::{Builder, IntValue, Leaf, Radix, Struct, StructId};

/// The OUTPUT-artifact kind carrying the diagnostics DEMUX table for a linked package
/// (`DESIGN-package-linking.md` §6). A cross-file diagnostic's `node` is a GLOBAL merged `StructId`;
/// with several files spliced into one arena, that global id no longer maps to a single file's span
/// table. This artifact lets a consumer demux: a global node `n` falls in exactly one file's
/// `[struct_base, struct_base+struct_count)` range → `(path, n - struct_base)` = the per-file LOCAL id
/// that file's own span table (or its `spans` input) is keyed by. The payload is the canonical BINARY
/// AST (`cadenza_ast::codec`) — a list of per-file `(list [Str path, Int base, Int count])` forms — the
/// SAME wire every compile-boundary artifact speaks (operator seq-254: binary AST everywhere, no
/// bespoke encodings), so a consumer decodes it with the one shared codec. A consumer-side two-level
/// back-reference (global id → (file, local id) → span), so the compiler stays span-free. Present only
/// for a multi-file package.
pub const KIND_LINK_MAP: &str = "link-map";

/// Encode a package's `FileSpan` table as the `link-map` artifact bytes — the canonical BINARY AST
/// (`cadenza_ast::codec`): a root `Ast.List` of per-file forms, each a `(list [Str path, Int base, Int
/// count])`, in splice order (operator seq-254: binary AST everywhere — no bespoke text format). The
/// inverse a consumer applies: find the file whose `[base, base+count)` contains a diagnostic's global
/// node id, then subtract `base` for the per-file local id. Round-trips with [`decode_link_map`].
pub fn encode_link_map(files: &[FileSpan]) -> Vec<u8> {
    let mut b = Builder::new();
    let forms: Vec<StructId> = files
        .iter()
        .map(|f| {
            let path = b.atom_leaf(Leaf::Str(f.path.as_str().into()));
            let base = int_leaf(&mut b, f.struct_base);
            let count = int_leaf(&mut b, f.struct_count);
            b.list(vec![path, base, count])
        })
        .collect();
    let root = b.list(forms);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `link-map` artifact bytes back into its `FileSpan` list — the inverse of
/// [`encode_link_map`], so a CLI reporter can demux a linked-package diagnostic's GLOBAL node id to the
/// `(file, local id)` its per-file span table is keyed by. Reads the canonical BINARY AST via the shared
/// `cadenza_ast::codec` (root `Ast.List` of `(list [Str path, Int base, Int count])`). TOTAL: a malformed
/// tree / wrong-shape or out-of-range entry yields an EMPTY table (a skipped entry) rather than failing —
/// the reporter degrades to a location-less diagnostic, never a crash.
pub fn decode_link_map(bytes: &[u8]) -> Vec<FileSpan> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Struct::List(forms) = a.get(a.root).clone() else {
        return Vec::new();
    };
    forms
        .iter()
        .filter_map(|&f| {
            let Struct::List(cols) = a.get(f) else {
                return None;
            };
            let path = a.as_str(*cols.first()?)?.to_string();
            let base = u32::try_from(a.as_int(*cols.get(1)?)?.to_i64()?).ok()?;
            let count = u32::try_from(a.as_int(*cols.get(2)?)?.to_i64()?).ok()?;
            Some(FileSpan {
                path,
                struct_base: base,
                struct_count: count,
            })
        })
        .collect()
}

/// An `Ast.Int` (decimal) leaf for a `FileSpan`'s `struct_base`/`struct_count` — the same atom the
/// sidecar codec uses for node-id operands, so the link-map rides the one shared integer encoding.
fn int_leaf(b: &mut Builder, n: u32) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(i64::from(n)),
        radix: Radix::Dec,
    })
}

/// One file's contribution to the merged arena — the span of structure ids it owns, so a global
/// `StructId` demuxes back to `(path, local id)` for source mapping (`DESIGN-package-linking.md` §6).
/// A diagnostic's global node id `n` falls in exactly one file's `[struct_base, struct_base +
/// struct_count)` range; `n - struct_base` is the per-file local id that file's own span table is
/// keyed by. The link-synthesized `(do …)` root sits OUTSIDE every file's range (it belongs to no
/// source file), so it never mis-demuxes to a file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileSpan {
    /// The file's artifact name (the `<path>` an `(import …)` names it by).
    pub path: String,
    /// The first merged `StructId` this file's structure entries occupy.
    pub struct_base: u32,
    /// How many structure entries this file contributed (its range length).
    pub struct_count: u32,
}

impl FileSpan {
    /// Whether the global structure id `id` falls in this file's range.
    pub fn contains(&self, id: StructId) -> bool {
        let n = id.0;
        n >= self.struct_base && n < self.struct_base + self.struct_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST link-map wire round-trips exactly (operator seq-254: the payload is canonical binary
    // AST, not a bespoke tab text). A tabbed/awkward path, a zero-count file, and the empty table all
    // survive encode→decode — the byte-stable contract the rcdzc producer + cdz consumer rely on.
    #[test]
    fn link_map_binary_ast_round_trips() {
        let table = vec![
            FileSpan {
                path: "app".into(),
                struct_base: 0,
                struct_count: 42,
            },
            FileSpan {
                path: "src/lib/util".into(),
                struct_base: 42,
                struct_count: 7,
            },
            FileSpan {
                path: "weird\tname".into(), // a tab in the path was the fragile case for the old text wire
                struct_base: 49,
                struct_count: 0,
            },
        ];
        assert_eq!(decode_link_map(&encode_link_map(&table)), table);
        // Empty table round-trips to empty (not an error).
        assert!(decode_link_map(&encode_link_map(&[])).is_empty());
        // A non-AST / garbage payload decodes to an empty table (total, graceful-degrade — never panics).
        assert!(decode_link_map(b"not a binary-ast tree").is_empty());
    }
}
