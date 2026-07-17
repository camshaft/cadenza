//! Byte spans

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Whether `byte_index` falls within this HALF-OPEN span `[start, end)` — the end byte is NOT
    /// contained (it is the first byte AFTER the span). An empty span (`start == end`) contains nothing.
    pub fn contains(&self, byte_index: usize) -> bool {
        byte_index >= self.start && byte_index < self.end
    }

    /// Whether `span` is a SUB-RANGE of this one: `[span.start, span.end) ⊆ [self.start, self.end)`.
    /// This is range CONTAINMENT (compared with endpoints), NOT two `contains` point-checks — the old
    /// implementation used `self.contains(span.start) && self.contains(span.end)`, which was wrong on
    /// two counts: `contains` is half-open, so `contains(span.end)` was false whenever `span` shared
    /// this span's end (a span did not even contain ITSELF — `(0..5).contains_span(0..5)` was false),
    /// and an empty inner span was mishandled. A span contains itself, contains any sub-range up to and
    /// including a shared endpoint, and (vacuously) contains an empty span that sits within `[start, end]`.
    pub fn contains_span(&self, span: Self) -> bool {
        self.start <= span.start && span.end <= self.end
    }

    pub fn merge(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

impl From<usize> for Span {
    fn from(value: usize) -> Self {
        Self {
            start: value,
            end: value,
        }
    }
}

impl From<Range<usize>> for Span {
    fn from(value: Range<usize>) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

impl From<Span> for Range<usize> {
    fn from(value: Span) -> Self {
        value.start..value.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_and_is_empty() {
        assert_eq!(Span::new(2, 5).len(), 3);
        assert_eq!(Span::new(4, 4).len(), 0);
        assert!(Span::new(4, 4).is_empty());
        assert!(!Span::new(4, 5).is_empty());
    }

    #[test]
    fn contains_is_half_open() {
        let s = Span::new(2, 5); // [2, 5)
        assert!(!s.contains(1)); // before
        assert!(s.contains(2)); // start IS contained
        assert!(s.contains(4)); // last interior byte
        assert!(!s.contains(5)); // end is NOT contained (half-open)
        assert!(!s.contains(6));
        // An empty span contains no byte.
        assert!(!Span::new(3, 3).contains(3));
    }

    #[test]
    fn contains_span_is_range_containment() {
        let s = Span::new(2, 8);
        // A span contains ITSELF (the old point-check impl got this wrong — contains(end) was false).
        assert!(s.contains_span(s));
        assert!(s.contains_span(Span::new(2, 8)));
        // Proper sub-ranges, including ones sharing an endpoint with the outer span.
        assert!(s.contains_span(Span::new(3, 7)));
        assert!(s.contains_span(Span::new(2, 5))); // shares start
        assert!(s.contains_span(Span::new(5, 8))); // shares end
        // An empty span sitting anywhere within [start, end] is (vacuously) contained.
        assert!(s.contains_span(Span::new(2, 2)));
        assert!(s.contains_span(Span::new(8, 8)));
        assert!(s.contains_span(Span::new(5, 5)));
        // Ranges that poke outside are NOT contained.
        assert!(!s.contains_span(Span::new(1, 8))); // start before
        assert!(!s.contains_span(Span::new(2, 9))); // end after
        assert!(!s.contains_span(Span::new(0, 20))); // wholly larger
        assert!(!s.contains_span(Span::new(9, 10))); // wholly after
    }

    #[test]
    fn merge_is_the_bounding_span() {
        assert_eq!(Span::new(2, 5).merge(Span::new(7, 9)), Span::new(2, 9));
        assert_eq!(Span::new(7, 9).merge(Span::new(2, 5)), Span::new(2, 9)); // order-independent
        assert_eq!(Span::new(2, 9).merge(Span::new(4, 6)), Span::new(2, 9)); // nested
        assert_eq!(Span::new(2, 5).merge(Span::new(2, 5)), Span::new(2, 5)); // idempotent
        // The merge of a span with any sub-span it contains is itself (used pervasively by the lexer to
        // extend a token span through a bumped char).
        let outer = Span::new(2, 9);
        assert!(outer.contains_span(outer.merge(Span::new(3, 4))));
    }

    #[test]
    fn conversions_round_trip() {
        // usize → point span.
        assert_eq!(Span::from(4usize), Span::new(4, 4));
        // Range ↔ Span.
        assert_eq!(Span::from(2..7), Span::new(2, 7));
        assert_eq!(Range::from(Span::new(2, 7)), 2..7);
        let s = Span::new(3, 11);
        assert_eq!(Span::from(Range::from(s)), s);
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible fuzz without a dependency, matching the
    /// lexer/codec/parser house style (the crate stays "plain").
    struct SplitMix64(u64);
    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
    }

    #[test]
    fn merge_is_a_bounding_semilattice_over_generated_spans() {
        // `Span::merge` is a min-start/max-end bounding join — a SEMILATTICE. `merge_is_the_bounding_span`
        // pins a few cases; this sweeps the algebraic laws that the lexer (extending a token span through
        // each bumped char) and every arena builder (a parent span = the merge of its children) rely on:
        //   * IDEMPOTENT:    a.merge(a) == a
        //   * COMMUTATIVE:   a.merge(b) == b.merge(a)
        //   * ASSOCIATIVE:   (a.merge(b)).merge(c) == a.merge(b.merge(c))
        //   * ABSORBING:     a.merge(b) contains_span BOTH a and b  (for well-formed start<=end spans)
        //   * the merge is the exact bounding box: start = min starts, end = max ends.
        // A regression in any (e.g. a max/min swap, or an off-by-one in contains_span) breaks span
        // extension silently — a token/node would report a wrong source range with no error.
        let mut seed = SplitMix64(0x5a4c_0de5_0bad_f00d);
        // A well-formed span (start <= end) drawn from a small coordinate space so overlaps/nesting occur.
        let span = |s: &mut SplitMix64| {
            let a = (s.next() % 40) as usize;
            let b = (s.next() % 40) as usize;
            Span::new(a.min(b), a.max(b))
        };
        for _ in 0..20_000 {
            let a = span(&mut seed);
            let b = span(&mut seed);
            let c = span(&mut seed);
            // Idempotent.
            assert_eq!(a.merge(a), a, "merge not idempotent for {a:?}");
            // Commutative.
            assert_eq!(
                a.merge(b),
                b.merge(a),
                "merge not commutative for {a:?}, {b:?}"
            );
            // Associative.
            assert_eq!(
                a.merge(b).merge(c),
                a.merge(b.merge(c)),
                "merge not associative for {a:?}, {b:?}, {c:?}"
            );
            // The result is the exact bounding box…
            let m = a.merge(b);
            assert_eq!(
                m,
                Span::new(a.start.min(b.start), a.end.max(b.end)),
                "merge is not the bounding box for {a:?}, {b:?}"
            );
            // …and absorbs both operands (each is contained in the merge).
            assert!(
                m.contains_span(a) && m.contains_span(b),
                "merge {m:?} does not contain both {a:?} and {b:?}"
            );
        }
    }

    #[test]
    fn contains_span_agrees_with_pointwise_contains_over_generated_spans() {
        // `contains_span` and `contains` are the two containment predicates the span table + LSP
        // node-at-offset lookups rest on. The doc on `contains_span` records a REAL past bug: it was
        // implemented as `self.contains(span.start) && self.contains(span.end)` — wrong because `contains`
        // is half-open (so `contains(end)` failed on a shared end, and a span didn't even contain itself).
        // The hand tests pin specific cases; this sweeps the two predicates' RELATIONSHIP so a regression
        // back to a point-check impl (or any endpoint off-by-one) is caught over the whole coordinate space:
        //   * point/range consistency: `a.contains(i)` ⟺ `a.contains_span(i..i+1)` — a single byte is the
        //     unit-length sub-range, the exact bridge between the two predicates;
        //   * range containment = pointwise containment for a NON-EMPTY inner span: `a.contains_span(b)`
        //     (b non-empty) ⟺ every byte in `[b.start, b.end)` satisfies `a.contains(_)`;
        //   * an EMPTY inner span is contained iff its point sits in the CLOSED `[start, end]` (the
        //     half-open `contains` says nothing here — this is the documented vacuous-containment case).
        // Small coordinate space so nesting / shared endpoints / just-outside cases actually occur.
        let mut seed = SplitMix64(0xc047_a1a5_4c0d_e501);
        let span = |s: &mut SplitMix64| {
            let a = (s.next() % 24) as usize;
            let b = (s.next() % 24) as usize;
            Span::new(a.min(b), a.max(b))
        };
        for _ in 0..20_000 {
            let a = span(&mut seed);
            let b = span(&mut seed);
            // (1) point ⇔ unit-range: contains(i) iff contains_span(i..i+1), over a byte range covering
            // a's neighbourhood (before start, interior, at/after end).
            for i in 0..26usize {
                assert_eq!(
                    a.contains(i),
                    a.contains_span(Span::new(i, i + 1)),
                    "contains({i}) disagrees with contains_span({i}..{}) for {a:?}",
                    i + 1
                );
            }
            // (2) for a NON-EMPTY inner span, range containment ⇔ every interior byte is contained.
            if !b.is_empty() {
                let pointwise = (b.start..b.end).all(|i| a.contains(i));
                assert_eq!(
                    a.contains_span(b),
                    pointwise,
                    "contains_span({b:?}) disagrees with pointwise contains over its bytes for {a:?}"
                );
            } else {
                // (3) an EMPTY inner span is contained iff its point is in the CLOSED interval [start,end]
                // (the documented vacuous case — half-open `contains` would wrongly reject the endpoint).
                assert_eq!(
                    a.contains_span(b),
                    a.start <= b.start && b.start <= a.end,
                    "empty-span containment wrong for point {} in {a:?}",
                    b.start
                );
            }
        }
    }
}
