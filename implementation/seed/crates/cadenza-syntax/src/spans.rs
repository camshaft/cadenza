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
    use super::*;
    use crate::sexpr;

    #[test]
    fn remap_rekeys_drops_unreachable_and_preserves_file() {
        // A table over 3 OLD ids, remapped through an OLD→NEW map. `remap` must: move each mapped span
        // to its NEW index, DROP a `None` (unreachable old node), leave an untouched new slot at the
        // default (0,0), and preserve the file id. This unit-pins the defensive behaviors the ML-surface
        // span remap (`ml-parser-node-order`) relies on — separately from the canon integration test.
        let mut t = SpanTable::new(FileId(7));
        t.push(Span::new(0, 1)); // old 0
        t.push(Span::new(2, 3)); // old 1  (will be dropped — maps to None)
        t.push(Span::new(4, 5)); // old 2
        // old 0 → new 2, old 1 → None (unreachable), old 2 → new 0. new_len = 4 (one slot never filled).
        let id_map = [Some(StructId(2)), None, Some(StructId(0))];
        let r = t.remap(&id_map, 4);
        assert_eq!(r.len(), 4, "sized to new_len");
        assert_eq!(r.file(), FileId(7), "file id preserved");
        assert_eq!(r.get(StructId(0)), Some(Span::new(4, 5)), "old 2 → new 0");
        assert_eq!(r.get(StructId(2)), Some(Span::new(0, 1)), "old 0 → new 2");
        // old 1 was None → its span is dropped; new slots 1 and 3 stay at the default (0,0).
        assert_eq!(
            r.get(StructId(1)),
            Some(Span::new(0, 0)),
            "unfilled slot is default"
        );
        assert_eq!(
            r.get(StructId(3)),
            Some(Span::new(0, 0)),
            "trailing slot is default"
        );
        assert_eq!(r.get(StructId(4)), None, "past the end is None");
    }

    #[test]
    fn remap_drops_a_span_whose_new_id_is_out_of_range() {
        // Defensive: a map entry pointing past `new_len` must be DROPPED, not panic or grow the table.
        let mut t = SpanTable::new(FileId(0));
        t.push(Span::new(0, 1)); // old 0
        t.push(Span::new(1, 2)); // old 1
        // old 0 → new 0 (in range); old 1 → new 5 (out of range for new_len=2) → dropped.
        let r = t.remap(&[Some(StructId(0)), Some(StructId(5))], 2);
        assert_eq!(r.len(), 2);
        assert_eq!(r.get(StructId(0)), Some(Span::new(0, 1)));
        assert_eq!(
            r.get(StructId(1)),
            Some(Span::new(0, 0)),
            "out-of-range new id dropped"
        );
    }

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

    #[test]
    fn node_at_offset_prefers_the_smallest_containing_span() {
        // The tie-break is by span LENGTH: given nested spans containing the offset, the SMALLEST
        // (innermost) wins. Build a table by hand — id 0 the outer [0,10), id 1 an inner [2,6), id 2 a
        // still-smaller [3,4) — all containing offset 3; the deepest ([3,4)) must be chosen.
        let mut t = SpanTable::new(FileId(0));
        t.push(Span::new(0, 10)); // id 0 — outer
        t.push(Span::new(2, 6)); // id 1 — middle
        t.push(Span::new(3, 4)); // id 2 — innermost, contains 3
        assert_eq!(
            t.node_at_offset(3),
            Some(StructId(2)),
            "smallest containing span wins"
        );
        // Offset 2 is in id 1 [2,6) and id 0 [0,10) but NOT id 2 [3,4) → id 1 (the smaller of the two).
        assert_eq!(t.node_at_offset(2), Some(StructId(1)));
        // Offset 0 is only in id 0.
        assert_eq!(t.node_at_offset(0), Some(StructId(0)));
        // Offset at the outer end is half-open — contained by none.
        assert_eq!(t.node_at_offset(10), None);
    }

    #[test]
    fn node_at_offset_on_an_empty_table_is_none() {
        let t = SpanTable::new(FileId(0));
        assert_eq!(t.node_at_offset(0), None);
    }
}
