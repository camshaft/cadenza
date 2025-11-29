//! Interning infrastructure for efficient value comparison and storage.
//!
//! This module provides a flexible interning system that is parameterized over
//! a zero-sized type (ZST) that determines where the storage is located.
//! This allows for both static/global interning and local interning.
//!
//! # Design
//!
//! The interning system is built around the `Interned<S>` type, where `S` is a
//! storage marker (ZST) that determines where values are stored. The `Interned`
//! type implements `Deref`, allowing direct access to the interned value.
//!
//! # Example
//!
//! ```
//! use cadenza_eval::interner::{Interner, InternedId};
//!
//! let mut interner = Interner::new();
//! let id1 = interner.intern("hello");
//! let id2 = interner.intern("hello");
//! assert_eq!(id1, id2); // Same string → same ID
//! assert_eq!(interner.resolve(id1), "hello");
//! ```

use crate::map::StringMap;
use std::{fmt, marker::PhantomData, ops::Deref};

// =============================================================================
// Core Types (backwards compatible)
// =============================================================================

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

// =============================================================================
// Storage Trait
// =============================================================================

/// A trait for intern storage, parameterized by a ZST marker type.
///
/// Storage implementations determine where interned values are stored.
/// The storage must provide static lifetime references for `Deref` to work.
///
/// # Type Parameters
///
/// - `Index`: The index type used to reference stored values (typically `u32`).
/// - `Value`: The type of values stored (can be unsized, e.g., `str`).
pub trait Storage: Sized + 'static {
    /// The index type for referencing stored values.
    type Index: Copy;

    /// The value type stored in this storage.
    type Value: 'static + ?Sized;

    /// Inserts a value into storage and returns its index.
    fn insert(value: &str) -> Self::Index;

    /// Resolves an index to a static reference to the stored value.
    fn resolve(index: Self::Index) -> &'static Self::Value;
}

// =============================================================================
// Interned: The main interning type
// =============================================================================

/// An interned value that is parameterized over its storage.
///
/// `Interned<S>` represents a value that has been interned into storage `S`.
/// It implements `Deref` to provide direct access to the stored value.
///
/// # Type Parameters
///
/// - `S`: The storage type (a ZST marker) that determines where values are stored.
///
/// # Example
///
/// ```ignore
/// define_string_storage!(MyStrings);
///
/// let s: Interned<MyStrings> = Interned::from("hello");
/// assert_eq!(&*s, "hello");
/// ```
pub struct Interned<S: Storage> {
    index: S::Index,
    _marker: PhantomData<S>,
}

// Manual implementations to avoid requiring S::Index bounds
impl<S: Storage> Clone for Interned<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: Storage> Copy for Interned<S> {}

impl<S: Storage> PartialEq for Interned<S>
where
    S::Index: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<S: Storage> Eq for Interned<S> where S::Index: Eq {}

impl<S: Storage> std::hash::Hash for Interned<S>
where
    S::Index: std::hash::Hash,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<S: Storage> Interned<S> {
    /// Creates a new `Interned` value from a string.
    ///
    /// The string is inserted into the storage if not already present.
    pub fn from(value: &str) -> Self {
        Self {
            index: S::insert(value),
            _marker: PhantomData,
        }
    }

    /// Returns the raw index of this interned value.
    #[inline]
    pub fn index(self) -> S::Index {
        self.index
    }

    /// Creates an `Interned` from a raw index.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the index is valid for this storage.
    #[inline]
    pub fn from_index(index: S::Index) -> Self {
        Self {
            index,
            _marker: PhantomData,
        }
    }
}

impl<S: Storage> Deref for Interned<S> {
    type Target = S::Value;

    fn deref(&self) -> &Self::Target {
        S::resolve(self.index)
    }
}

impl<S: Storage> fmt::Debug for Interned<S>
where
    S::Value: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<S: Storage> fmt::Display for Interned<S>
where
    S::Value: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

// =============================================================================
// Macro to define string storage
// =============================================================================

