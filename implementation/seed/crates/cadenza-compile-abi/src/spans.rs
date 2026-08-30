//! The span sidecar — the source-position side-table, crossing as its OWN kinded input artifact.
//!
//! The compiler is deliberately SPAN-FREE: the binary AST (`codec`) carries no source positions, so
//! formatting cannot change its bytes and the `Db` columns stay position-free. But DEBUG INFORMATION
//! (`DESIGN-debug-info-rcdzc.md`) needs, per emitted instruction, the SOURCE RANGE the instruction
//! derives from — to build the DWARF line program. The operator's ruling (design §2.1a) resolves this
//! WITHOUT re-introducing spans into the AST: the span table crosses as a SIBLING artifact
//! (`kind == "spans"`), keyed by the same `StructId` the arena uses, so it aligns 1:1 with the decoded
//! structure arena. The front-end (which already builds a `SpanTable`) emits it; the driver passes it
//! alongside the `ast` artifact when a debug `Emit` request is in the sidecar; the BACKEND reads it to
//! emit debug sections. When absent (the common no-debug build), nothing reads it and the artifact is
//! exactly today's bytes.
//!
//! **The wire form** is canonical BINARY AST (`cadenza_ast::codec`) — the SAME wire every compile-boundary
//! artifact speaks (operator P0, seq-284: "Binary AST everywhere" — no bespoke hand-rolled leb128 framing),
//! TOTAL decode (never panics on untrusted bytes, so it ports to the Cadenza self-host):
//!
//! ```text
//!   (list [ Str module_path
//!           (list [ (list [Int start] [Int len]) … ])   -- one 2-element form per node, span-count implicit
//!           Str source ])
//! ```
//!
//! A span is stored as `(start, LENGTH)` rather than `(start, end)` because a length is always `>= 0` and
//! typically small (and the AST-level `Int` leaf carries it directly) — smaller and impossible to encode
//! backwards. The path is the TREE-RELATIVE module path (`source-tree-encoding.md` §MUST include each
//! module's tree-relative path), never an absolute filesystem path — the reproducibility contract
//! (design §4) records this path in the DWARF file entry, so an absolute path would leak the build
//! directory. A malformed span artifact is a DECLINE (a diagnostic), never a panic or a silent drop.
//!
//! A span is keyed by `StructId` — an index into the canonical structure arena — so the source
//! location a debugger recovers is a range over the CANONICAL representation, stable under any textual
//! rendering rather than tied to one textual syntax:
//!
//= spec/capabilities/debug-information.md#a-source-location-is-a-span-over-the-canonical-representation
//# A source location recorded in debug information MUST be a source span over the canonical representation, so that the location is stable under any textual rendering rather than tied to one textual syntax.

use cadenza_ast::ast::{Builder, IntValue, Leaf, Radix, Struct, StructId};

/// The kinded input artifact carrying the span side-table.
pub const KIND_SPANS: &str = "spans";

/// The decoded span side-table: the tree-relative module path plus one `(start, len)` byte range per
/// AST occurrence, positionally indexed by `StructId` (so `spans[id.0]` is that occurrence's range).
/// This is what the backend reads to map a code offset back to a source position. The `spans` vector is
/// as long as the structure arena when produced by a conformant front-end; a shorter one simply yields
/// `None` from [`SpanData::range`] for the missing tail (total — never an out-of-bounds panic).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SpanData {
    /// The tree-relative module path — the DWARF file-table name. Never absolute (design §4).
    //= spec/capabilities/debug-information.md#a-file-reference-is-a-tree-relative-module-path
    //# A file reference recorded in debug information MUST be the tree-relative module path fixed by the source-tree-encoding contract rather than an absolute filesystem path, so that debug information names a source module the same way the canonical source tree does and carries no build-host path.
    pub module_path: String,
    /// `(start, len)` byte ranges, indexed positionally by `StructId`.
    pub spans: Vec<(u32, u32)>,
    /// The module's source TEXT — carried so line/col can be derived from a byte offset at emit time (a
    /// DWARF `.debug_line` row needs `(line, col)`, but the front-end records byte-offset spans). The
    /// design's §2.1a sub-decision, resolved in favour of the artifact carrying the text (self-contained
    /// — the backend needs no filesystem access, keeping the compile a pure function of its inputs).
    /// Empty when the producer omits it (then line derivation falls back to line 1).
    pub source: String,
}

