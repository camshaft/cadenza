//! Efficient text representation for syntax nodes.

use std::{borrow::Cow, fmt, ops::Deref};

/// Text of a syntax node.
///
/// This is similar to Rowan's SyntaxText but simpler. It represents the text
/// of a node as either a borrowed str or an owned String.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SyntaxText(TextInner);

#[derive(Clone, PartialEq, Eq, Hash)]
enum TextInner {
    Borrowed(&'static str),
    Owned(String),
}

impl SyntaxText {
    /// Create text from a borrowed static string.
    pub fn borrowed(s: &'static str) -> Self {
        Self(TextInner::Borrowed(s))
    }

    /// Create text from an owned string.
    pub fn owned(s: String) -> Self {
        Self(TextInner::Owned(s))
    }

    /// Get the text as a string slice.
    pub fn as_str(&self) -> &str {
        match &self.0 {
            TextInner::Borrowed(s) => s,
            TextInner::Owned(s) => s,
        }
    }

    /// Get the length of the text in bytes.
    pub fn len(&self) -> usize {
        self.as_str().len()
    }

    /// Check if the text is empty.
    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    /// Convert to a Cow for flexible ownership.
    pub fn to_cow(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.as_str())
    }
}

impl Deref for SyntaxText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for SyntaxText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for SyntaxText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl From<&'static str> for SyntaxText {
    fn from(s: &'static str) -> Self {
        Self::borrowed(s)
    }
}

impl From<String> for SyntaxText {
    fn from(s: String) -> Self {
        Self::owned(s)
    }
}

impl PartialEq<str> for SyntaxText {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for SyntaxText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
