//! Efficient text representation for syntax nodes.

use crate::interner::InternedString;
use std::{borrow::Cow, fmt, ops::Deref};

/// Text of a syntax node.
///
/// This uses interned strings for efficient comparison and cloning.
/// String comparisons are O(1) via index comparison.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyntaxText(InternedString);

impl SyntaxText {
    /// Create text from an interned string.
    pub fn new(s: InternedString) -> Self {
        Self(s)
    }

    /// Create text from a string slice by interning it.
    pub fn from_str(s: &str) -> Self {
        Self(InternedString::new(s))
    }

    /// Get the text as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
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
    
    /// Get the underlying interned string.
    pub fn interned(&self) -> InternedString {
        self.0
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

impl From<&str> for SyntaxText {
    fn from(s: &str) -> Self {
        Self::from_str(s)
    }
}

impl From<String> for SyntaxText {
    fn from(s: String) -> Self {
        Self::from_str(&s)
    }
}

impl From<InternedString> for SyntaxText {
    fn from(s: InternedString) -> Self {
        Self::new(s)
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
