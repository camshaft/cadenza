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
//! The taxonomy grows one variant per added check; its `str` form is the stable `CDZ####` string a
//! tool branches on.

/// A stable, machine-readable diagnostic code. Its `code()` string is the durable identity a
/// consumer matches on; the enum variant is the compiler-internal handle.
///
//= spec/capabilities/diagnostics.md#every-diagnostic-has-a-stable-code
//# Every diagnostic the compiler emits MUST carry a machine-readable code that is stable across changes to unrelated diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    /// A reference to a name with no binding in scope — the unbound-name rule, unconditional and not
    /// gated on reachability (`core-semantics.md` §Binding Is Lexical).
    Unbound,
    /// A binder position binds the same name more than once — a non-linear binder. A pattern
    /// (`core-semantics.md` §Patterns Compose: "A pattern MUST bind each name at most once … rather than
    /// silently shadowing an earlier binder") and a function's PARAMETER LIST are the same binder-linearity
    /// surface, so `(def (f x x) …)` (or `(tuple a a)` in a pattern) is this error rather than a last-wins
    /// shadow that makes the first binder unreachable.
    NonLinearBinder,
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
    /// A `match` that does not cover its scrutinee — a coverage defect (the pattern engine's
    /// non-exhaustiveness rejection, distinct from a shape defect). For a scalar scrutinee this is a
    /// match with no wildcard tail; for a sum it is a missing variant (a later increment).
    NonExhaustive,
    /// A computation the compiler PROVES would trap (`ConstTrap`'s outcome) was ELIMINATED because its
    /// value is unobserved — an unprojected tuple/record element, an unreferenced `let` binding, an
    /// argument bound to an unused parameter. NOT a rejection: the build succeeds (the dead computation
    /// need not run — `core-semantics.md` §A Trap Occurs Only Where Its Computation Is Observed). This
    /// is the WARNING severity's code, emitted so a program does not silently discard a computation that
    /// could never have produced a value (almost always a defect). The error-severity companion is
    /// `ConstTrap` (CDZ0304), emitted when the same provable trap IS observed.
    DeadTrap,
    /// An effect operation is reached at a point with NEITHER an enclosing handler for its effect NOR an
    /// enclosing host delegation of it — the merged "no home for a reached effect" check
    /// (`capabilities-and-effects.md` §An Ungranted Effect Is A Compile-Time Error). This single code
    /// subsumes both the reached-but-undelegated host operation and the undischarged intra-program effect
    /// (the retired CDZ0402), because host-binding is an entrypoint routing decision, not a
    /// declaration-time property — an effect reached the entrypoint's top with no home.
    EffectNoHome,
    /// A handler arm names an operation the arm's effect does not declare — a closed-set violation
    /// (`capabilities-and-effects.md` §A Handler Arm Names An Operation Its Effect Declares). An effect's
    /// operations are a closed, statically-known set (like a sum's variants), so discharging an operation
    /// that does not exist is ill-formed.
    HandlerUndeclaredOp,
    /// A host delegation names an effect the delegated computation never reaches — latent authority
    /// (`capabilities-and-effects.md` §Host Delegation Is An Entrypoint's Prerogative). The manifest must
    /// be exactly the effects that escape, no more and no fewer, so a granted-but-unexercised capability
    /// is rejected rather than carried.
    LatentAuthority,
}

impl Code {
    /// The stable `CDZ####` string. These are the identities a tool and the corpus branch on, so
    /// they change only by the coordinated act a code taxonomy change is.
    pub fn code(self) -> &'static str {
        match self {
            Code::Unbound => "CDZ0101",
            Code::NonLinearBinder => "CDZ0102",
            Code::Malformed => "CDZ0201",
            Code::TypeMismatch => "CDZ0203",
            Code::NumericMismatch => "CDZ0301",
            Code::IntOutOfRange => "CDZ0302",
            Code::ConstTrap => "CDZ0304",
            Code::DeadTrap => "CDZ0305",
            Code::NonExhaustive => "CDZ0210",
            Code::EffectNoHome => "CDZ0401",
            Code::HandlerUndeclaredOp => "CDZ0403",
            Code::LatentAuthority => "CDZ0404",
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
