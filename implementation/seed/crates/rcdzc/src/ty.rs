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
//! The universe is a CLOSED, exhaustively-matched set of variants: integers (width- and
//! sign-indexed), Bool, Unit, records, tuples, sums, function types, the type-value type, a
//! unification variable, and `Any`. Because it is closed, adding a type is adding a variant, which
//! forces every pass that reads a type to say what it does with it — the property that keeps a new
//! type honest as the language grows.
//!
//! **An integer type carries its width and signedness, not a fixed name.** `Int64` is not a type
//! unto itself — it is the signed, 64-bit instance of the one integer type `(Int width)`, whose
//! signedness and width are data:
//!
//= spec/capabilities/numeric-model.md#an-integer-type-is-indexed-by-a-compile-time-width
//# An integer type MUST be identified by a signedness and a bit width, so that two integer types of different width or signedness are distinct types that do not silently convert to one another.
//!
//! So a width is a value the compiler computes and unifies, never a new `Ty` variant per width — the
//! single [`IntTy`] carries both axes, and every named width (`Int8`, `UInt32`, …) is one instance.

/// The default bit width an integer literal grounds to when nothing fixes it — the width the backend
/// picks for an unresolved literal (`Int64`).
pub const DEFAULT_INT_WIDTH: u32 = 64;

/// The width of an integer type — the parameter that makes an intrinsic generic over the integer
/// type. Three states: a `Fixed` concrete width (`64` = `Int64`), a `Deferred` width a bare literal
/// carries until a constraint fixes it (numeric-literal polymorphism, grounds to the default), or a
/// `Var` unification variable an intrinsic's signature introduces so `+ : (Int w) → (Int w) → (Int
/// w)` unifies its operands' widths rather than hard-coding one (`build-order.md` §Stage 2 — generic
/// over the integer type). Inference resolves a `Var`/`Deferred` to a `Fixed`; the backend grounds a
/// still-unresolved width to the default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Width {
    Fixed(u32),
    Deferred,
    Var(u32),
}

/// The signedness of an integer type — the SAME three-state shape as [`Width`], because a bare integer
/// literal is polymorphic in its sign exactly as it is in its width. `Fixed(true)` = signed,
/// `Fixed(false)` = unsigned; `Deferred` = a bare literal's sign before anything constrains it (grounds
/// to signed); `Var` = a unification variable an intrinsic/annotation introduces. Making sign a
/// variable (not a baked `bool`) is what lets `(: 200 UInt8)` GROUND a literal to unsigned through
/// ordinary unification — "Annotations Constrain, Never Contradict" — rather than clashing with a
/// signed default. Inference resolves a `Var`/`Deferred` to a `Fixed`; the backend grounds a
/// still-unresolved sign to signed (the default literal type).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sign {
    Fixed(bool),
    Deferred,
    Var(u32),
}

/// An integer type: a [`Sign`] and a [`Width`]. `IntTy { sign: Fixed(true), width: Fixed(64) }` is
/// `Int64`. Both axes unify (unify only at equal sign and width, no implicit promotion) and both can be
/// deferred (a bare literal) or a variable (an operator generic over the integer type) — so a width
/// AND a signedness are data the compiler unifies, never a hard-coded case.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IntTy {
    pub sign: Sign,
    pub width: Width,
}

impl IntTy {
    /// A deferred integer — the type a bare integer literal takes before any constraint or defaulting
    /// fixes its sign and width. BOTH axes are deferred, so an annotation (or an operator's signature)
    /// can ground either.
    pub fn deferred() -> IntTy {
        IntTy {
            sign: Sign::Deferred,
            width: Width::Deferred,
        }
    }

    /// The signed 64-bit integer (`Int64`) — the concrete type an unresolved sign+width grounds to.
    pub fn i64() -> IntTy {
        IntTy {
            sign: Sign::Fixed(true),
            width: Width::Fixed(DEFAULT_INT_WIDTH),
        }
    }

    /// A concrete `(signed, width)` integer — the ordinary constructor for a fixed integer type.
    pub fn fixed(signed: bool, width: u32) -> IntTy {
        IntTy {
            sign: Sign::Fixed(signed),
            width: Width::Fixed(width),
        }
    }

