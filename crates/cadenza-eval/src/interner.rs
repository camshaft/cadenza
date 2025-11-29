//! Interning infrastructure for efficient value comparison and storage.
//!
//! This module provides a flexible interning system that is parameterized over
//! a zero-sized type (ZST) that determines where the storage is located.
//! This allows for both static/global interning and local interning.
//!
//! # Design
//!
//! The interning system is built around two key concepts:
//!
//! 1. **Storage**: A ZST marker type that determines where interned values live.
//!    The storage can be static (global) or local (passed around).
//!
//! 2. **InternId**: A lightweight handle (u32 index) that acts like a pointer
//!    to the interned value. It can be resolved anywhere the storage is accessible.
//!
//! # Transforming Interning
//!
//! The interning system supports transformations during interning. For example,
//! you can intern a string literal and get back either the parsed integer value
//! or None if parsing fails.
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
use std::{cell::RefCell, fmt, hash::Hash, marker::PhantomData};

// =============================================================================
// Core Types
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
// Storage Trait and Implementations
// =============================================================================

/// A trait for intern storage, parameterized by the value type.
///
/// Storage implementations determine where interned values are stored.
/// This can be local storage (e.g., a `RefCell<Vec<T>>`) or static storage
/// (e.g., a thread-local or global map).
pub trait Storage<T> {
    /// Returns a reference to the storage data.
    fn with<R>(&self, f: impl FnOnce(&InternData<T>) -> R) -> R;

    /// Returns a mutable reference to the storage data.
    fn with_mut<R>(&self, f: impl FnOnce(&mut InternData<T>) -> R) -> R;
}

/// The internal data structure for interned values.
///
/// This holds the actual interned values and the forward/reverse lookup maps.
#[derive(Debug, Default)]
pub struct InternData<T> {
    /// The interned values, indexed by InternId.
    values: Vec<T>,
}

impl<T> InternData<T> {
    /// Creates a new empty intern data.
    pub fn new() -> Self {
        Self { values: Vec::new() }
    }

    /// Returns the number of interned values.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true if no values have been interned.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Adds a value and returns its ID.
    pub fn push(&mut self, value: T) -> InternId<T> {
        let id = InternId::new(self.values.len() as u32);
        self.values.push(value);
        id
    }

    /// Gets a value by ID.
    pub fn get(&self, id: InternId<T>) -> Option<&T> {
        self.values.get(id.index() as usize)
    }
}

/// Local storage that wraps a `RefCell<InternData<T>>`.
///
/// This is the default storage type for non-static interning.
#[derive(Debug, Default)]
pub struct LocalStorage<T> {
    data: RefCell<InternData<T>>,
}

impl<T> LocalStorage<T> {
    /// Creates a new empty local storage.
    pub fn new() -> Self {
        Self {
            data: RefCell::new(InternData::new()),
        }
    }
}

impl<T> Storage<T> for LocalStorage<T> {
    fn with<R>(&self, f: impl FnOnce(&InternData<T>) -> R) -> R {
        f(&self.data.borrow())
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut InternData<T>) -> R) -> R {
        f(&mut self.data.borrow_mut())
    }
}

// =============================================================================
// Static Storage
// =============================================================================

/// A marker trait for ZST types that define static storage locations.
///
/// Implementing this trait on a zero-sized type allows you to create
/// static interning locations that can be accessed from anywhere in the program.
///
/// # Example
///
/// ```
/// use cadenza_eval::interner::{StaticStorage, InternData, Storage};
/// use std::cell::RefCell;
///
/// // Define a ZST marker for string interning
/// pub struct StringStore;
///
/// // Thread-local storage for the strings
/// thread_local! {
///     static STRING_DATA: RefCell<InternData<String>> = RefCell::new(InternData::new());
/// }
///
/// impl StaticStorage<String> for StringStore {
///     fn with<R>(f: impl FnOnce(&InternData<String>) -> R) -> R {
///         STRING_DATA.with(|data| f(&data.borrow()))
///     }
///
///     fn with_mut<R>(f: impl FnOnce(&mut InternData<String>) -> R) -> R {
///         STRING_DATA.with(|data| f(&mut data.borrow_mut()))
///     }
/// }
/// ```
pub trait StaticStorage<T>: Sized {
    /// Access the storage data immutably.
    fn with<R>(f: impl FnOnce(&InternData<T>) -> R) -> R;

    /// Access the storage data mutably.
    fn with_mut<R>(f: impl FnOnce(&mut InternData<T>) -> R) -> R;
}

/// A wrapper that adapts a `StaticStorage` ZST to implement `Storage`.
///
/// This allows using static storage with the `Intern` type.
#[derive(Debug, Default)]
pub struct StaticStorageAdapter<S, T> {
    _marker: PhantomData<(S, T)>,
}

