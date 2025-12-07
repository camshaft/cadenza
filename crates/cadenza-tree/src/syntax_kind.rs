//! Raw syntax kind type used internally by the tree.

use std::num::NonZeroU16;

/// Internal representation of a syntax kind.
///
/// This is a simple wrapper around a u16 that provides type safety.
/// User-defined token kinds are converted to/from this type via the Language trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SyntaxKind(pub NonZeroU16);

impl SyntaxKind {
    /// Create a new SyntaxKind from a u16.
    ///
    /// # Panics
    /// Panics if value is 0.
    #[inline]
    pub fn new(value: u16) -> Self {
        Self(NonZeroU16::new(value).expect("SyntaxKind cannot be 0"))
    }

    /// Get the raw u16 value.
    #[inline]
    pub fn into_raw(self) -> u16 {
        self.0.get()
    }
}

impl From<u16> for SyntaxKind {
    fn from(value: u16) -> Self {
        Self::new(value)
    }
}

impl From<SyntaxKind> for u16 {
    fn from(kind: SyntaxKind) -> Self {
        kind.into_raw()
    }
}
