//! Hash map type alias using FxHash for better performance.
//!
//! The std HashMap has DoS protection which is unnecessary overhead
//! for a local compiler. We use rustc-hash's FxHasher instead.

use crate::interner::InternedString;
use rustc_hash::FxHasher;
use std::{collections::HashMap, hash::BuildHasherDefault};

/// A hash map using FxHash for interned string keys.
pub type Map<V> = HashMap<InternedString, V, BuildHasherDefault<FxHasher>>;
