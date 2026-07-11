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
    /// A reference to a name with no binding in scope — the unbound-name rule, unconditional and not
    /// gated on reachability (`core-semantics.md` §Binding Is Lexical).
    Unbound,
    /// A malformed or ill-typed construct that the compiler positively proves ill-formed.
    Malformed,
    /// A type mismatch (e.g. an `if` condition that is not a boolean; branches of differing type).
    TypeMismatch,
    /// Two operands of different numeric types with no explicit conversion (no silent promotion).
    NumericMismatch,
    /// An integer literal that does not fit the width its use requires.
    IntOutOfRange,
    /// A constant operation whose defined outcome on its (compile-time-known) operands is a trap —
    /// e.g. a provable overflow. A compile-provable trap fails the build rather than shipping a
    /// component that traps at run time (`reference-compiler.md` §A Compile-Provable Trap Fails The
    /// Build).
    ConstTrap,
}

impl Code {
    /// The stable `CDZ####` string. These are the identities a tool and the corpus branch on, so
    /// they change only by the coordinated act a code taxonomy change is.
    pub fn code(self) -> &'static str {
        match self {
            Code::Unbound => "CDZ0101",
            Code::Malformed => "CDZ0201",
            Code::TypeMismatch => "CDZ0203",
            Code::NumericMismatch => "CDZ0301",
            Code::IntOutOfRange => "CDZ0302",
            Code::ConstTrap => "CDZ0304",
        }
    }
}

/// A produced "no": either a coded rejection or an uncoded decline, each carrying a human message and
/// the AST node it is about. The `code` is `Some` for a rejection/poison (an ill-formed program) and
/// `None` for a decline (a construct the compiler does not yet realize) — the branch a downstream sink
/// must preserve rather than collapse (`reference-compiler.md` §The Kind Of A "No" Is Fixed Where It
/// Is Produced).
///
/// **The origin is a node identity, not a span.** `at` is the `StructId` the fault is about. The
/// compiler holds NO source positions — a `StructId` is a stable node identity, and the FRONT-END
/// (which parsed the text into the binary AST and holds the span table keyed by that same identity)
/// maps the id back to a text region (`query-engine.md` §Provenance Is Recovered By Back-Reference).
/// So the whole compiler — and its eventual Cadenza port — stays span-free: a diagnostic names a node,
/// the application layer resolves it to a region. Program node ids are preserved through decode (the
/// prelude appends its nodes AFTER the program's), so an `at` on a program node IS the id the
/// front-end's span table is keyed by. `None` = an un-anchored "no" (a synthesized node, or a
/// producer that did not stamp one).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reject {
    /// `Some(code)` = a rejection (ill-formed); `None` = a decline (not yet built).
    pub code: Option<Code>,
    pub message: String,
    /// The AST node this "no" is about — the front-end maps it to a text region. `None` if unstamped.
    pub at: Option<crate::ast::StructId>,
}

impl Reject {
    /// A coded rejection: the program is ill-formed, and this is why. Unanchored — a caller with the
    /// faulting node in hand attaches it via [`Reject::at`] (or a collector stamps it with
    /// [`Reject::set_origin_if_absent`]).
    pub fn coded(code: Code, message: impl Into<String>) -> Reject {
        Reject {
            code: Some(code),
            message: message.into(),
            at: None,
        }
    }

    /// An uncoded decline: a construct the compiler does not yet realize. NOT a statement that the
    /// program is wrong — it is the compiler declining to compile it, the safe outcome
    /// (`reference-compiler.md` §Outcomes Are Ordered By Safety).
    pub fn decline(message: impl Into<String>) -> Reject {
        Reject {
            code: None,
            message: message.into(),
            at: None,
        }
    }

    /// Attach (or replace) the node this "no" is about — the fluent form a producer uses when it holds
    /// the precise faulting node: `Reject::coded(..).at(id)`.
    pub fn at(mut self, id: crate::ast::StructId) -> Reject {
        self.at = Some(id);
        self
    }

    /// Stamp the origin node ONLY if none is set yet — so a precise producer's anchor is never
    /// overwritten, while a collector walking node `id` can supply a default origin for a "no" that
    /// was produced without one. The innermost frame that has the faulting node wins.
    pub fn set_origin_if_absent(&mut self, id: crate::ast::StructId) {
        if self.at.is_none() {
            self.at = Some(id);
        }
    }

    /// Whether this "no" is a decline (uncoded) rather than a coded rejection.
    pub fn is_decline(&self) -> bool {
        self.code.is_none()
    }
}