/// A macro to define string storage for interning.
///
/// This creates a ZST marker type and implements `Storage` for it
/// using thread-local storage. The generated type does not implement
/// `Sync` or `Send` to prevent the interned values from being sent
/// to a different thread.
///
/// # Example
///
/// ```
/// use cadenza_eval::define_string_storage;
/// use cadenza_eval::interner::Interned;
///
/// define_string_storage!(MyStrings);
///
/// let s: Interned<MyStrings> = Interned::from("hello");
/// assert_eq!(&*s, "hello");
/// ```
#[macro_export]
macro_rules! define_string_storage {
    ($name:ident) => {
        /// A ZST marker for string storage.
        ///
        /// This type does not implement `Sync` or `Send` to ensure
        /// interned values cannot be sent to a different thread.
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name {
            // PhantomData to prevent Sync/Send
            _not_send_sync: ::std::marker::PhantomData<*const ()>,
        }

        #[allow(dead_code)]
        impl $name {
            /// Creates a new storage marker.
            pub const fn new() -> Self {
                Self {
                    _not_send_sync: ::std::marker::PhantomData,
                }
            }
        }

        ::std::thread_local! {
            static STORAGE: ::std::cell::RefCell<$crate::interner::StringStorageData> =
                ::std::cell::RefCell::new($crate::interner::StringStorageData::new());
        }

        impl $crate::interner::Storage for $name {
            type Index = u32;
            type Value = str;

            fn insert(value: &str) -> Self::Index {
                STORAGE.with(|data| data.borrow_mut().insert(value))
            }

            fn resolve(index: Self::Index) -> &'static Self::Value {
                STORAGE.with(|data| {
                    // SAFETY: The string lives in thread-local storage for the lifetime
                    // of the thread. We return a 'static reference because the storage
                    // outlives any single call. The PhantomData in the storage marker
                    // prevents Send/Sync, ensuring the reference isn't used across threads.
                    let borrowed = data.borrow();
                    let s = borrowed.get(index);
                    unsafe { ::std::mem::transmute::<&str, &'static str>(s) }
                })
            }
        }
    };
}

/// Internal data structure for string storage.
///
/// This is used by the `define_string_storage!` macro.
#[derive(Debug, Default)]
pub struct StringStorageData {
    /// Maps strings to their indices for deduplication.
    map: rustc_hash::FxHashMap<String, u32>,
    /// The stored strings, indexed by their ID.
    strings: Vec<String>,
}

impl StringStorageData {
    /// Creates a new empty string storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a string and returns its index.
    pub fn insert(&mut self, s: &str) -> u32 {
        if let Some(&index) = self.map.get(s) {
            return index;
        }

        let index = self.strings.len() as u32;
        let owned = s.to_string();
        self.strings.push(owned.clone());
        self.map.insert(owned, index);
        index
    }

    /// Gets a string by index.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds.
    pub fn get(&self, index: u32) -> &str {
        &self.strings[index as usize]
    }
}

// =============================================================================
// String Interner (backwards compatible)
// =============================================================================

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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for the original Interner (backwards compatibility)
    mod interner {
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

    // Tests for the new Interned type with static storage
    mod interned {
        use super::*;

        define_string_storage!(TestStrings);

        #[test]
        fn interned_from_creates_value() {
            let s: Interned<TestStrings> = Interned::from("hello");
            assert_eq!(&*s, "hello");
        }

        #[test]
        fn interned_deduplicates() {
            let s1: Interned<TestStrings> = Interned::from("world");
            let s2: Interned<TestStrings> = Interned::from("world");
            assert_eq!(s1.index(), s2.index());
        }

        #[test]
        fn interned_is_copy() {
            let s1: Interned<TestStrings> = Interned::from("copy_test");
            let s2 = s1;
            assert_eq!(&*s1, &*s2);
        }

        #[test]
        fn interned_equality() {
            let s1: Interned<TestStrings> = Interned::from("eq_test");
            let s2: Interned<TestStrings> = Interned::from("eq_test");
            let s3: Interned<TestStrings> = Interned::from("different");
            assert_eq!(s1, s2);
            assert_ne!(s1, s3);
        }

        #[test]
        fn interned_display() {
            let s: Interned<TestStrings> = Interned::from("display_test");
            assert_eq!(format!("{}", s), "display_test");
        }

        #[test]
        fn interned_debug() {
            let s: Interned<TestStrings> = Interned::from("debug_test");
            assert_eq!(format!("{:?}", s), "\"debug_test\"");
        }
    }

    // Tests for string storage data
    mod string_storage_data {
        use super::*;

        #[test]
        fn insert_and_get() {
            let mut data = StringStorageData::new();
            let idx = data.insert("hello");
            assert_eq!(data.get(idx), "hello");
        }

        #[test]
        fn insert_deduplicates() {
            let mut data = StringStorageData::new();
            let idx1 = data.insert("same");
            let idx2 = data.insert("same");
            assert_eq!(idx1, idx2);
        }

        #[test]
        fn different_strings_different_indices() {
            let mut data = StringStorageData::new();
            let idx1 = data.insert("one");
            let idx2 = data.insert("two");
            assert_ne!(idx1, idx2);
        }
    }
}
