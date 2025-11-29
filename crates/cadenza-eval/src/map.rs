//! Hash map type alias using FxHash for better performance.
//!
//! The std HashMap has DoS protection which is unnecessary overhead
//! for a local compiler. We use rustc-hash's FxHasher instead.

use crate::interner::InternedId;
use rustc_hash::FxHasher;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;

/// A hash map using FxHash for interned ID keys.
pub type Map<V> = HashMap<InternedId, V, BuildHasherDefault<FxHasher>>;

/// A hash map using FxHash for string keys.
pub type StringMap<V> = HashMap<String, V, BuildHasherDefault<FxHasher>>;
