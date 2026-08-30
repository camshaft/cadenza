//! The link-map RESULT wire — the diagnostics DEMUX table a linked package emits (`KIND_LINK_MAP`
//! artifact) + its `FileSpan` table and `encode`/`decode` codec. A compile-BOUNDARY concern: a
//! consumer (`cdz check`, the error reporter) reads it to demux a GLOBAL merged `StructId` back to
//! `(file, local id)` for source mapping, so it must speak this format WITHOUT linking `rcdzc`. The
//! LINKER itself (`link()`, `Linkage`, `LinkedProgram`) stays in `rcdzc::link`, which `pub use`s these
//! so `crate::link::{KIND_LINK_MAP, FileSpan, encode_link_map, decode_link_map}` stay byte-stable.

use cadenza_ast::ast::StructId;

/// The OUTPUT-artifact kind carrying the diagnostics DEMUX table for a linked package
/// (`DESIGN-package-linking.md` §6). A cross-file diagnostic's `node` is a GLOBAL merged `StructId`;
/// with several files spliced into one arena, that global id no longer maps to a single file's span
/// table. This artifact lets a consumer demux: a global node `n` falls in exactly one file's
/// `[struct_base, struct_base+struct_count)` range → `(path, n - struct_base)` = the per-file LOCAL id
/// that file's own span table (or its `spans` input) is keyed by. One line per file:
/// `<path>\t<struct_base>\t<struct_count>`, mirroring the `uses` artifact's node-id-per-line style —
/// a consumer-side two-level back-reference (global id → (file, local id) → span), so the compiler
/// stays span-free. Present only for a multi-file package.
pub const KIND_LINK_MAP: &str = "link-map";

/// Encode a package's `FileSpan` table as the `link-map` artifact bytes — one line per file,
/// `<path>\t<struct_base>\t<struct_count>`, in splice order. The inverse a consumer applies:
/// find the file whose `[base, base+count)` contains a diagnostic's global node id, then subtract
/// `base` for the per-file local id.
pub fn encode_link_map(files: &[FileSpan]) -> Vec<u8> {
    let mut out = String::new();
    for f in files {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            f.path, f.struct_base, f.struct_count
        ));
    }
    out.into_bytes()
}

/// Decode the `link-map` artifact bytes back into its `FileSpan` list — the inverse of
/// [`encode_link_map`], so a CLI reporter can demux a linked-package diagnostic's GLOBAL node id to the
/// `(file, local id)` its per-file span table is keyed by. A malformed line (wrong column count, a
/// non-numeric base/count) is skipped rather than failing the whole decode — the reporter degrades to a
/// location-less diagnostic, never a crash. `path` may itself contain no tab (an artifact name), so the
/// split takes exactly the first two tabs.
pub fn decode_link_map(bytes: &[u8]) -> Vec<FileSpan> {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .filter_map(|line| {
            let mut cols = line.rsplitn(3, '\t');
            // Right-split: count, base, then the remaining prefix is the path (which never contains a
            // tab in practice, but rsplitn keeps a tabbed path intact as the final piece).
            let count = cols.next()?.parse::<u32>().ok()?;
            let base = cols.next()?.parse::<u32>().ok()?;
            let path = cols.next()?.to_string();
            Some(FileSpan {
                path,
                struct_base: base,
                struct_count: count,
            })
        })
        .collect()
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
