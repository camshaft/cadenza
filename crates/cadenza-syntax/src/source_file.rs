//! Source file representation with content hashing.

use crate::contents::Contents;
use cadenza_tree::InternedString;
use core::{fmt, ops::Deref};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Hash)]
pub struct SourceFile {
    pub(crate) path: InternedString,
    pub(crate) contents: Contents,
}

impl SourceFile {
    pub fn new<C>(path: InternedString, contents: C) -> Result<Self, core::str::Utf8Error>
    where
        C: Into<Contents>,
    {
        let contents = contents.into();
        // Validate UTF-8
        let _ = core::str::from_utf8(&contents)?;
        Ok(Self { path, contents })
    }

    pub fn path(&self) -> InternedString {
        self.path
    }

    pub fn hash(&self) -> &[u8; 32] {
        self.contents.hash()
    }
}

impl Deref for SourceFile {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        unsafe {
            // Safety: UTF-8 validity was checked at creation
            core::str::from_utf8_unchecked(&self.contents)
        }
    }
}

impl AsRef<str> for SourceFile {
    fn as_ref(&self) -> &str {
        self
    }
}

impl fmt::Display for SourceFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)
    }
}
