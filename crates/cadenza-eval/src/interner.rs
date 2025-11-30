//! Interning infrastructure for efficient value comparison and storage.
//!
//! This module provides interned types for strings, integers, and floats.
//! Each type uses static storage with `OnceLock` for thread-safe initialization.
//!
//! # Design
//!
//! The interning system is built around the `Interned<S>` type, where `S` is a
//! storage marker (ZST) that determines where values are stored. The `Interned`
//! type implements `Deref`, allowing direct access to the interned value.

use std::{
    fmt,
    marker::PhantomData,
    num::{ParseFloatError, ParseIntError},
    ops::Deref,
    sync::OnceLock,
};

// =============================================================================
// Storage Trait
// =============================================================================

/// A trait for intern storage, parameterized by a ZST marker type.
///
/// Storage implementations determine where interned values are stored.
/// The storage must provide static lifetime references for `Deref` to work.
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
    #[inline]
    pub fn new(value: &str) -> Self {
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
}

impl<S: Storage> From<&str> for Interned<S> {
    fn from(value: &str) -> Self {
        Self::new(value)
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
// String Storage
// =============================================================================

/// Storage for interned strings.
///
/// This type is `Send + Sync` since the underlying storage is a static `OnceLock`
/// protected by a `Mutex`, making it safe to share across threads.
#[derive(Debug, Clone, Copy, Default)]
pub struct Strings;

/// Internal storage data for strings.
struct StringData {
    map: rustc_hash::FxHashMap<String, u32>,
    strings: Vec<String>,
}

impl StringData {
    fn new() -> Self {
        Self {
            map: rustc_hash::FxHashMap::default(),
            strings: Vec::new(),
        }
    }

    fn insert(&mut self, s: &str) -> u32 {
        if let Some(&index) = self.map.get(s) {
            return index;
        }
        let index = self.strings.len() as u32;
        let owned = s.to_string();
        self.strings.push(owned.clone());
        self.map.insert(owned, index);
        index
    }

    fn get(&self, index: u32) -> &str {
        &self.strings[index as usize]
    }
}

static STRING_STORAGE: OnceLock<std::sync::Mutex<StringData>> = OnceLock::new();

fn string_storage() -> &'static std::sync::Mutex<StringData> {
    STRING_STORAGE.get_or_init(|| std::sync::Mutex::new(StringData::new()))
}

impl Storage for Strings {
    type Index = u32;
    type Value = str;

    fn insert(value: &str) -> Self::Index {
        string_storage().lock().unwrap().insert(value)
    }

    fn resolve(index: Self::Index) -> &'static Self::Value {
        // SAFETY: Strings are never removed from storage, so the reference is valid
        // for the lifetime of the program.
        let storage = string_storage().lock().unwrap();
        let s = storage.get(index);
        unsafe { std::mem::transmute::<&str, &'static str>(s) }
    }
}

/// Type alias for interned strings.
pub type InternedString = Interned<Strings>;

// =============================================================================
// Integer Storage
// =============================================================================

/// Storage for interned integer literals.
///
/// Integer literals are stored as `Result<i64, ParseIntError>` where `Err` contains
/// the parse error. This allows us to intern the literal string and cache the
/// parse result with meaningful error messages.
///
/// This type is `Send + Sync` since the underlying storage is a static `OnceLock`
/// protected by a `Mutex`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Integers;

/// Internal storage data for integers.
struct IntegerData {
    map: rustc_hash::FxHashMap<String, u32>,
    values: Vec<Result<i64, ParseIntError>>,
}

impl IntegerData {
    fn new() -> Self {
        Self {
            map: rustc_hash::FxHashMap::default(),
            values: Vec::new(),
        }
    }

    fn insert(&mut self, s: &str) -> u32 {
        if let Some(&index) = self.map.get(s) {
            return index;
        }
        let index = self.values.len() as u32;
        // Parse the integer, removing underscores
        let clean = s.replace('_', "");
        let value = clean.parse::<i64>();
        self.values.push(value);
        self.map.insert(s.to_string(), index);
        index
    }

    fn get(&self, index: u32) -> &Result<i64, ParseIntError> {
        &self.values[index as usize]
    }
}

static INTEGER_STORAGE: OnceLock<std::sync::Mutex<IntegerData>> = OnceLock::new();

fn integer_storage() -> &'static std::sync::Mutex<IntegerData> {
    INTEGER_STORAGE.get_or_init(|| std::sync::Mutex::new(IntegerData::new()))
}

impl Storage for Integers {
    type Index = u32;
    type Value = Result<i64, ParseIntError>;

    fn insert(value: &str) -> Self::Index {
        integer_storage().lock().unwrap().insert(value)
    }

    fn resolve(index: Self::Index) -> &'static Self::Value {
        let storage = integer_storage().lock().unwrap();
        let v = storage.get(index);
        unsafe {
            std::mem::transmute::<&Result<i64, ParseIntError>, &'static Result<i64, ParseIntError>>(
                v,
            )
        }
    }
}

/// Type alias for interned integer literals.
pub type InternedInteger = Interned<Integers>;

