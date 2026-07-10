//! The solved-type universe — what inference determines and every pass below reads.
//!
//! A node's solved type is materialized once by inference into the type column and read downstream;
//! no later pass re-derives it (`reference-compiler.md` §Types Are Solved Once And Read Downstream).
//! A pass that must choose a value's *machine* representation reads it off this type
//! (`reference-compiler.md` §The Machine Representation Is A Read-Off Of The Solved Type).
//!
//! This type is **target-neutral**: it sits above the backend seam and carries NO wasm valtype byte,
//! no component-model encoding — those are a target's concern, computed by that target's backend
//! (`backends-and-targets.md` §The Boundary Layout Is Computed Once, Target-Neutrally, And Reused).
//! The wasm backend maps a `Ty` to its own valtypes (see `backend::wasm`); a second backend maps the
//! same `Ty` to its target's representation. What lives here is only the language-level type and the
//! names the value renderer supplies (`reference-compiler.md` §Rendering Walks A Static Shape And
//! Supplies The Names) — a language fact, not a target one.
//!
//! Stage 0's universe is the three scalar types the thin slice needs. Later stages widen this sum
//! (parametric types, sums, collections) — it is a closed, exhaustively-matched set, so a new type
//! is a new variant and every pass that reads a type is forced to say what it does with it.
//!
//! **An integer type carries its width and signedness, not a fixed name.** `Int64` is not a type
//! unto itself — it is the signed, 64-bit instance of the one integer type `(Int width)`, whose
//! signedness and width are data (`prelude-and-resolution.md` §A Numeric Width Is A Type Record, Its
//! Machine Operation Read From Its Meta). Carrying the parameter from genesis is what keeps widths
//! from being a retrofit (`build-order.md` §Stage 7): Stage 0 only ever *produces* the signed-64
//! instance, but the representation is already the general one, so a later width is a value the
//! compiler computes rather than a new `Ty` variant.

/// The default bit width an integer literal grounds to when nothing fixes it — the width the backend
/// picks for an unresolved literal (`Int64`).
pub const DEFAULT_INT_WIDTH: u32 = 64;

/// An integer type: a signedness and a bit width that MAY be deferred. A bare integer literal starts
/// with `width: None` — it is polymorphic in its width until a constraint fixes it (numeric-literal
/// defaulting); inference resolves it to a `Some`, or, if it is still `None` when the program is
/// lowered, the backend grounds it to [`DEFAULT_INT_WIDTH`]. `IntTy { signed: true, width: Some(64) }`
/// is `Int64`. The full width semantics — unify only at equal width and signedness, no implicit
/// promotion, per-width overflow — arrive in a later stage; Stage 0 carries the parameter (and the
/// deferral) without exercising the constraints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IntTy {
    pub signed: bool,
    /// The bit width, or `None` if not yet determined — deferred until inference or the backend fixes
    /// it.
    pub width: Option<u32>,
}

impl IntTy {
    /// A deferred signed integer — the type a bare integer literal takes before any constraint or
    /// defaulting fixes its width.
    pub fn deferred() -> IntTy {
        IntTy { signed: true, width: None }
    }

    /// The signed 64-bit integer (`Int64`) — the concrete type an unresolved width grounds to.
    pub fn i64() -> IntTy {
        IntTy { signed: true, width: Some(DEFAULT_INT_WIDTH) }
    }

    /// The concrete width this integer takes at the machine boundary: its resolved width, or the
    /// default if still deferred. The backend reads THIS to pick a representation, so a literal whose
    /// width inference never constrained still lowers to a definite width.
    pub fn ground_width(self) -> u32 {
        self.width.unwrap_or(DEFAULT_INT_WIDTH)
    }
}

/// A solved type. Frozen for Stage 0/1 at the scalars the slice exercises, records, and `Any`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ty {
    /// An integer of a given signedness and a possibly-deferred width.
    Int(IntTy),
    /// The boolean.
    Bool,
    /// The unit value: no information, no runtime slot.
    Unit,
    /// A record: a fixed SET of named fields, each with its own type. Held as a canonically-ordered
    /// `BTreeMap` so two records of the same field-set are the SAME type regardless of the order the
    /// fields were written (`core-semantics.md` §A Record Has A Fixed Set Of Named Fields), and a
    /// field's type is looked up by name in O(log n).
    Record(std::collections::BTreeMap<crate::resolved::Symbol, Ty>),
    /// The type of a node the compiler could not type — a poison's type. It is COMPATIBLE with every
    /// type, so a "no" never induces a spurious mismatch upward (the poison itself is the reported
    /// fault, not a type error it would otherwise cascade). A top type for Stage 0's purposes.
    Any,
}