impl<S, T> StaticStorageAdapter<S, T> {
    /// Creates a new static storage adapter.
    pub const fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<S, T> Clone for StaticStorageAdapter<S, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S, T> Copy for StaticStorageAdapter<S, T> {}

impl<S: StaticStorage<T>, T> Storage<T> for StaticStorageAdapter<S, T> {
    fn with<R>(&self, f: impl FnOnce(&InternData<T>) -> R) -> R {
        S::with(f)
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut InternData<T>) -> R) -> R {
        S::with_mut(f)
    }
}

/// A macro to define static storage for a given type.
///
/// This creates a ZST marker type and implements `StaticStorage` for it
/// using thread-local storage.
///
/// # Example
///
/// ```
/// use cadenza_eval::define_static_storage;
///
/// // Define static storage for interned integers
/// define_static_storage!(IntegerStore, i64);
///
/// // Now you can use IntegerStore as a static storage marker
/// ```
#[macro_export]
macro_rules! define_static_storage {
    ($name:ident, $value_type:ty) => {
        /// A ZST marker for static storage.
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name;

        ::std::thread_local! {
            static STORAGE: ::std::cell::RefCell<$crate::interner::InternData<$value_type>> =
                ::std::cell::RefCell::new($crate::interner::InternData::new());
        }

        impl $crate::interner::StaticStorage<$value_type> for $name {
            fn with<R>(f: impl FnOnce(&$crate::interner::InternData<$value_type>) -> R) -> R {
                STORAGE.with(|data| f(&data.borrow()))
            }

            fn with_mut<R>(
                f: impl FnOnce(&mut $crate::interner::InternData<$value_type>) -> R,
            ) -> R {
                STORAGE.with(|data| f(&mut data.borrow_mut()))
            }
        }
    };
}

// =============================================================================
// Generic InternId with Type Parameter
// =============================================================================

/// A generic interned ID that carries type information.
///
/// `InternId<T>` represents an index into a storage of type `T`.
/// It implements `Copy`, `Eq`, `Hash` with zero-cost comparison.
pub struct InternId<T> {
    index: u32,
    _marker: PhantomData<fn() -> T>,
}

// Manual implementations to avoid requiring T: Copy/Clone
impl<T> Clone for InternId<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for InternId<T> {}

impl<T> PartialEq for InternId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T> Eq for InternId<T> {}

impl<T> std::hash::Hash for InternId<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<T> InternId<T> {
    /// Creates a new InternId with the given index.
    #[inline]
    fn new(index: u32) -> Self {
        Self {
            index,
            _marker: PhantomData,
        }
    }

    /// Returns the raw index of this interned ID.
    #[inline]
    pub fn index(self) -> u32 {
        self.index
    }

    /// Resolves this ID using the given storage.
    #[inline]
    pub fn resolve<S: Storage<T>>(self, storage: &S) -> Option<T>
    where
        T: Clone,
    {
        storage.with(|data| data.get(self).cloned())
    }

    /// Resolves this ID using static storage.
    ///
    /// This allows resolving the ID anywhere in the program where the
    /// static storage type `S` is accessible.
    #[inline]
    pub fn resolve_static<S: StaticStorage<T>>(self) -> Option<T>
    where
        T: Clone,
    {
        S::with(|data| data.get(self).cloned())
    }
}

impl<T> fmt::Debug for InternId<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InternId({})", self.index)
    }
}

impl<T> fmt::Display for InternId<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.index)
    }
}

// =============================================================================
// Intern: The main interning interface
// =============================================================================

/// A generic interner that can intern values with optional transformation.
///
/// `Intern<T, K, S>` interns values of type `T` with key type `K` into storage `S`.
/// It supports deduplication based on the key type.
#[derive(Debug)]
pub struct Intern<T, K = T, S = LocalStorage<T>>
where
    S: Storage<T>,
{
    storage: S,
    /// Maps keys to their interned IDs for deduplication.
    lookup: RefCell<rustc_hash::FxHashMap<K, InternId<T>>>,
    _marker: PhantomData<T>,
}