// =============================================================================
// Float Storage
// =============================================================================

/// Storage for interned float literals.
///
/// Float literals are stored as `Result<f64, ParseFloatError>` where `Err` contains
/// the parse error with meaningful error messages.
///
/// This type is `Send + Sync` since the underlying storage is a static `OnceLock`
/// protected by a `Mutex`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Floats;

/// Internal storage data for floats.
struct FloatData {
    map: rustc_hash::FxHashMap<String, u32>,
    values: Vec<Result<f64, ParseFloatError>>,
}

impl FloatData {
    fn new() -> Self {
        Self {
            map: rustc_hash::FxHashMap::default(),
            values: Vec::new(),
        }
    }

    fn insert(&mut self, s: &str) -> u32 {
        if let Some(&index) = self.map.get(s) {
            return index;
        }
        let index = self.values.len() as u32;
        // Parse the float, removing underscores
        let clean = s.replace('_', "");
        let value = clean.parse::<f64>();
        self.values.push(value);
        self.map.insert(s.to_string(), index);
        index
    }

    fn get(&self, index: u32) -> &Result<f64, ParseFloatError> {
        &self.values[index as usize]
    }
}

static FLOAT_STORAGE: OnceLock<std::sync::Mutex<FloatData>> = OnceLock::new();

fn float_storage() -> &'static std::sync::Mutex<FloatData> {
    FLOAT_STORAGE.get_or_init(|| std::sync::Mutex::new(FloatData::new()))
}

impl Storage for Floats {
    type Index = u32;
    type Value = Result<f64, ParseFloatError>;

    fn insert(value: &str) -> Self::Index {
        float_storage().lock().unwrap().insert(value)
    }

    fn resolve(index: Self::Index) -> &'static Self::Value {
        let storage = float_storage().lock().unwrap();
        let v = storage.get(index);
        unsafe {
            std::mem::transmute::<
                &Result<f64, ParseFloatError>,
                &'static Result<f64, ParseFloatError>,
            >(v)
        }
    }
}

/// Type alias for interned float literals.
pub type InternedFloat = Interned<Floats>;

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod interned_string {
        use super::*;

        #[test]
        fn from_creates_value() {
            let s: InternedString = "hello".into();
            assert_eq!(&*s, "hello");
        }

        #[test]
        fn new_creates_value() {
            let s = InternedString::new("world");
            assert_eq!(&*s, "world");
        }

        #[test]
        fn deduplicates() {
            let s1: InternedString = "dedup_test".into();
            let s2: InternedString = "dedup_test".into();
            assert_eq!(s1.index(), s2.index());
        }

        #[test]
        fn is_copy() {
            let s1: InternedString = "copy_test".into();
            let s2 = s1;
            assert_eq!(&*s1, &*s2);
        }

        #[test]
        fn equality() {
            let s1: InternedString = "eq_test".into();
            let s2: InternedString = "eq_test".into();
            let s3: InternedString = "different".into();
            assert_eq!(s1, s2);
            assert_ne!(s1, s3);
        }

        #[test]
        fn display() {
            let s: InternedString = "display_test".into();
            assert_eq!(format!("{}", s), "display_test");
        }

        #[test]
        fn debug() {
            let s: InternedString = "debug_test".into();
            assert_eq!(format!("{:?}", s), "\"debug_test\"");
        }
    }

    mod interned_integer {
        use super::*;

        #[test]
        fn parses_valid_integer() {
            let i: InternedInteger = "42".into();
            assert_eq!(*i, Ok(42));
        }

        #[test]
        fn handles_underscores() {
            let i: InternedInteger = "1_000_000".into();
            assert_eq!(*i, Ok(1_000_000));
        }

        #[test]
        fn returns_err_for_invalid() {
            let i: InternedInteger = "not_a_number".into();
            assert!(i.is_err());
            // Check that we can get a meaningful error message
            let err = i.as_ref().unwrap_err();
            assert!(!err.to_string().is_empty());
        }

        #[test]
        fn deduplicates() {
            let i1: InternedInteger = "123".into();
            let i2: InternedInteger = "123".into();
            assert_eq!(i1.index(), i2.index());
        }

        #[test]
        fn handles_negative() {
            let i: InternedInteger = "-42".into();
            assert_eq!(*i, Ok(-42));
        }
    }

    mod interned_float {
        use super::*;

        #[test]
        fn parses_valid_float() {
            let f: InternedFloat = "3.5".into();
            assert_eq!(*f, Ok(3.5));
        }

        #[test]
        fn handles_underscores() {
            let f: InternedFloat = "1_000.5".into();
            assert_eq!(*f, Ok(1000.5));
        }

        #[test]
        fn returns_err_for_invalid() {
            let f: InternedFloat = "not_a_float".into();
            assert!(f.is_err());
            // Check that we can get a meaningful error message
            let err = f.as_ref().unwrap_err();
            assert!(!err.to_string().is_empty());
        }

        #[test]
        fn deduplicates() {
            let f1: InternedFloat = "2.5".into();
            let f2: InternedFloat = "2.5".into();
            assert_eq!(f1.index(), f2.index());
        }
    }
}
