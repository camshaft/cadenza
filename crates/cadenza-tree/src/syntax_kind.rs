//! Raw syntax kind type used internally by the tree.

/// Internal representation of a syntax kind.
///
/// This is a simple wrapper around a u16 that provides type safety.
/// User-defined token kinds are converted to/from this type via the Language trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SyntaxKind(pub u16);

impl SyntaxKind {
    /// Create a new SyntaxKind from a u16.
    #[inline]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Get the raw u16 value.
    #[inline]
    pub const fn into_raw(self) -> u16 {
        self.0
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
