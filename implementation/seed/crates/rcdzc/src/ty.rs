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

    /// Whether this integer's width was FIXED by inference (an annotation `(Int 8)` / an operator
    /// signature), rather than DEFAULTED (a bare literal whose width nothing constrained, so it grounds
    /// to the default `Int64`). Distinguishes a literal that overflows a CHOSEN width (`(: 256 (Int 8))`
    /// — an out-of-range error, CDZ0302) from one that overflows the DEFAULT `Int64` with no width in
    /// sight (`9223372036854775808` — a malformed literal, CDZ0201): only a defaulted width has "no
    /// annotation to blame".
    pub fn width_is_fixed(self) -> bool {
        matches!(self.width, Width::Fixed(_))
    }
}

/// The default float width a float literal grounds to when nothing fixes it (`Float64` — binary64).
pub const DEFAULT_FLOAT_WIDTH: u32 = 64;

/// The set of IEEE-754 binary float widths the numeric model admits — the widths a conforming wasm
/// runtime provides (`f32`/`f64`), pinned at `options/numeric-model/`. A `(Float N)` for any other `N`
/// fails the width constraint (CDZ0302), exactly as an out-of-range integer width does. This is
/// set-MEMBERSHIP, not a range (unlike integers, where every width in `1..=64` is a type): a float
/// width is an IEEE format, not an arbitrary bit count. The admitted-set check lives at the `Float`
/// constructor (`prelude::build_float_ty`), exactly as the integer `1..=64` check lives at `Int`.
pub const ADMITTED_FLOAT_WIDTHS: [u32; 2] = [32, 64];

/// A floating-point type: a [`Width`] (there is no signedness axis — a float is inherently signed, so
/// this is [`IntTy`] minus its `Sign`). `FloatTy { width: Fixed(64) }` is `Float64` (binary64), the
/// type a bare float literal grounds to; `Fixed(32)` is `Float32`. The width is polymorphic exactly as
/// an integer's is: `Deferred` a bare literal, `Var` an operator generic over the float width (`+. :
/// (Float a) → (Float a) → (Float a)`), `Fixed` a concrete width. Crucially it REUSES the integer
/// [`Width`] machinery (`unify_width`/`apply_width`/freshen) — parametric "just like ints" — because a
/// width variable is a width variable regardless of whether it ends up bound in a float or integer
/// context, and a float only ever unifies with a float so the two never cross. Only the admitted SET
/// differs ({32,64} vs 1..=64), enforced at the constructor, not here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FloatTy {
    pub width: Width,
}

impl FloatTy {
    /// A deferred float — the type a bare float literal takes before any constraint or defaulting fixes
    /// its width. An annotation (or a float operator's signature) can ground it.
    pub fn deferred() -> FloatTy {
        FloatTy {
            width: Width::Deferred,
        }
    }

    /// The 64-bit float (`Float64`, binary64) — the concrete type an unresolved float width grounds to.
    pub fn f64() -> FloatTy {
        FloatTy {
            width: Width::Fixed(DEFAULT_FLOAT_WIDTH),
        }
    }

    /// A concrete-width float — the ordinary constructor for a fixed float type (`(Float 32)`/`(Float
    /// 64)`).
    pub fn fixed(width: u32) -> FloatTy {
        FloatTy {
            width: Width::Fixed(width),
        }
    }

