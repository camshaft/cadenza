//! A span-carrying char iterator.

use crate::span::Span;
use std::str::CharIndices;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Char {
    pub span: Span,
    pub value: char,
}

impl From<Char> for Span {
    fn from(val: Char) -> Self {
        val.span
    }
}

impl Char {
    /// The span from `self`'s start through `end`'s end. Was `impl From<(Char, Char)> for Span`, but
    /// `Span` now lives in `cadenza-syntax-core` (a foreign type), and the orphan rule forbids
    /// implementing the foreign `From` for it over a `(Char, Char)` tuple (a tuple is not fundamental,
    /// so the local `Char` inside it doesn't confer locality). An inherent method is equivalent.
    pub fn span_to(self, end: Char) -> Span {
        self.span.merge(end.span)
    }
}

impl PartialEq<char> for Char {
    fn eq(&self, other: &char) -> bool {
        self.value == *other
    }
}

impl PartialEq<char> for &Char {
    fn eq(&self, other: &char) -> bool {
        self.value == *other
    }
}

impl PartialEq<Char> for char {
    fn eq(&self, other: &Char) -> bool {
        *self == other.value
    }
}

impl PartialEq<&Char> for char {
    fn eq(&self, other: &&Char) -> bool {
        *self == other.value
    }
}

pub struct Chars<'a> {
    inner: CharIndices<'a>,
}

impl<'a> Chars<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            inner: input.char_indices(),
        }
    }
}

impl Iterator for Chars<'_> {
    type Item = Char;

    fn next(&mut self) -> Option<Self::Item> {
        let (start, value) = self.inner.next()?;
        let end = start + value.len_utf8();
        let span = (start..end).into();
        Some(Char { span, value })
    }
}
