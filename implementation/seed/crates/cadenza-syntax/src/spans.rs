//! The span side-table: `StructId -> source span`.
//!
//! Spans live BESIDE the arena, never in it — the binary AST is span-free so formatting cannot
//! change its bytes. The table is total by construction: the parser records one span per structure
//! occurrence as it creates it, and occurrences are never deduplicated, so every `StructId` maps to
//! exactly the source range it came from (the two `x`s in `x + x` have distinct ids and distinct
//! spans, even though both resolve to one interned leaf). This is the source-tracking substrate
//! diagnostics and every later analysis (types, …) key off.

use crate::ast::StructId;
use crate::span::Span;

/// A source file identifier. `0` is the anonymous/single-file default; a multi-file driver assigns
/// distinct ids. Kept tiny and out of the arena — purely diagnostic metadata.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct FileId(pub u32);

/// Maps each structure occurrence to where it came from in source. Indexed by `StructId` position,
/// so lookup is O(1) and the table is exactly as long as the structure arena.
#[derive(Clone, Debug, Default)]
pub struct SpanTable {
    file: FileId,
    spans: Vec<Span>,
}

impl SpanTable {
    pub fn new(file: FileId) -> SpanTable {
        SpanTable {
            file,
            spans: Vec::new(),
        }
    }

    /// Record the span for the next occurrence. The parser calls this once per created `StructId`,
    /// in id order, so `spans[id]` is that occurrence's span.
    pub fn push(&mut self, span: Span) {
        self.spans.push(span);
    }

    /// The source span of an occurrence, if recorded.
    pub fn get(&self, id: StructId) -> Option<Span> {
        self.spans.get(id.0 as usize).copied()
    }

    /// Re-key this table through an OLD→NEW structure-id map (from `canon::canonicalize_with_map`), so
    /// the table is indexed by the CANONICAL ids — the ids `codec::encode` produces, hence the ids the
    /// COMPILER reports back. Without this, a table built by a non-canonical reader (the ML surface) is
    /// keyed by pre-canonical ids and a lookup by a compiler node id lands on the wrong node
    /// (`ml-parser-node-order`). `new_len` sizes the result (the canonical arena's node count); a `None`
    /// map entry (an unreachable old node) is dropped. A span whose new id is out of range is dropped
    /// (defensive). The file id is preserved.
    pub fn remap(&self, id_map: &[Option<StructId>], new_len: usize) -> SpanTable {
        let default = Span { start: 0, end: 0 };
        let mut spans = vec![default; new_len];
        for (old, &new) in id_map.iter().enumerate() {
            if let Some(new) = new
                && let Some(&s) = self.spans.get(old)
                && (new.0 as usize) < new_len
            {
                spans[new.0 as usize] = s;
            }
        }
        SpanTable {
            file: self.file,
            spans,
        }
    }

    /// The file these spans are in.
    pub fn file(&self) -> FileId {
        self.file
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// The INNERMOST occurrence whose span contains `byte_offset` — the deepest node under a cursor.
    /// "Innermost" = the smallest-length containing span (a child's span is nested inside its parent's,
    /// so the smallest one is the leaf/most-specific node). Returns `None` when no recorded span
    /// contains the offset. This is the offset→node resolution a "type at cursor" (hover) needs: the
    /// caller maps a source position to a node id, then asks the compiler for that node's type — so the
    /// COMPILER stays span-free (the span table lives here, in the front-end) while the type query is by
    /// node identity. Shared so the `cdz type-at` CLI and the browser IDE resolve a cursor the same way.
    pub fn node_at_offset(&self, byte_offset: usize) -> Option<StructId> {
        let mut best: Option<(StructId, Span)> = None;
        for (i, &s) in self.spans.iter().enumerate() {
            if s.contains(byte_offset) && best.is_none_or(|(_, b)| s.len() < b.len()) {
                best = Some((StructId(i as u32), s));
            }
        }
        best.map(|(id, _)| id)
    }
}

#[cfg(test)]
mod tests {
    use crate::sexpr;

    #[test]
    fn node_at_offset_finds_the_innermost_node() {
        // `(+ a b)` — hovering the `a` returns the `a` leaf, NOT the enclosing `(+ a b)` list, because
        // the innermost (smallest) containing span wins.
        let src = "(+ a b)";
        let (arenas, spans) = sexpr::read_all_spanned(src).expect("parse");
        let a_off = src.find('a').unwrap();
        let node = spans.node_at_offset(a_off).expect("a node at `a`");
        assert_eq!(
            arenas.as_name(node),
            Some("a"),
            "innermost node under `a` is the `a` leaf"
        );

        // An offset on the `+` head returns the `+` leaf, not the list.
        let plus_off = src.find('+').unwrap();
        let head = spans.node_at_offset(plus_off).expect("a node at `+`");
        assert_eq!(arenas.as_name(head), Some("+"));
    }

    #[test]
    fn node_at_offset_past_the_source_is_none() {
        let (_, spans) = sexpr::read_all_spanned("(+ 1 2)").expect("parse");
        assert_eq!(spans.node_at_offset(9999), None);
    }
}
