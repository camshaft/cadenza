//! `Str` — a cheaply-clonable, `Bytes`-backed UTF-8 string (`design/cadenza-platform.md` §12).
//!
//! Everywhere a value would otherwise be a `String` (or `Arc<str>`) — an id, a name, a reason, a
//! target — the platform uses [`Str`]. Why: those text values are cloned constantly as they thread
//! through routing, dispatch, and results, and a `String` clone is an allocation + copy; a `Str` clone
//! is an O(1) `Bytes` refcount bump. It also gives text and bytes ONE representation, so a value crosses
//! the text/binary boundary without re-allocating.
//!
//! We build our own newtype (over `bytes::Bytes`) rather than pull a third-party str-bytes crate, so the
//! platform owns this type outright (operator directive 2026-08-20). Invariant: the wrapped `Bytes` is
//! always valid UTF-8 — every constructor establishes it, so [`Str::as_str`] is a zero-cost view.

use bytes::Bytes;
use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;
use std::str::Utf8Error;

/// A `Bytes`-backed UTF-8 string. `Clone` is an O(1) refcount bump. Deref/Borrow/AsRef to `str` so it
/// works anywhere a `&str` does; equality and ordering are by string content.
#[derive(Clone, Default)]
pub struct Str(Bytes);

impl Str {
    /// The empty string.
    #[must_use]
    pub const fn new() -> Self {
        Self(Bytes::new())
    }

    /// Borrow the text. Zero-cost: the invariant guarantees the bytes are valid UTF-8.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // SAFETY: every constructor establishes and preserves the "wrapped Bytes is valid UTF-8"
        // invariant (the fallible ones validate; the `unchecked` one documents the caller's obligation),
        // and `Bytes` is immutable, so the bytes are still valid UTF-8 here.
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }

    /// The underlying UTF-8 bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consume into the backing `Bytes` (O(1)) — for crossing to the binary side without a copy.
    #[must_use]
    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    /// Length in bytes (not chars), like `str::len`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the string is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Wrap `Bytes` as a `Str`, validating UTF-8. The O(1)-clone entry point for text that arrived as
    /// bytes (a payload, a wire field) without re-allocating.
    ///
    /// # Errors
    /// Returns the [`Utf8Error`] if `bytes` is not valid UTF-8.
    pub fn from_utf8(bytes: Bytes) -> Result<Self, Utf8Error> {
        std::str::from_utf8(&bytes)?;
        Ok(Self(bytes))
    }

    /// Wrap `Bytes` as a `Str` WITHOUT validating UTF-8.
    ///
    /// # Safety
    /// The caller must guarantee `bytes` is valid UTF-8; otherwise [`Str::as_str`] is undefined behaviour.
    #[must_use]
    pub const unsafe fn from_utf8_unchecked(bytes: Bytes) -> Self {
        Self(bytes)
    }
}

impl From<&str> for Str {
    fn from(s: &str) -> Self {
        Self(Bytes::copy_from_slice(s.as_bytes()))
    }
}

impl From<String> for Str {
    fn from(s: String) -> Self {
        // reuses the String's allocation as the Bytes buffer — no copy.
        Self(Bytes::from(s.into_bytes()))
    }
}

impl Deref for Str {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Str {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Str {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

// Content equality/ordering + hashing — by the string, so a `Str` interns/compares like its text and
// works as a map/set key alongside `&str` lookups (via `Borrow<str>`).
impl PartialEq for Str {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for Str {}
impl PartialOrd for Str {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Str {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}
impl std::hash::Hash for Str {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // hash as the str would, so `Borrow<str>` keys hash-match a `&str` lookup.
        self.as_str().hash(state);
    }
}

// Convenience equality with the primitive string types (so tests + call sites read naturally).
impl PartialEq<str> for Str {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<&str> for Str {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[cfg(test)]
mod tests {
    use super::Str;
    use bytes::Bytes;
    use std::collections::HashMap;

    #[test]
    fn from_and_views_round_trip() {
        let s = Str::from("cadenza");
        assert_eq!(s.as_str(), "cadenza");
        assert_eq!(s.as_bytes(), b"cadenza");
        assert_eq!(s.len(), 7);
        assert!(!s.is_empty());
        assert!(Str::new().is_empty());
        assert_eq!(Str::default().as_str(), "");
        // From<String> reuses the allocation; content preserved.
        assert_eq!(Str::from(String::from("héllo")).as_str(), "héllo");
        // into_bytes gives back the same bytes.
        assert_eq!(Str::from("x").into_bytes(), Bytes::from_static(b"x"));
    }

    #[test]
    fn from_utf8_validates() {
        assert_eq!(
            Str::from_utf8(Bytes::from_static(b"ok")).unwrap().as_str(),
            "ok"
        );
        // 0xFF is not valid UTF-8.
        assert!(Str::from_utf8(Bytes::from_static(&[0xFF, 0xFE])).is_err());
        // multi-byte UTF-8 survives.
        let e = "café".to_string().into_bytes();
        assert_eq!(Str::from_utf8(Bytes::from(e)).unwrap().as_str(), "café");
    }

    #[test]
    fn clone_shares_the_buffer() {
        // A clone is a refcount bump on the same allocation, not a copy — assert they alias.
        let a = Str::from("shared buffer, one allocation");
        let b = a.clone();
        assert_eq!(a, b);
        assert_eq!(
            a.as_bytes().as_ptr(),
            b.as_bytes().as_ptr(),
            "clone must share the buffer"
        );
    }

    #[test]
    fn deref_and_equality_with_primitives() {
        let s = Str::from("verb");
        // Deref<str>: str methods work directly.
        assert!(s.starts_with("ve"));
        assert_eq!(s.to_uppercase(), "VERB");
        // equality with &str / str.
        assert_eq!(s, "verb");
        assert_eq!(s, *"verb");
        assert_ne!(s, "other");
    }

    #[test]
    fn borrow_str_lets_a_str_key_look_up_a_str_map() {
        // Str hashes + compares as its text, so a HashMap<Str, _> is queryable by &str (Borrow<str>).
        let mut m: HashMap<Str, i32> = HashMap::new();
        m.insert(Str::from("key"), 7);
        assert_eq!(m.get("key"), Some(&7));
    }

    #[test]
    fn ordering_is_by_content() {
        let mut v = [Str::from("banana"), Str::from("apple"), Str::from("cherry")];
        v.sort();
        assert_eq!(
            v.iter().map(Str::as_str).collect::<Vec<_>>(),
            ["apple", "banana", "cherry"]
        );
    }
}
