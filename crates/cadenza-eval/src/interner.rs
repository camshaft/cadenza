//! String interning for efficient identifier comparison.
//!
//! All identifiers in the AST and environment are interned once during parsing.
//! This provides O(1) comparison and hashing for identifier lookups.

use crate::map::StringMap;
use std::fmt;

/// An interned identifier, represented as a u32 index.
///
/// `InternedId` implements `Copy`, `Eq`, `Hash` with zero-cost comparison.
/// The actual string is stored in the [`Interner`] and can be looked up when needed.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct InternedId(u32);

impl InternedId {
    /// Returns the raw index of this interned ID.
    #[inline]
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for InternedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InternedId({})", self.0)
    }
}

impl fmt::Display for InternedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A string interner that maps strings to unique `InternedId`s.
///
/// The interner is the single source of truth for identifier hashing.
/// If the hasher ever needs to change, only this struct needs to be modified.
#[derive(Debug, Default)]
pub struct Interner {
    /// Maps strings to their interned IDs (using FxHash for performance)
    map: StringMap<InternedId>,
    /// Reverse lookup: maps IDs back to strings
    strings: Vec<String>,
}

impl Interner {
    /// Creates a new empty interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns a string, returning its unique ID.
    ///
    /// If the string has already been interned, returns the existing ID.
    /// Otherwise, assigns a new ID and stores the string.
    pub fn intern(&mut self, s: &str) -> InternedId {
        if let Some(&id) = self.map.get(s) {
            return id;
        }

        let id = InternedId(self.strings.len() as u32);
        let owned = s.to_string();
        self.strings.push(owned.clone());
        self.map.insert(owned, id);
        id
    }

    /// Looks up the string for an interned ID.
    ///
    /// # Panics
    ///
    /// Panics if the ID was not created by this interner.
    pub fn resolve(&self, id: InternedId) -> &str {
        &self.strings[id.0 as usize]
    }

    /// Tries to look up an existing interned ID without creating a new one.
    pub fn get(&self, s: &str) -> Option<InternedId> {
        self.map.get(s).copied()
    }

    /// Returns the number of interned strings.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Returns true if no strings have been interned.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_same_string_returns_same_id() {
        let mut interner = Interner::new();
        let id1 = interner.intern("hello");
        let id2 = interner.intern("hello");
        assert_eq!(id1, id2);
    }

    #[test]
    fn intern_different_strings_returns_different_ids() {
        let mut interner = Interner::new();
        let id1 = interner.intern("hello");
        let id2 = interner.intern("world");
        assert_ne!(id1, id2);
    }

    #[test]
    fn resolve_returns_original_string() {
        let mut interner = Interner::new();
        let id = interner.intern("test_string");
        assert_eq!(interner.resolve(id), "test_string");
    }

    #[test]
    fn get_returns_existing_id() {
        let mut interner = Interner::new();
        let id = interner.intern("existing");
        assert_eq!(interner.get("existing"), Some(id));
        assert_eq!(interner.get("nonexistent"), None);
    }

    #[test]
    fn len_tracks_unique_strings() {
        let mut interner = Interner::new();
        assert_eq!(interner.len(), 0);
        interner.intern("a");
        assert_eq!(interner.len(), 1);
        interner.intern("b");
        assert_eq!(interner.len(), 2);
        interner.intern("a"); // duplicate
        assert_eq!(interner.len(), 2);
    }
}
