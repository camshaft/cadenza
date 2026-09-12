//! The format's error type — implements `serde::ser::Error` + `serde::de::Error` so serde's derive
//! machinery can construct failures, plus `std::error::Error`/`Display` for host use.

use std::fmt;

/// An (de)serialization error from the binary-AST serde format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A `serde` error carrying its own message (a required field missing, a length mismatch, a
    /// custom `de::Error::custom`, etc.).
    Message(String),
    /// `from_bytes` was handed bytes that are not a valid canonical binary-AST (`codec::decode`
    /// returned `None`).
    MalformedBinaryAst,
    /// The decoded AST node did not have the shape the target type expects — e.g. a bool was
    /// requested but the node is an integer atom, or a struct was requested but the node is not a
    /// record. Carries a short description of the mismatch.
    UnexpectedShape(String),
    /// An integer value in the AST did not fit the target machine width (e.g. a `u64`-range value
    /// deserialized into a `u8`).
    IntOutOfRange,
    /// A `char` was requested but the AST held a non-scalar / malformed char marker.
    BadChar,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Message(m) => write!(f, "{m}"),
            Error::MalformedBinaryAst => write!(f, "input is not a valid canonical binary-AST"),
            Error::UnexpectedShape(m) => write!(f, "unexpected AST shape: {m}"),
            Error::IntOutOfRange => write!(f, "integer value out of range for the target type"),
            Error::BadChar => write!(f, "AST char leaf is not a valid scalar value"),
        }
    }
}

impl std::error::Error for Error {}

impl serde::ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::Message(msg.to_string())
    }
}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::Message(msg.to_string())
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, Error>;