impl SpanData {
    /// The `(start, end)` byte range of an occurrence, if recorded. Total: an id past the table (a
    /// prelude/synthesized node, or a truncated table) yields `None`, so the caller emits no line-table
    /// row for it rather than mapping to a garbage position (the same discipline `sanitize_origin`
    /// applies at the diagnostic edge — only a real source node maps back).
    pub fn range(&self, id: StructId) -> Option<(u32, u32)> {
        let &(start, len) = self.spans.get(id.0 as usize)?;
        Some((start, start.saturating_add(len)))
    }

    /// The 1-based source LINE a byte offset falls on — a one-pass newline count over `source` up to
    /// `byte_off` (design §2.1a: "a one-pass newline index over the module's source text"). Falls back
    /// to line 1 when the source text is absent or the offset is past its end (total — never panics).
    /// Line/col derivation lives here so both a DWARF line row and a diagnostic can share it — the
    /// projection of a canonical byte-span to the textual `(line, col)` a debug view presents.
    //= spec/capabilities/debug-information.md#a-source-location-is-a-span-over-the-canonical-representation
    //# A source span recorded in debug information MUST be renderable to a textual source location by the printer, so that a textual debug view is a projection of the canonical form through the same printer any textual syntax uses rather than a second authority over where a construct is.
    pub fn line_at(&self, byte_off: u32) -> u32 {
        if self.source.is_empty() {
            return 1;
        }
        let end = (byte_off as usize).min(self.source.len());
        1 + self.source.as_bytes()[..end]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as u32
    }

    /// The 1-based source COLUMN a byte offset falls on — the count of bytes since the last newline,
    /// plus one. Symmetric to [`line_at`] (both derive from the same byte offset over `source`), so a
    /// DWARF row can carry a `(line, column)` position and a debugger can highlight the exact
    /// sub-expression on a line — the payoff for s-expression Cadenza, where `(if c a b)` packs several
    /// constructs onto one line. Falls back to column 1 when the source is absent or the offset is past
    /// its end (total — never panics). Byte columns (not UTF-8 scalar columns) — the DWARF convention,
    /// and what a byte-offset debugger maps back with; ASCII source (the common case) makes them equal.
    pub fn col_at(&self, byte_off: u32) -> u32 {
        if self.source.is_empty() {
            return 1;
        }
        let end = (byte_off as usize).min(self.source.len());
        let bytes = &self.source.as_bytes()[..end];
        match bytes.iter().rposition(|&b| b == b'\n') {
            // Bytes after the last newline (the newline itself excluded), plus one for 1-based.
            Some(nl) => (end - nl) as u32,
            // No newline before `byte_off` — it is on the first line, column = offset + 1.
            None => end as u32 + 1,
        }
    }

    /// A reusable LINE-START INDEX over `source` — the byte offset of each line's first char, ascending
    /// (`[0]` = 0). Built ONCE in O(len); then [`line_at`]/[`col_at`] become a binary search + a bounded
    /// per-line count via [`LineStarts::line_col`], instead of the O(byte_off) scan-from-start `line_at`
    /// does. A CALLER that maps MANY offsets over one source — the `cdz compile` diagnostic report (one
    /// per fault) and the DWARF line-table (one per emitted function) — builds this ONCE and reuses it,
    /// turning what was O(sites × source_len) = O(N²) into O(len + sites·log). A one-shot caller can keep
    /// using `line_at`/`col_at`. The `(line, col)` are byte-identical to `line_at`/`col_at` (byte columns).
    ///
    /// [`line_at`]: SpanData::line_at
    /// [`col_at`]: SpanData::col_at
    pub fn line_starts(&self) -> LineStarts {
        let mut starts = vec![0u32];
        for (i, &b) in self.source.as_bytes().iter().enumerate() {
            if b == b'\n' {
                starts.push(i as u32 + 1);
            }
        }
        LineStarts {
            starts,
            len: self.source.len() as u32,
        }
    }
}

/// A prebuilt line-start index for one `SpanData`'s source — the sorted byte offsets of each line start
/// (see [`SpanData::line_starts`]). Binary-searches a byte offset to a 1-based `(line, col)` in
/// O(log lines), byte-identical to [`SpanData::line_at`]/[`SpanData::col_at`] (byte columns).
pub struct LineStarts {
    starts: Vec<u32>,
    len: u32,
}

