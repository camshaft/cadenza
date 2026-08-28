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

    // NOTE: the two `node_at_offset` tests that drove input through `sexpr::read_all_spanned` moved to
    // `cadenza-syntax/tests/spans_surface.rs` — this crate (`cadenza-syntax-core`) is BELOW every surface
    // reader, so a span test that needs a reader lives in the facade where both are available. The
    // hand-built table tests below stay here (no reader dependency).

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

    #[test]
    fn node_at_offset_breaks_an_equal_length_tie_by_lowest_id() {
        // Two spans of IDENTICAL length both containing the offset — e.g. a `(comment "x" form)` wrapper
        // and its sole child often carry the same source extent. The `s.len() < b.len()` test is STRICT,
        // so a later equal-length span does NOT displace an earlier one: the LOWEST structure id wins.
        // Pin this so a hover on such a coincident span is deterministic (and matches the wrapper-first
        // id order the reader produces).
        let mut t = SpanTable::new(FileId(0));
        t.push(Span::new(0, 5)); // id 0
        t.push(Span::new(0, 5)); // id 1 — same extent as id 0
        t.push(Span::new(0, 5)); // id 2 — same again
        assert_eq!(
            t.node_at_offset(2),
            Some(StructId(0)),
            "an equal-length tie resolves to the lowest structure id (first encountered)"
        );
        // A strictly-smaller span still wins over the equal-length run.
        t.push(Span::new(1, 3)); // id 3 — smaller, contains 2
        assert_eq!(t.node_at_offset(2), Some(StructId(3)));
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible generation without a dependency (mirrors
    /// the unit-test PRNGs in `codec.rs`/`lexer.rs`).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    #[test]
    fn node_at_offset_matches_a_brute_force_reference_over_generated_tables() {
        // `node_at_offset`'s contract — the INNERMOST (smallest-length) recorded span containing the
        // offset, ties broken by LOWEST id — swept over random span tables at every offset, checked
        // against an independent brute-force reference. The hand-picked tests pin specific shapes
        // (nesting, past-end, tie); this pins the FULL contract (an off-by-one in `contains`, a `<=`-vs-`<`
        // tie regression, or a wrong extremum) over arbitrary overlapping/empty/coincident spans. Spans
        // are drawn over a small coordinate range so overlaps + exact-length ties happen often.
        let mut rng = Rng(0x5da7_c0de_5da7_c0de);
        for _ in 0..4000 {
            let n = 1 + rng.below(12); // 1..=12 spans
            let mut t = SpanTable::new(FileId(0));
            let mut spans: Vec<Span> = Vec::with_capacity(n);
            for _ in 0..n {
                // Random [start, end) over 0..=10, possibly empty (start == end).
                let a = rng.below(11);
                let b = rng.below(11);
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                let s = Span::new(lo, hi);
                t.push(s);
                spans.push(s);
            }
            // Probe every offset in the coordinate range plus one past the end.
            for off in 0..=11usize {
                // Brute-force reference: the min-length containing span, ties → lowest id.
                let mut want: Option<(usize, usize)> = None; // (id, len)
                for (i, s) in spans.iter().enumerate() {
                    if s.contains(off) {
                        let len = s.len();
                        if want.is_none_or(|(_, wlen)| len < wlen) {
                            want = Some((i, len));
                        }
                    }
                }
                let want_id = want.map(|(i, _)| StructId(i as u32));
                assert_eq!(
                    t.node_at_offset(off),
                    want_id,
                    "node_at_offset({off}) disagreed with brute force for spans {spans:?}"
                );
            }
        }
    }

    // NOTE: `remap_then_resolve_round_trips_a_cursor_through_canonicalization` moved to
    // `cadenza-syntax/tests/spans_surface.rs` — it pairs the span remap with the ML reader +
    // `canon::canonicalize_with_map`, neither of which this below-the-surface crate may depend on.

    #[test]
    fn remap_invariants_hold_over_generated_tables_and_id_maps() {
        // `SpanTable::remap` is the defensive re-key the ML-surface span table relies on
        // (`ml-parser-node-order`): given an OLD→NEW id map + a new length, it moves each mapped span to
        // its new slot, drops unmapped/out-of-range ones, sizes to new_len, and preserves the file id.
        // The hand tests pin specific cases; this sweeps random tables × random id_maps (with `None`s,
        // out-of-range targets, and new_len smaller/larger than the table) and asserts the full contract
        // against an INDEPENDENT reference — so a re-key/drop/sizing bug on some shape is caught.
        let mut rr = Rng(0x51a7_ab1e_c0de_5eed);
        for _ in 0..4000 {
            let file = FileId(rr.next() as u32);
            let n_old = rr.below(8); // 0..=7 old nodes
            let mut t = SpanTable::new(file);
            for _ in 0..n_old {
                let a = rr.below(30);
                let b = rr.below(30);
                t.push(Span::new(a.min(b), a.max(b)));
            }
            // new_len straddles n_old (sometimes smaller → drops, sometimes larger → default slots).
            let new_len = rr.below(10);
            // A random OLD→NEW map: each old id → None, or Some(new id) that may be in- or out-of-range.
            let id_map: Vec<Option<StructId>> = (0..n_old)
                .map(|_| {
                    if rr.next().is_multiple_of(4) {
                        None
                    } else {
                        Some(StructId((rr.below(12)) as u32)) // may exceed new_len (out-of-range → dropped)
                    }
                })
                .collect();
            let r = t.remap(&id_map, new_len);
            // (a) sized to new_len, file preserved.
            assert_eq!(r.len(), new_len, "remap sizes to new_len");
            assert_eq!(r.file(), file, "remap preserves the file id");
            // (b) INDEPENDENT reference: build the expected table directly.
            let mut expected = vec![Span::new(0, 0); new_len];
            for (old, &new) in id_map.iter().enumerate() {
                if let Some(new) = new
                    && (new.0 as usize) < new_len
                {
                    // The LAST old id mapping to a given new slot wins (matches remap's forward loop).
                    expected[new.0 as usize] = t.get(StructId(old as u32)).unwrap();
                }
            }
            for (i, &want) in expected.iter().enumerate() {
                assert_eq!(
                    r.get(StructId(i as u32)),
                    Some(want),
                    "remap slot {i} mismatch (n_old={n_old}, new_len={new_len}, id_map={id_map:?})"
                );
            }
            // (c) past the end is None.
            assert_eq!(r.get(StructId(new_len as u32)), None, "past-end is None");
        }
    }
}