impl Ty {
    /// A fresh integer-literal type: signed, width deferred. Inference or the backend fixes the width.
    pub fn int() -> Ty {
        Ty::Int(IntTy::deferred())
    }

    /// The signed 64-bit integer type (`Int64`) — an integer literal grounded to its default width.
    pub fn int64() -> Ty {
        Ty::Int(IntTy::i64())
    }

    /// Whether two types agree for Stage-0 checking. `Any` agrees with anything (a poison never
    /// disagrees); two integers agree if their signedness matches and their widths are compatible (a
    /// deferred width is compatible with any width — it has not been fixed yet). Full unification
    /// arrives with real inference; this is the Stage-0 compatibility relation.
    pub fn agrees_with(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Any, _) | (_, Ty::Any) => true,
            (Ty::Int(a), Ty::Int(b)) => {
                a.signed == b.signed
                    && match (a.width, b.width) {
                        (Some(wa), Some(wb)) => wa == wb,
                        _ => true, // a deferred width has not been fixed, so it is compatible.
                    }
            }
            (Ty::Bool, Ty::Bool) => true,
            (Ty::Unit, Ty::Unit) => true,
            // Two records agree iff they have the same field-name set and each field's types agree.
            (Ty::Record(a), Ty::Record(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, ta)| match b.get(k) {
                        Some(tb) => ta.agrees_with(tb),
                        None => false,
                    })
            }
            _ => false,
        }
    }

    /// The "more defined" of two agreeing types — the join used to type an `if` from its branches:
    /// `Any` yields the other; a deferred-width int yields the branch that fixed its width. This is
    /// how the deferred width flows from a constrained branch to the whole conditional.
    pub fn join(&self, other: &Ty) -> Ty {
        match (self, other) {
            (Ty::Any, t) | (t, Ty::Any) => t.clone(),
            (Ty::Int(a), Ty::Int(b)) => {
                let width = a.width.or(b.width); // prefer whichever branch fixed the width.
                Ty::Int(IntTy { signed: a.signed, width })
            }
            // Two agreeing records join field-wise (a deferred width in one branch's field is fixed by
            // the other). If they disagree, keep `self` — the branches-agree check reports the fault.
            (Ty::Record(a), Ty::Record(b)) if self.agrees_with(other) => {
                let joined = a
                    .iter()
                    .map(|(k, ta)| {
                        let t = b.get(k).map(|tb| ta.join(tb)).unwrap_or_else(|| ta.clone());
                        (k.clone(), t)
                    })
                    .collect();
                Ty::Record(joined)
            }
            _ => self.clone(),
        }
    }

    /// The type's name as it appears in a rendered value's annotation (e.g. the corpus `(: 42
    /// Int64)`). Supplied by the value renderer, which walks the static type; the runtime holds no
    /// such name. An integer's name is composed from its signedness and its GROUND width — a deferred
    /// width renders as its default — so an observed value's type is always concrete (`Int64`,
    /// `UInt32`, …). A language-level fact, target-neutral.
    pub fn render_name(&self) -> String {
        match self {
            Ty::Int(it) => {
                let stem = if it.signed { "Int" } else { "UInt" };
                format!("{stem}{}", it.ground_width())
            }
            Ty::Bool => "Bool".to_string(),
            Ty::Unit => "Unit".to_string(),
            // A record renders as `(record (name Type) …)` in canonical (sorted) field order — the
            // shape the renderer walks. The runtime holds no field names; this type does.
            Ty::Record(fields) => {
                let mut s = String::from("(record");
                for (k, t) in fields {
                    s.push_str(&format!(" ({} {})", k.name, t.render_name()));
                }
                s.push(')');
                s
            }
            Ty::Any => "Any".to_string(),
        }
    }
}