impl LineStarts {
    /// The 1-based `(line, col)` of `byte_off` — the same pair `(line_at(byte_off), col_at(byte_off))`
    /// returns, via binary search. A byte past the end clamps (like `line_at`/`col_at`).
    pub fn line_col(&self, byte_off: u32) -> (u32, u32) {
        if self.starts.len() == 1 && self.len == 0 {
            return (1, 1); // empty source — matches line_at/col_at's line-1/col-1 fallback
        }
        let off = byte_off.min(self.len);
        // The line is the last start `<= off`. `partition_point` counts starts `<= off`; `starts[0] == 0
        // <= off` always, so the count (the 1-based line) is `>= 1`.
        let line = self.starts.partition_point(|&s| s <= off) as u32;
        let line_start = self.starts[line as usize - 1];
        // Column = bytes since the line start + 1 (byte columns, exactly `col_at`).
        (line, off - line_start + 1)
    }
}

/// Decode a span side-table from its wire bytes. Total: a truncated or malformed table yields `None`
/// (the caller turns that into a decline diagnostic), never a panic. Mirrors `codec::decode` /
/// `sidecar::decode` discipline exactly so the format ports to the self-host.
pub fn decode(bytes: &[u8]) -> Option<SpanData> {
    // Canonical binary AST (see module docs): a 3-column root `Ast.List` of `[Str path, spans-list, Str
    // source]`, where spans-list is a `List` of `(list [Int start][Int len])` forms. Read via the SAME
    // shared `cadenza_ast::codec` every compile-boundary artifact speaks. TOTAL: a non-AST payload, a
    // wrong-shape tree, or an out-of-range operand yields `None` (a decline), never a panic.
    let a = cadenza_ast::codec::decode(bytes)?;
    let Struct::List(cols) = a.get(a.root).clone() else {
        return None;
    };
    if cols.len() != 3 {
        return None;
    }
    let module_path = a.as_str(cols[0])?.to_string();
    let Struct::List(span_forms) = a.get(cols[1]).clone() else {
        return None;
    };
    let mut spans = Vec::with_capacity(span_forms.len());
    for f in span_forms {
        let Struct::List(pair) = a.get(f) else {
            return None;
        };
        if pair.len() != 2 {
            return None;
        }
        let start = u32::try_from(a.as_int(pair[0])?.to_i64()?).ok()?;
        let len = u32::try_from(a.as_int(pair[1])?.to_i64()?).ok()?;
        spans.push((start, len));
    }
    let source = a.as_str(cols[2])?.to_string();
    Some(SpanData {
        module_path,
        spans,
        source,
    })
}