    /// The concrete width this float takes at the machine boundary: its fixed width, or the default
    /// (`Float64`) if still deferred or an unresolved variable. The backend/renderer reads THIS.
    pub fn ground_width(self) -> u32 {
        match self.width {
            Width::Fixed(w) => w,
            Width::Deferred | Width::Var(_) => DEFAULT_FLOAT_WIDTH,
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
    /// A LIST: a homogeneous, variable-length sequence of one ELEMENT type (`collections-and-text.md` §A
    /// List Is A Homogeneous Sequence). Unlike a tuple (fixed arity, per-position type), a list's length
    /// is a runtime property and every element shares ONE type — so `(list 1 true)` (mixed) is ill-typed
    /// (the elements do not unify) and `(list)` is a list of a deferred element type. `List Int64` and
    /// `List Bool` are distinct; the element type is held behind a `Box` (one type, unlike the tuple's
    /// slice). Backed at run time by the persistent `vec-*` RRB heap ops.
    List(Box<Ty>),
    /// A MAP: a persistent association of KEYS of one type with VALUES of one type
    /// (`collections-and-text.md` §A Map Associates Keys With Values). PARAMETRIC in two types — the
    /// key type and the value type — held as two `Box`es (unlike `List`'s single element type). The
    /// map's KEY SET is runtime data, NOT part of its type: two maps with different keys but the same
    /// `(key, value)` types are the SAME type `Map<K,V>` (the crucial contrast with a `Record`, whose
    /// field-name SET IS its shape). So comparing two maps with different keys is well-typed (yields
    /// `false`), and a `List (Map K V)` whose elements have different key sets is homogeneous. `Map
    /// Int64 Int64` and `Map Int64 Bool` are distinct. Backed at run time by the persistent CHAMP
    /// `map-*` heap ops; keys are compared by VALUE under structural equality, and its canonical form
    /// renders entries in sorted key order.
    Map(Box<Ty>, Box<Ty>),
    /// A BYTES sequence: a homogeneous, variable-length sequence of BYTES (`collections-and-text.md` §A
    /// Byte Sequence Is A Sequence Of Bytes). NOT parametric — a byte is a byte, so unlike `List` it
    /// carries no element type (a LEAF in the type universe). Distinct from `List Int64`: a `Bytes` packs
    /// its bytes and renders `b"…"` (the byte-string form), whereas a list renders `(list …)`; the
    /// compiler keeps them separate types even though a byte is an integer. Backed at run time by the
    /// persistent rope `bytes-*` heap ops; its observable form is the byte-string literal.
    Bytes,
    /// A STRING: an immutable sequence of Unicode text (`collections-and-text.md` §A String Is A
    /// Sequence Of Unicode Scalar Values). One monomorphic type (no element parameter, unlike `List`) —
    /// every string is `String`. Backed at run time by the same UTF-8 byte-rope the value heap uses for
    /// `Bytes`; only the STATIC type differs (a `String` renders `"…"` with the closed escape set, a
    /// `Bytes` renders `\xNN`). This increment realizes the CONSTANT string (a literal folds + equality);
    /// runtime string ops (`concat`/`len`/`at`) + string escape arrive later.
    String,
    /// A CHAR: a single Unicode scalar value (`collections-and-text.md` §A Char Is A Single Unicode
    /// Scalar Value) — the element type of a string's scalar sequence. One monomorphic LEAF type (no
    /// parameter). Its value is its scalar, so equality is scalar equality and its order is the numeric
    /// order of its scalar value. A `#\a` literal is a `Char` constant; `Char.to-int`/`from-int` convert
    /// to/from `Int64` totally. This increment realizes the CONSTANT char (literal + equality/ordering).
    Char,
    /// A FLOATING-POINT number, indexed by its bit WIDTH (`numeric-model.md` §A Floating-Point Type Is
    /// Indexed By A Compile-Time Width) — the float analogue of [`Ty::Int`], carrying a [`FloatTy`]
    /// (a possibly-deferred [`FloatWidth`]) rather than a fixed name. `Float64` is the signed 64-bit
    /// (binary64) instance a bare float literal grounds to; `Float32` is `(Float 32)`. Two float types
    /// of different width are DISTINCT (no silent promotion), and an integer never unifies with a float
    /// (`(+ 2 2.0)` → CDZ0301). Backed at the boundary by the component-model `f64`/`f32`.
    Float(FloatTy),
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
    ///
    /// `args` are the sum's TYPE ARGUMENTS — the concrete types its (implicit) type parameters are
    /// instantiated at (`type-system.md §Generics Are Type-Valued Parameters`). A MONOMORPHIC sum
    /// (`(type Sign Neg Zero Pos)` — no free type variable in any payload) has `args: []`. A GENERIC sum
    /// `(type Option (Some a) None)` at a concrete instantiation carries them: `Option Int64` is `Sum {
    /// decl: <Option>, name: "Option", args: [Int64] }`. Two sums are the SAME type iff their `decl` AND
    /// their `args` agree, so `Option Int64 ≠ Option Bool` (same declaration, different instantiation) —
    /// the payload-type discrimination `type-system.md §the head Option agrees but the payload does not`
    /// requires. Args are positional, in the parameters' first-appearance order.
    Sum {
        decl: crate::ast::StructId,
        name: String,
        args: Vec<Ty>,
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

    /// A fresh float-literal type: width deferred. Inference or the backend fixes the width (`Float64`).
    pub fn float() -> Ty {
        Ty::Float(FloatTy::deferred())
    }

    /// The 64-bit float type (`Float64`) — a float literal grounded to its default width.
    pub fn float64() -> Ty {
        Ty::Float(FloatTy::f64())
    }

    /// Whether the type contains an UNRESOLVED type variable ([`Ty::Var`]) — a payload/element/parameter
    /// the inference solve never determined (a bare `(None)` is `Option ?0`; an empty `(list)` is `List
    /// ?0`). Such a type has no defined serialization, so a value of it reaching the HOST BOUNDARY is a
    /// rejection that should name the AMBIGUITY (annotate the type) rather than a boundary-SHAPE error.
    /// (Deferred integer width/sign are NOT free variables — they ground to a default; only a `Ty::Var`
    /// is genuinely undetermined.)
    pub fn has_free_var(&self) -> bool {
        match self {
            Ty::Var(_) => true,
            Ty::Fn(p, r) => p.has_free_var() || r.has_free_var(),
            Ty::Tuple(elems) => elems.iter().any(|t| t.has_free_var()),
            Ty::List(elem) => elem.has_free_var(),
            Ty::Map(k, v) => k.has_free_var() || v.has_free_var(),
            Ty::Record(fields) => fields.values().any(|t| t.has_free_var()),
            Ty::Sum { args, .. } => args.iter().any(|t| t.has_free_var()),
            // Bytes, String, and Char are leaves — no inner type, so no free variable.
            Ty::Int(_)
            | Ty::Bool
            | Ty::Unit
            | Ty::Type
            | Ty::Any
            | Ty::Bytes
            | Ty::String
            | Ty::Char
            | Ty::Float(_) => false,
        }
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
            // Two lists agree iff their ELEMENT types agree — `List Int64` ≠ `List Bool`, and a list of a
            // deferred element type is compatible via the recursive `agrees_with` (the empty-list case).
            (Ty::List(a), Ty::List(b)) => a.agrees_with(b),
            // Two maps agree iff their KEY types agree AND their VALUE types agree — `Map Int64 Int64` ≠
            // `Map Int64 Bool`. A map's KEY SET is NOT compared here (it is runtime data, not shape): two
            // maps with different keys but the same `(key, value)` types are the SAME type and agree, so a
            // comparison between them is well-typed (yielding `false`) and a list of them is homogeneous —
            // the contrast with `Record` above (whose field-name set IS its shape). Deferred key/value
            // types (an empty map) are compatible via the recursive `agrees_with`.
            (Ty::Map(ka, va), Ty::Map(kb, vb)) => ka.agrees_with(kb) && va.agrees_with(vb),
            // Bytes is a leaf — a bytes agrees only with another bytes (no element type to compare).
            (Ty::Bytes, Ty::Bytes) => true,
            // Two sums agree iff their DECLARATIONS match AND their type ARGS agree pairwise — a sum's
            // identity is its declaration (`type-system.md §160`: distinct FQNs ⇒ distinct types, so
            // module A's `Foo` ≠ module B's `Foo`) TOGETHER WITH its instantiation (`Option Int64 ≠
            // Option Bool` — same declaration, different type argument; §the head agrees but the payload
            // does not). A monomorphic sum has empty `args` on both sides, so this reduces to the decl
            // check. A deferred arg (an as-yet-unsolved payload) is compatible via the recursive
            // `agrees_with`.
            (
                Ty::Sum {
                    decl: a, args: aa, ..
                },
                Ty::Sum {
                    decl: b, args: ab, ..
                },
            ) => a == b && aa.len() == ab.len() && aa.iter().zip(ab).all(|(x, y)| x.agrees_with(y)),
            // `String` is monomorphic — the one string type agrees only with itself.
            (Ty::String, Ty::String) => true,
            // `Char` is monomorphic — the one char type agrees only with itself.
            (Ty::Char, Ty::Char) => true,
            // Two floats agree iff their WIDTHS agree — `Float32` ≠ `Float64` (no silent promotion), a
            // deferred/variable width is compatible (not yet fixed). A float never agrees with an integer
            // (numeric-model.md §Numeric Types Do Not Silently Promote). Mirrors the `Ty::Int` width check.
            (Ty::Float(a), Ty::Float(b)) => match (a.width, b.width) {
                (Width::Fixed(wa), Width::Fixed(wb)) => wa == wb,
                _ => true,
            },
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
            // Two agreeing SUMS join their type ARGS pairwise, so a payload resolved in EITHER branch
            // determines the joined arg — `(Option ?0)` ⊔ `(Option Int64)` = `(Option Int64)`. Without
            // this, an `if` whose branches are the SAME sum built two ways (a nullary variant `(None)` :
            // `Option ?0` in one branch, a payload variant `(Some n)` : `Option Int64` in the other) took
            // the `_ => self.clone()` fallthrough and kept the FIRST branch's type — so a leading `None`
            // pinned the result to `(Option ?0)` with `?0` free, and the value-heap layout then declined
            // "projecting a tuple element of type ?0 needs the value heap". Joining the args makes the
            // conditional's type ORDER-INDEPENDENT: the payload-carrying branch fixes the parameter in
            // either position. (`agrees_with` guarantees same `decl` + arg arity.)
            (
                Ty::Sum {
                    decl,
                    name,
                    args: aa,
                },
                Ty::Sum { args: ab, .. },
            ) if self.agrees_with(other) => Ty::Sum {
                decl: *decl,
                name: name.clone(),
                args: aa.iter().zip(ab.iter()).map(|(x, y)| x.join(y)).collect(),
            },
            // Two agreeing lists join their element type — a deferred element (`List ?0`, the empty list)
            // is fixed by the other branch's `List Int64`, the list analogue of the sum-arg join above.
            (Ty::List(a), Ty::List(b)) if self.agrees_with(other) => Ty::List(Box::new(a.join(b))),
            // Two agreeing maps join their key type AND value type — a deferred key/value (`Map ?0 ?1`,
            // the empty map) is fixed by the other branch's `Map Int64 Int64`, the map analogue of the
            // list join above.
            (Ty::Map(ka, va), Ty::Map(kb, vb)) if self.agrees_with(other) => {
                Ty::Map(Box::new(ka.join(kb)), Box::new(va.join(vb)))
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
            // A string renders as `String` — one monomorphic type, no parameters.
            Ty::String => "String".to_string(),
            // A char renders as `Char` — one monomorphic type (its VALUES render `#\c`).
            Ty::Char => "Char".to_string(),
            // A float renders as its aliased width name — `Float32`/`Float64`. Every admitted float
            // width ({32, 64}) has an alias, so an observed float type is always a concrete `FloatN`
            // (an unresolved width grounds to `Float64`), mirroring the integer `IntN`/`UIntN` render.
            Ty::Float(ft) => format!("Float{}", ft.ground_width()),
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
            // A list renders as `(List Elem)` — the element type is its only type parameter.
            Ty::List(elem) => format!("(List {})", elem.render_name()),
            // A map renders as `(Map Key Value)` — its two type parameters, key first (the corpus `(:
            // (map (1 10) (2 20)) (Map Int64 Int64))` form). The key SET is runtime data, not the type.
            Ty::Map(k, v) => format!("(Map {} {})", k.render_name(), v.render_name()),
            // Bytes renders as the bare type name `Bytes` (its VALUES render `b"…"`, but the type
            // annotation is the name — the corpus `(: b"…" Bytes)` form).
            Ty::Bytes => "Bytes".to_string(),
            // A sum renders as its NOMINAL NAME, applied to its type ARGS when generic: a monomorphic
            // sum (`args: []`) is the bare name (`(: (Neg unit) Sign)`); a generic sum is `(Name arg…)`
            // — `(: (Some 5) (Option Int64))` (`type-system.md §158`; the corpus form). The variant set
            // is not part of the rendered type (a match reads it from `db.type_decls`).
            Ty::Sum { name, args, .. } => {
                if args.is_empty() {
                    name.clone()
                } else {
                    let mut s = format!("({name}");
                    for a in args {
                        s.push(' ');
                        s.push_str(&a.render_name());
                    }
                    s.push(')');
                    s
                }
            }
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