impl<T, K, S> Intern<T, K, S>
where
    S: Storage<T>,
    K: Eq + Hash + Clone,
{
    /// Creates a new interner with the given storage.
    pub fn with_storage(storage: S) -> Self {
        Self {
            storage,
            lookup: RefCell::new(rustc_hash::FxHashMap::default()),
            _marker: PhantomData,
        }
    }

    /// Interns a value, returning its ID.
    ///
    /// If a value with the same key has already been interned, returns the existing ID.
    pub fn intern(&self, key: K, value: T) -> InternId<T> {
        if let Some(&id) = self.lookup.borrow().get(&key) {
            return id;
        }

        let id = self.storage.with_mut(|data| data.push(value));
        self.lookup.borrow_mut().insert(key, id);
        id
    }

    /// Interns a value with a transformation function.
    ///
    /// The transformation function converts the key to a value.
    /// If the key has already been interned, returns the existing ID without
    /// calling the transformation function.
    pub fn intern_with<F>(&self, key: K, f: F) -> InternId<T>
    where
        F: FnOnce(&K) -> T,
    {
        if let Some(&id) = self.lookup.borrow().get(&key) {
            return id;
        }

        let value = f(&key);
        let id = self.storage.with_mut(|data| data.push(value));
        self.lookup.borrow_mut().insert(key, id);
        id
    }

    /// Looks up an existing interned ID without creating a new one.
    pub fn get(&self, key: &K) -> Option<InternId<T>> {
        self.lookup.borrow().get(key).copied()
    }

    /// Resolves an ID to its value.
    pub fn resolve(&self, id: InternId<T>) -> Option<T>
    where
        T: Clone,
    {
        self.storage.with(|data| data.get(id).cloned())
    }

    /// Returns a reference to the underlying storage.
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Returns the number of interned values.
    pub fn len(&self) -> usize {
        self.storage.with(|data| data.len())
    }

    /// Returns true if no values have been interned.
    pub fn is_empty(&self) -> bool {
        self.storage.with(|data| data.is_empty())
    }
}

impl<T, K> Intern<T, K, LocalStorage<T>>
where
    K: Eq + Hash + Clone,
{
    /// Creates a new interner with local storage.
    pub fn new() -> Self {
        Self::with_storage(LocalStorage::new())
    }
}

impl<T, K> Default for Intern<T, K, LocalStorage<T>>
where
    K: Eq + Hash + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

/// A type alias for an interner using static storage.
///
/// This is useful when you want to use a ZST marker type for static storage.
pub type StaticIntern<T, K, S> = Intern<T, K, StaticStorageAdapter<S, T>>;