/// Encode a span side-table to its wire bytes — the counterpart to [`decode`], used by a driver (and
/// the tests) to build a `spans` input. Canonical binary AST (see module docs). `decode(encode(s)) == s`.
pub fn encode(data: &SpanData) -> Vec<u8> {
    let mut b = Builder::new();
    let path = b.atom_leaf(Leaf::Str(data.module_path.as_str().into()));
    let span_forms: Vec<StructId> = data
        .spans
        .iter()
        .map(|&(start, len)| {
            let s = int_leaf(&mut b, start);
            let l = int_leaf(&mut b, len);
            b.list(vec![s, l])
        })
        .collect();
    let spans_list = b.list(span_forms);
    let source = b.atom_leaf(Leaf::Str(data.source.as_str().into()));
    let root = b.list(vec![path, spans_list, source]);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// An `Ast.Int` (decimal) leaf for a span's `start`/`len` — the same integer-atom encoding the sibling
/// compile-boundary wires (`link_map`, `sidecar`) use for their node-id operands.
fn int_leaf(b: &mut Builder, n: u32) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(i64::from(n)),
        radix: Radix::Dec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let data = SpanData {
            module_path: "src/main.cdz".to_string(),
            spans: vec![(0, 5), (6, 1), (8, 42), (100, 0)],
            source: "(module m\n  (def (main) 42))".to_string(),
        };
        assert_eq!(decode(&encode(&data)), Some(data));
    }

    #[test]
    fn empty_round_trips() {
        let data = SpanData {
            module_path: String::new(),
            spans: vec![],
            source: String::new(),
        };
        assert_eq!(decode(&encode(&data)), Some(data));
    }

    #[test]
    fn range_maps_start_len_to_start_end() {
        let data = SpanData {
            module_path: "m".to_string(),
            spans: vec![(10, 3), (20, 0)],
            ..Default::default()
        };
        assert_eq!(data.range(StructId(0)), Some((10, 13)));
        assert_eq!(data.range(StructId(1)), Some((20, 20)));
        // An id past the table is None (a prelude/synthesized node), not a panic.
        assert_eq!(data.range(StructId(2)), None);
    }

    #[test]
    fn line_at_counts_newlines() {
        let data = SpanData {
            source: "aaa\nbbb\nccc".to_string(),
            ..Default::default()
        };
        assert_eq!(data.line_at(0), 1); // in "aaa"
        assert_eq!(data.line_at(3), 1); // the '\n' itself is still line 1
        assert_eq!(data.line_at(4), 2); // first byte of "bbb"
        assert_eq!(data.line_at(8), 3); // first byte of "ccc"
        assert_eq!(data.line_at(999), 3); // past the end clamps, no panic
        // With no source text, everything is line 1 (the fallback).
        let empty = SpanData::default();
        assert_eq!(empty.line_at(42), 1);
    }

    #[test]
    fn col_at_counts_bytes_since_the_last_newline() {
        let data = SpanData {
            source: "aaa\nbbbb\nc".to_string(),
            ..Default::default()
        };
        assert_eq!(data.col_at(0), 1); // first byte of "aaa" (no newline yet)
        assert_eq!(data.col_at(2), 3); // third byte of "aaa"
        assert_eq!(data.col_at(3), 4); // the '\n' is column 4 of line 1
        assert_eq!(data.col_at(4), 1); // first byte of "bbbb" — column resets after the newline
        assert_eq!(data.col_at(7), 4); // fourth byte of "bbbb"
        assert_eq!(data.col_at(9), 1); // first byte of "c" on the last line
        assert_eq!(data.col_at(999), 2); // past the end clamps to len (one past "c"), no panic
        // With no source text, everything is column 1 (the fallback).
        let empty = SpanData::default();
        assert_eq!(empty.col_at(42), 1);
    }

    #[test]
    fn line_starts_index_matches_line_at_col_at_at_every_offset() {
        // `LineStarts::line_col` (binary search, the batch path the `cdz compile` diagnostic report and
        // the DWARF line-table use) must return EXACTLY `(line_at(off), col_at(off))` at every offset —
        // the byte-identity the O(N²)→O(N) fix relies on. Cover an empty line, a trailing newline, a byte
        // past the end, and the empty source (the line-1/col-1 fallback).
        for src in ["aaa\nbbb\nccc", "", "\n\n\n", "no newline", "x\n"] {
            let data = SpanData {
                source: src.to_string(),
                ..Default::default()
            };
            let idx = data.line_starts();
            for off in 0..=src.len() as u32 + 3 {
                assert_eq!(
                    idx.line_col(off),
                    (data.line_at(off), data.col_at(off)),
                    "LineStarts disagrees with line_at/col_at at offset {off} of {src:?}"
                );
            }
        }
    }

    #[test]
    fn malformed_bytes_are_none_not_panic() {
        // Now that the wire is canonical binary AST, TOTAL decode means: non-AST / garbage bytes, an empty
        // input, and a well-formed AST of the WRONG SHAPE all yield None (a decline) rather than panicking —
        // the same graceful-degrade the sibling `link_map` / `result_types` codecs give.
        assert_eq!(decode(b"not a binary-ast tree"), None);
        assert_eq!(decode(&[0xff, 0xff, 0xff, 0xff]), None);
        assert!(decode(&[]).is_none());
        // A bare `Str` root (not the 3-column list) is the wrong shape → None.
        let mut b = Builder::new();
        let root = b.atom_leaf(Leaf::Str("nope".into()));
        assert_eq!(decode(&cadenza_ast::codec::encode(&b.finish(root))), None);
        // A 3-col list whose middle column is not a list-of-pairs → None (a span form of the wrong arity).
        let mut b = Builder::new();
        let path = b.atom_leaf(Leaf::Str("m".into()));
        let one = int_leaf(&mut b, 1);
        let bad_form = b.list(vec![one]); // a 1-element span form (want 2)
        let spans = b.list(vec![bad_form]);
        let src = b.atom_leaf(Leaf::Str(String::new().as_str().into()));
        let root = b.list(vec![path, spans, src]);
        assert_eq!(decode(&cadenza_ast::codec::encode(&b.finish(root))), None);
    }
}
