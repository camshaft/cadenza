//! Diagnostics — a "no" as a first-class value produced where the decision is made.
//!
//! The compiler distinguishes three kinds of "no", ordered by safety
//! (`reference-compiler.md` §Outcomes Are Ordered By Safety): a **reject** (the program is
//! ill-formed — carries a stable machine-readable [`Code`]), a **decline** (the compiler does not
//! yet realize the construct — uncoded, an honest "not built yet"), and a **trap** (a run-time halt;
//! its compile-provable form is a poison, which is a reject with a code). This module gives the
//! value that carries a reject/decline; the kind is fixed at the point it is produced and carried
//! concretely, never reconstructed downstream from an artifact's shape
//! (`reference-compiler.md` §The Kind Of A "No" Is Fixed Where It Is Produced).
//!
//! Stage 0 needs only a handful of codes; the taxonomy grows one variant per added check, and its
//! `str` form is the stable `CDZ####` string a tool branches on.

/// A stable, machine-readable diagnostic code. Its `code()` string is the durable identity a
/// consumer matches on; the enum variant is the compiler-internal handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    /// A malformed or ill-typed construct that the compiler positively proves ill-formed.
    Malformed,
    /// A type mismatch (e.g. an `if` condition that is not a boolean; branches of differing type).
    TypeMismatch,
    /// An integer literal that does not fit the width its use requires.
    IntOutOfRange,
}

impl Code {
    /// The stable `CDZ####` string. These are the identities a tool and the corpus branch on, so
    /// they change only by the coordinated act a code taxonomy change is.
    pub fn code(self) -> &'static str {
        match self {
            Code::Malformed => "CDZ0201",
            Code::TypeMismatch => "CDZ0203",
            Code::IntOutOfRange => "CDZ0302",
        }
    }
}

/// A produced "no": either a coded rejection or an uncoded decline, each carrying a human message.
/// The `code` is `Some` for a rejection/poison (an ill-formed program) and `None` for a decline (a
/// construct the compiler does not yet realize) — the branch a downstream sink must preserve rather
/// than collapse (`reference-compiler.md` §The Kind Of A "No" Is Fixed Where It Is Produced).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reject {
    /// `Some(code)` = a rejection (ill-formed); `None` = a decline (not yet built).
    pub code: Option<Code>,
    pub message: String,
}

impl Reject {
    /// A coded rejection: the program is ill-formed, and this is why.
    pub fn coded(code: Code, message: impl Into<String>) -> Reject {
        Reject { code: Some(code), message: message.into() }
    }

    /// An uncoded decline: a construct the compiler does not yet realize. NOT a statement that the
    /// program is wrong — it is the compiler declining to compile it, the safe outcome
    /// (`reference-compiler.md` §Outcomes Are Ordered By Safety).
    pub fn decline(message: impl Into<String>) -> Reject {
        Reject { code: None, message: message.into() }
    }

    /// Whether this "no" is a decline (uncoded) rather than a coded rejection.
    pub fn is_decline(&self) -> bool {
        self.code.is_none()
    }
}
