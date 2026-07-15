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
}