impl<T, K, S> Intern<T, K, StaticStorageAdapter<S, T>>
where
    S: StaticStorage<T>,
    K: Eq + Hash + Clone,
{
    /// Creates a new interner using the specified static storage.
    pub fn with_static_storage() -> Self {
        Self::with_storage(StaticStorageAdapter::new())
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
// Transforming Interners for Literals
// =============================================================================

/// An interner for integer literals that parses and stores the integer value.
///
/// This allows interning integer literal strings and getting back
/// either the parsed value or None if parsing fails.
pub type IntegerIntern = Intern<Option<i64>, String>;

impl IntegerIntern {
    /// Interns an integer literal string, parsing it to an i64.
    ///
    /// Returns the InternId, which can be resolved to get the parsed value
    /// (or None if parsing failed).
    pub fn intern_integer(&self, s: &str) -> InternId<Option<i64>> {
        self.intern_with(s.to_string(), |key| {
            // Remove underscores for parsing (e.g., "1_000_000" -> "1000000")
            let clean = key.replace('_', "");
            clean.parse::<i64>().ok()
        })
    }

    /// Resolves an integer InternId to its parsed value.
    pub fn resolve_integer(&self, id: InternId<Option<i64>>) -> Option<i64> {
        self.resolve(id).flatten()
    }
}

/// An interner for float literals that parses and stores the float value.
pub type FloatIntern = Intern<Option<f64>, String>;

impl FloatIntern {
    /// Interns a float literal string, parsing it to an f64.
    pub fn intern_float(&self, s: &str) -> InternId<Option<f64>> {
        self.intern_with(s.to_string(), |key| {
            let clean = key.replace('_', "");
            clean.parse::<f64>().ok()
        })
    }

    /// Resolves a float InternId to its parsed value.
    pub fn resolve_float(&self, id: InternId<Option<f64>>) -> Option<f64> {
        self.resolve(id).flatten()
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

    // Tests for the new generic Intern type
    mod generic_intern {
        use super::*;

        #[test]
        fn intern_with_local_storage() {
            let interner: Intern<i32, i32> = Intern::new();
            let id1 = interner.intern(42, 42);
            let id2 = interner.intern(42, 42);
            assert_eq!(id1, id2);
            assert_eq!(interner.resolve(id1), Some(42));
        }

        #[test]
        fn intern_with_transformation() {
            let interner: Intern<i64, String> = Intern::new();
            let id = interner.intern_with("123".to_string(), |s| s.parse().unwrap());
            assert_eq!(interner.resolve(id), Some(123));
        }

        #[test]
        fn intern_deduplicates_by_key() {
            let interner: Intern<String, String> = Intern::new();
            let id1 = interner.intern("hello".to_string(), "world".to_string());
            let id2 = interner.intern("hello".to_string(), "ignored".to_string());
            assert_eq!(id1, id2);
            // The first value is kept
            assert_eq!(interner.resolve(id1), Some("world".to_string()));
        }

        #[test]
        fn get_returns_existing_id() {
            let interner: Intern<i32, String> = Intern::new();
            let id = interner.intern("test".to_string(), 42);
            assert_eq!(interner.get(&"test".to_string()), Some(id));
            assert_eq!(interner.get(&"nonexistent".to_string()), None);
        }

        #[test]
        fn len_tracks_unique_values() {
            let interner: Intern<i32, i32> = Intern::new();
            assert_eq!(interner.len(), 0);
            interner.intern(1, 100);
            assert_eq!(interner.len(), 1);
            interner.intern(2, 200);
            assert_eq!(interner.len(), 2);
            interner.intern(1, 100); // duplicate key
            assert_eq!(interner.len(), 2);
        }
    }

    // Tests for integer/float interning
    mod literal_interns {
        use super::*;

        #[test]
        fn integer_intern_parses_valid_integers() {
            let interner = IntegerIntern::new();
            let id = interner.intern_integer("42");
            assert_eq!(interner.resolve_integer(id), Some(42));
        }

        #[test]
        fn integer_intern_handles_underscores() {
            let interner = IntegerIntern::new();
            let id = interner.intern_integer("1_000_000");
            assert_eq!(interner.resolve_integer(id), Some(1_000_000));
        }

        #[test]
        fn integer_intern_returns_none_for_invalid() {
            let interner = IntegerIntern::new();
            let id = interner.intern_integer("not_a_number");
            assert_eq!(interner.resolve_integer(id), None);
        }

        #[test]
        fn integer_intern_deduplicates() {
            let interner = IntegerIntern::new();
            let id1 = interner.intern_integer("42");
            let id2 = interner.intern_integer("42");
            assert_eq!(id1, id2);
        }

        #[test]
        fn float_intern_parses_valid_floats() {
            let interner = FloatIntern::new();
            let id = interner.intern_float("2.5");
            assert_eq!(interner.resolve_float(id), Some(2.5));
        }

        #[test]
        fn float_intern_handles_underscores() {
            let interner = FloatIntern::new();
            let id = interner.intern_float("1_000.5");
            assert_eq!(interner.resolve_float(id), Some(1000.5));
        }
    }

    // Tests for InternId
    mod intern_id {
        use super::*;

        #[test]
        fn intern_id_is_copy() {
            let interner: Intern<i32, i32> = Intern::new();
            let id = interner.intern(42, 42);
            let id_copy = id;
            assert_eq!(id, id_copy);
        }

        #[test]
        fn intern_id_can_resolve_with_storage() {
            let storage = LocalStorage::<i32>::new();
            let id = storage.with_mut(|data| data.push(123));
            assert_eq!(id.resolve(&storage), Some(123));
        }
    }

    // Tests for static storage
    mod static_storage {
        use super::*;

        // Define static storage for testing
        define_static_storage!(TestIntStore, i32);

        #[test]
        fn static_storage_intern_and_resolve() {
            let interner: StaticIntern<i32, i32, TestIntStore> = Intern::with_static_storage();

            let id = interner.intern(42, 42);
            assert_eq!(interner.resolve(id), Some(42));
        }

        #[test]
        fn static_storage_deduplicates() {
            let interner: StaticIntern<i32, i32, TestIntStore> = Intern::with_static_storage();

            let id1 = interner.intern(100, 100);
            let id2 = interner.intern(100, 100);
            assert_eq!(id1, id2);
        }

        #[test]
        fn intern_id_can_resolve_with_static_storage() {
            // Use a separate static storage for this test to avoid interference
            define_static_storage!(TestResolveStore, String);

            let interner: StaticIntern<String, String, TestResolveStore> =
                Intern::with_static_storage();

            let id = interner.intern("hello".to_string(), "world".to_string());

            // Resolve using static storage directly (without the interner)
            let resolved = id.resolve_static::<TestResolveStore>();
            assert_eq!(resolved, Some("world".to_string()));
        }

        #[test]
        fn static_storage_with_transformation() {
            define_static_storage!(TestTransformStore, Option<i64>);

            let interner: StaticIntern<Option<i64>, String, TestTransformStore> =
                Intern::with_static_storage();

            let id = interner.intern_with("42".to_string(), |s| s.parse::<i64>().ok());
            assert_eq!(interner.resolve(id), Some(Some(42)));
        }
    }
}