    /// The concrete SIGNEDNESS this integer takes at the machine boundary — its fixed sign, or signed
    /// (the default) if still deferred or an unresolved variable. The backend/renderer reads THIS.
    pub fn ground_signed(self) -> bool {
        match self.sign {
            Sign::Fixed(s) => s,
            Sign::Deferred | Sign::Var(_) => true,
        }
    }

    /// The concrete width this integer takes at the machine boundary: its fixed width, or the default
    /// if still deferred or an unresolved variable. The backend reads THIS to pick a representation,
    /// so a literal whose width inference never constrained still lowers to a definite width.
    pub fn ground_width(self) -> u32 {
        match self.width {
            Width::Fixed(w) => w,
            Width::Deferred | Width::Var(_) => DEFAULT_INT_WIDTH,
        }
    }
}

/// A solved type — the closed variant set inference determines and every pass below reads.
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
    /// field's type is looked up by name in O(log n). Wrapped in an `Arc` so CLONING a `Ty::Record`
    /// (which `type_of` does on every memo read) is a refcount bump, not a deep map copy — a wide
    /// record read field-by-field was O(N²) in map clone. Faithful to the Cadenza port target, which
    /// is ref-counted throughout (a shared immutable value = one refcounted allocation).
    Record(std::sync::Arc<std::collections::BTreeMap<crate::resolved::Symbol, Ty>>),
    /// A tuple: a fixed-ARITY POSITIONAL product, each position with its own type. The arity AND the
    /// per-position types ARE the type — a tuple of a different arity, or with a differently-typed
    /// position, is a DIFFERENT type (`type-system.md` §The Structural Types Are Record, Tuple, And
    /// Sum). Distinct from `Record` (a set of NAMED fields): a tuple is projected by position. Held
    /// behind an `Arc<[Ty]>` (an immutable shared slice) so cloning a `Ty::Tuple` is a refcount bump,
    /// not a deep element copy: an N-element tuple type read once per projection (N projections off one
    /// shared tuple) was O(N²) in `Vec<Ty>` clone. Indexing/iterating are unchanged (it derefs to a
    /// slice); only construction differs (`.collect::<Vec<_>>().into()` or `vec![…].into()`).
    Tuple(std::sync::Arc<[Ty]>),
    /// A SUM: a value of one of a fixed set of named variants (`type-system.md` §The Structural Types
    /// Are Record, Tuple, And Sum — "a sum of named variants"). Declared by `(type NAME variant…)`,
    /// which tags it NOMINAL (`§Nominal Is An Orthogonal Modifier Over Any Structural Type`), so its
    /// identity is its fully-qualified NAME — "the module path in which it is declared together with
    /// its declared name" (`§A Nominal Type's Identity Is Its Fully-Qualified Name`) — NOT its shape.
    ///
    /// We realize that FQN identity as the DECLARATION's arena occurrence `decl` (the `TypeDecl.occ`),
    /// not the local name string. Two `(type Foo …)` declared in different modules are DISTINCT AST
    /// nodes, so they carry distinct `decl` ids and are distinct types (`§160` — distinct whenever
    /// their FQNs differ, even with identical structure and identical local name `Foo`). This is also
    /// IMPORT-SAFE by construction: package linking splices each file's arena into one `Db`, so every
    /// imported declaration keeps its own `StructId` — the "A's Foo ≠ B's Foo" property survives with
    /// no module-path plumbing. (It is the columns-model realization of the seed's `Arc::ptr_eq`
    /// identity — physical declaration identity, expressed as the node id everything is already keyed
    /// by.) `name` is the declared LOCAL name, carried for rendering (`(: (Some 5) Option)`) only;
    /// `decl` alone decides equality (a `decl` determines its `name`).
    ///
    /// The variant SET with its payload types — the shape a match's exhaustiveness (`§190`) and a
    /// constructor's payload check read — lives in `db.type_decls` (found by `decl`), NOT here, which
    /// also keeps `Ty` FINITE for a recursive sum (`(type Expr (Lit Int64) (Neg Expr))` — an eager
    /// payload map mentioning the sum itself would be an infinite type). The runtime representation is
    /// the heap sum handle (`sum-new`/`sum-disc`/`sum-payload`); the nominal tag is compile-time only
    /// (`§156` — it "adds nothing to the value's runtime representation").
    Sum {
        decl: crate::ast::StructId,
        name: String,
    },
    /// A function type `param → result`, curried (a multi-parameter operation is nested `Fn`s). What
    /// an operator's (and later a function's) `Meta.t` denotes; an application unifies the argument
    /// against `param` and takes `result`.
    Fn(Box<Ty>, Box<Ty>),
    /// The type of a type VALUE — the type of `Bool`, of `(Int 64)`, of the result of `(-> A B)`.
    /// Because a type is a first-class value, it has a type, and that type is `Type`. It is
    /// compile-time-only (a value of type `Type` is erased before the runtime boundary, like any
    /// type-value).
    Type,
    /// A unification variable — an as-yet-unsolved type inference introduces (e.g. a fresh operand
    /// type before it is constrained). Resolved to a concrete type by the Hindley-Milner solve in
    /// [`crate::unify`]; a variable that survives to the boundary is an undetermined type (a rejection,
    /// not a default). An operator's scheme carries one variable per generic axis so an application
    /// unifies its operands rather than hard-coding a type.
    Var(u32),
    /// The type of a node the compiler could not type — a poison's type. It is COMPATIBLE with every
    /// type, so a "no" never induces a spurious mismatch upward (the poison itself is the reported
    /// fault, not a type error it would otherwise cascade). Behaves as a top type in `agrees_with`
    /// and `unify`.
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

    /// Whether two types are COMPATIBLE — a structural yes/no relation, distinct from [`crate::unify`]
    /// which solves variables into a substitution. This is the cheap check the passes that only need a
    /// verdict use (an annotation's declared vs inferred type, an `if`'s two branches, a match
    /// pattern's type vs its scrutinee); the solving inference uses `unify`. `Any` agrees with anything
    /// (a poison never disagrees); two integers agree if their signedness matches and their widths are
    /// compatible (a deferred/variable width or sign is compatible — it has not been fixed yet).
    pub fn agrees_with(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Any, _) | (_, Ty::Any) => true,
            // A variable is not yet solved, so it is compatible with anything (unification, not this
            // relation, is what actually resolves it).
            (Ty::Var(_), _) | (_, Ty::Var(_)) => true,
            (Ty::Int(a), Ty::Int(b)) => {
                // A fixed sign must match; a deferred/variable sign is compatible (not yet fixed).
                let sign_ok = match (a.sign, b.sign) {
                    (Sign::Fixed(sa), Sign::Fixed(sb)) => sa == sb,
                    _ => true,
                };
                let width_ok = match (a.width, b.width) {
                    (Width::Fixed(wa), Width::Fixed(wb)) => wa == wb,
                    // a deferred or variable width has not been fixed, so it is compatible.
                    _ => true,
                };
                sign_ok && width_ok
            }
            (Ty::Bool, Ty::Bool) => true,
            (Ty::Unit, Ty::Unit) => true,
            // Two function types agree iff their parameters and results agree.
            (Ty::Fn(pa, ra), Ty::Fn(pb, rb)) => pa.agrees_with(pb) && ra.agrees_with(rb),
            // Two records agree iff they have the same field-name set and each field's types agree.
            (Ty::Record(a), Ty::Record(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, ta)| match b.get(k) {
                        Some(tb) => ta.agrees_with(tb),
                        None => false,
                    })
            }
            // Two tuples agree iff they have the SAME ARITY and each position's types agree — a
            // different arity is a different type (the corpus `if`-branch cases: a 2-tuple and a 3-tuple
            // do not agree, nor a `(Tuple Int Int)` with a `(Tuple Int Bool)`).
            (Ty::Tuple(a), Ty::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(ta, tb)| ta.agrees_with(tb))
            }
            // Two sums agree iff their DECLARATIONS match — a sum's identity is its fully-qualified
            // name, realized as the declaration occurrence (`type-system.md §160`: two nominal types
            // are distinct whenever their FQNs differ, even with identical structure AND identical local
            // name). Comparing `decl` (not `name`) is what makes module A's `Foo` and module B's `Foo`
            // distinct. No structural descent (same `decl` ⇒ same declaration ⇒ same shape).
            (Ty::Sum { decl: a, .. }, Ty::Sum { decl: b, .. }) => a == b,
            _ => false,
        }
    }

    /// The "more defined" of two agreeing types — the join used to type an `if` from its branches:
    /// `Any` yields the other; a deferred-width int yields the branch that fixed its width. This is
    /// how the deferred width flows from a constrained branch to the whole conditional.
    pub fn join(&self, other: &Ty) -> Ty {
        match (self, other) {
            (Ty::Any, t) | (t, Ty::Any) => t.clone(),
            // A variable yields the other side (the more-defined type).
            (Ty::Var(_), t) | (t, Ty::Var(_)) => t.clone(),
            (Ty::Int(a), Ty::Int(b)) => {
                // Prefer whichever side fixed each axis (Fixed > Deferred/Var).
                let width = match (a.width, b.width) {
                    (Width::Fixed(w), _) | (_, Width::Fixed(w)) => Width::Fixed(w),
                    (Width::Deferred, _) | (_, Width::Deferred) => Width::Deferred,
                    _ => a.width,
                };
                let sign = match (a.sign, b.sign) {
                    (Sign::Fixed(s), _) | (_, Sign::Fixed(s)) => Sign::Fixed(s),
                    (Sign::Deferred, _) | (_, Sign::Deferred) => Sign::Deferred,
                    _ => a.sign,
                };
                Ty::Int(IntTy { sign, width })
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
                Ty::Record(std::sync::Arc::new(joined))
            }
            // Two agreeing tuples join position-wise (same arity, guaranteed by `agrees_with`).
            (Ty::Tuple(a), Ty::Tuple(b)) if self.agrees_with(other) => {
                Ty::Tuple(a.iter().zip(b.iter()).map(|(ta, tb)| ta.join(tb)).collect())
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
                let stem = if it.ground_signed() { "Int" } else { "UInt" };
                format!("{stem}{}", it.ground_width())
            }
            Ty::Bool => "Bool".to_string(),
            Ty::Unit => "Unit".to_string(),
            // A record renders as `(record (name Type) …)` in canonical (sorted) field order — the
            // shape the renderer walks. The runtime holds no field names; this type does.
            Ty::Record(fields) => {
                let mut s = String::from("(record");
                for (k, t) in fields.iter() {
                    s.push_str(&format!(" ({} {})", k.name, t.render_name()));
                }
                s.push(')');
                s
            }
            // A tuple renders as `(Tuple T0 T1 …)` in position order — the shape the renderer walks
            // (its arity and element types are its type).
            Ty::Tuple(elems) => {
                let mut s = String::from("(Tuple");
                for t in elems.iter() {
                    s.push(' ');
                    s.push_str(&t.render_name());
                }
                s.push(')');
                s
            }
            // A sum renders as its NOMINAL NAME — its identity is that name (`type-system.md §158`), and
            // the observed value's annotation is the declared type (`(: (Some 5) Option)`). The variant
            // set is not part of the rendered type name (a match/constructor reads it from
            // `db.type_decls`); the name alone identifies the type.
            Ty::Sum { name, .. } => name.clone(),
            Ty::Fn(p, r) => format!("(-> {} {})", p.render_name(), r.render_name()),
            Ty::Type => "Type".to_string(),
            Ty::Var(n) => format!("?{n}"),
            Ty::Any => "Any".to_string(),
        }
    }
}

/// A type SCHEME — a polymorphic type quantified over some type and width variables (`∀ vars. ty`).
/// What an operator's (and later a function's) `Meta.t` denotes. Instantiating a scheme replaces its
/// bound variables with FRESH ones, so each use is independent — the mechanism that makes `+` generic
/// over the integer type and `(id x)` polymorphic. Bound variables are identified by the `Ty::Var` /
/// `Width::Var` numbers appearing in `ty`; `ty_vars`/`width_vars` list which of those are quantified.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Scheme {
    pub ty_vars: Vec<u32>,
    pub width_vars: Vec<u32>,
    pub sign_vars: Vec<u32>,
    pub ty: Ty,
}

impl Scheme {
    /// A monomorphic scheme — a plain type with nothing quantified.
    pub fn mono(ty: Ty) -> Scheme {
        Scheme {
            ty_vars: Vec::new(),
            width_vars: Vec::new(),
            sign_vars: Vec::new(),
            ty,
        }
    }
}
