//! The resolved node form — one entry of the resolved column, keyed by AST `StructId`.
//!
//! This is the tree-above-the-core rung, but it is NOT a separate arena: a node's resolved form is a
//! *column* over the AST's own identity (`query-engine.md` §The Compiler's State Is Columns Indexed
//! By Node Identity). `resolved_of(id)` fills the slot for one node; the node references its children
//! by their AST `StructId`, so descending into a child is the same lazy column read on a different
//! id. The source's nesting is therefore preserved (a child is reached through the parent) without
//! copying the tree into a second arena.
//!
//! Resolving a node is per-node and does NOT recurse: it classifies the AST occurrence at `id` and
//! records what it denotes, leaving the children as ids for a later demand to resolve. A "no" is a
//! value here — an unrecognized or malformed construct is [`Resolved::Poison`] — so the resolved
//! column is total over every node a query reaches.
//!
//! A bare name resolves — by the lexical-scope walk (`resolve::scope`) then the prelude map — to what
//! it denotes: a [`Resolved::Ref`] to the value occurrence it is bound to, or a `Poison` if unbound.
//! A member KEY is never resolved this way: it is a [`Symbol`] label (its spelling), read without any
//! scope/prelude lookup (`prelude-and-resolution.md` §A Member Key Is A Label, Not A Value).

use crate::ast::{IntValue, StructId};
use crate::diag::Reject;
use std::collections::BTreeMap;

/// A field/variant/member label — a name taken as data, NOT resolved to a value. A member access's
/// key and a record literal's field names are symbols: the projection finds a field BY this label and
/// never inspects a bound value for it.
///
/// A symbol carries an optional NAMESPACE so a name the language defines and a name a macro introduces
/// cannot collide (`contracts/ast-encoding.md` §A Prelude Symbol Is Namespaced). Ordering is by
/// (namespace, name), so a `BTreeMap` keyed by `Symbol` has a canonical field order — which is what
/// makes record equality and projection order-independent (a record's fields are a SET).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Symbol {
    /// The namespace this label belongs to, or `None` for an unqualified source name. Source names
    /// are unqualified today; the field exists so a macro-introduced label can carry its origin
    /// namespace when hygienic macros are added.
    pub namespace: Option<String>,
    pub name: String,
}

impl Symbol {
    /// An unqualified label from a source spelling (no namespace).
    pub fn plain(name: impl Into<String>) -> Symbol {
        Symbol {
            namespace: None,
            name: name.into(),
        }
    }
}

/// A NATIVE primitive — the irreducible bottom a `Meta.apply` (or a leaf value) names. Everything
/// user-facing is a record; a `Prim` is where the compiler's own machinery takes over ("bottom out on
/// an intrinsic, don't bloat the general node types"). There are two families:
///  - arithmetic operations (`+`/`-`/`*`) — `Meta.apply` of the operator records; folded/emitted in
///    `lower`/`select` by the width read off the solved type;
///  - type CONSTRUCTORS (`Int`/`UInt`) — `Meta.apply` builders the evaluator applies to a width to
///    build a concrete integer MODULE record, and the function-type constructor `->`.
///
/// A prelude `(intrinsic NAME)` node names one of these; the name→prim table is the ONE place a prim
/// spelling lives (the prelude authors it), so nothing downstream matches a source name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Prim {
    Add,
    Sub,
    Mul,
    /// Truncating integer division `/` (toward zero) and remainder `%` (sign of the dividend). Both
    /// trap on a zero divisor and on the `MIN / -1` overflow (`numeric-model.md` §Overflow Is Defined),
    /// so a provable trap folds to CDZ0304 like `*`.
    Div,
    Rem,
    /// Left shift `<<` — exact multiplication by `2^count`, so an overflowing shift traps like `*`, and
    /// a shift count outside `0..width` traps rather than masking (`numeric-model.md` §A Shift Is Not
    /// Exempt From Overflow Is Defined). Right shift `>>` is ARITHMETIC (sign-extending), also trapping
    /// on an out-of-range count.
    Shl,
    Shr,
    /// Bitwise `&` / `|` / `^` — total on the two's-complement value, never trap.
    BitAnd,
    BitOr,
    BitXor,
    /// The ordering comparisons `<` / `>` / `<=` / `>=` and equality `=` — each `∀a. a → a → Bool`.
    /// Unlike arithmetic (which is `∀a. (Int a) → (Int a) → (Int a)`), a comparison's result is `Bool`,
    /// and its operand is a BARE type variable — so it relates `Bool` as well as an integer (`(< false
    /// true)` = `true`), and STRUCTURALLY any value (a tuple, a map, a type-value). The I1 fold decides
    /// two constant SCALARS (`Int`/`Bool`); a compound or runtime operand declines (structural
    /// comparison over the value heap is a later stage) — the generic type stays, coverage grows behind
    /// a decline.
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    /// The TRUNCATING integer conversion `T.wrap : ∀(w,s). Int^s_w → T` — keeps the low `N` bits of the
    /// source's two's-complement value and interprets them at the TARGET width `N` and signedness. The
    /// source is a fully-polymorphic integer (any width/sign, via the operator record's type-lambda); the
    /// target is the MODULE's own width, read off the application's solved type at lowering. So there is
    /// ONE `Wrap` prim, not one per source type — no pair-explosion. It never traps and never returns an
    /// `Option`: truncation is total (`(UInt8.wrap 256) = 0`, `(UInt8.wrap -1) = 255`). The CHECKED
    /// companion `T.of` (returns `Option<T>`, `None` when out of range) arrives with sum types.
    Wrap,
    /// `Int : Nat → Module` — applied to a width, builds the signed integer module of that width.
    IntCtor,
    /// `UInt : Nat → Module` — the unsigned integer module builder.
    UIntCtor,
    /// `-> : (Type, Type) → Type` — the function-type constructor.
    FnCtor,
    /// `Tuple : (Type…) → Type` — the tuple-type constructor, VARIADIC over its element types. `(Tuple
    /// Int64 Bool)` builds the type-value `(Tuple Int64 Bool)`; a different arity or element type is a
    /// different type. Used in type position (an annotation `(: e (Tuple …))`), the arity/element check
    /// the annotation needs.
    TupleCtor,
    /// `Record : ((name Type)…) → Type` — the record-type constructor, VARIADIC over its `(name type)`
    /// field pairs. `(Record (a Int64) (b Bool))` builds the type-value `(Record (a Int64) (b Bool))`; the
    /// field-name SET and per-field types ARE the type. Used in type position (an annotation `(: e (Record
    /// …))`), giving the field-name/type check the annotation needs — the record companion of `TupleCtor`.
    RecordCtor,
    /// The ground type-values — nullary "constructors" that ARE a type-value directly (`Bool`/`Unit`
    /// resolve to a record whose `(meta t)` holds one of these).
    BoolTy,
    UnitTy,
    /// A SUM VARIANT CONSTRUCTOR — the `(meta apply)` of a variant field on a synthesized sum record
    /// (`crate::sums`). Applying it (`(Option.Some 5)`) builds the sum value `sum-new(disc, payload)`:
    /// the DISCRIMINANT is read off the variant record's `(meta variant)` channel at lowering (NOT
    /// baked into this prim — one `SumNew` serves every variant, like the one `Wrap` serves every target
    /// width, reading the target off the solved type). A NULLARY variant used bare is this prim applied
    /// to no arguments. The result is the owning sum type, read off the ctor's `(meta t)`.
    SumNew,
    /// A generic SUM TYPE CONSTRUCTOR — the `(meta apply)` of a GENERIC sum record (`crate::sums`).
    /// Applying it in TYPE position (`(Option Int64)`) builds the type-value `Ty::Sum { decl, args }`:
    /// the owning declaration is read off the record's `(meta sum-decl)` channel, the args are the
    /// applied type-values. One prim serves every generic sum (the decl is metadata, like `SumNew`'s
    /// discriminant), so `Option`/`Result`/… need no per-type prim — the same "type constructor's
    /// `(meta apply)` builds a type" model as `Int`/`Tuple`/`->`.
    SumCtor,
}

impl Prim {
    /// The primitive a prelude `(intrinsic NAME)` node names, or `None` if unrecognized. The one place
    /// a prim's source spelling is matched — the prelude authors these nodes, so no other pass sees a
    /// name.
    pub fn from_name(name: &str) -> Option<Prim> {
        match name {
            "+" => Some(Prim::Add),
            "-" => Some(Prim::Sub),
            "*" => Some(Prim::Mul),
            "/" => Some(Prim::Div),
            "%" => Some(Prim::Rem),
            "<<" => Some(Prim::Shl),
            ">>" => Some(Prim::Shr),
            "&" => Some(Prim::BitAnd),
            "|" => Some(Prim::BitOr),
            "^" => Some(Prim::BitXor),
            "<" => Some(Prim::Lt),
            ">" => Some(Prim::Gt),
            "<=" => Some(Prim::Le),
            ">=" => Some(Prim::Ge),
            "=" => Some(Prim::Eq),
            "wrap" => Some(Prim::Wrap),
            "Int" => Some(Prim::IntCtor),
            "UInt" => Some(Prim::UIntCtor),
            "->" => Some(Prim::FnCtor),
            "Tuple" => Some(Prim::TupleCtor),
            "Record" => Some(Prim::RecordCtor),
            "Bool" => Some(Prim::BoolTy),
            "Unit" => Some(Prim::UnitTy),
            "sum-new" => Some(Prim::SumNew),
            "sum-ctor" => Some(Prim::SumCtor),
            _ => None,
        }
    }

    /// Whether this primitive is a BINARY INTEGER operation — arithmetic, division, shift, or bitwise.
    /// Every one has the shape `∀a. (Int a) → (Int a) → (Int a)` and folds on two constant integer
    /// operands (a provable trap → CDZ0304); an operand that is not compile-time-known stays a runtime
    /// `Core::Arith`. (A comparison is NOT one of these — its result is `Bool`, handled separately.)
    pub fn is_arith(self) -> bool {
        matches!(
            self,
            Prim::Add
                | Prim::Sub
                | Prim::Mul
                | Prim::Div
                | Prim::Rem
                | Prim::Shl
                | Prim::Shr
                | Prim::BitAnd
                | Prim::BitOr
                | Prim::BitXor
        )
    }

    /// Whether this primitive is an integer CONVERSION — a unary op from a polymorphic source integer to
    /// a fixed target width. `Wrap` (truncating, returns `T`) is the only one now; the checked `Of`
    /// (returning `Option<T>`) joins it with sum types. Routed as a unary application in `lower`/`select`.
    pub fn is_conversion(self) -> bool {
        matches!(self, Prim::Wrap)
    }

    /// Whether this primitive is a relational comparison (`< > <= >=` or equality `=`) — shape `∀a. a →
    /// a → Bool`, a bare type variable so it relates `Bool` and (structurally) any value as well as
    /// integers, with a `Bool` result. Folds two constant SCALARS to a `ConstBool`; a compound/runtime
    /// operand declines. Never traps.
    pub fn is_comparison(self) -> bool {
        matches!(self, Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq)
    }

    /// Whether this primitive is a BINARY OPERATOR reached by application (arithmetic OR comparison) —
    /// the set the prelude installs as operator records and `meta_apply_of` dispatches. Used to route
    /// an `Apply` whose head is one of these into the operator-fold path.
    pub fn is_binop(self) -> bool {
        self.is_arith() || self.is_comparison()
    }

    /// The ground type-value this primitive denotes directly, if it is one (`BoolTy`→Bool, …). A
    /// ground type is a type VALUE with no application; a constructor prim returns `None` here (it
    /// yields a type only when applied).
    pub fn ground_type(self) -> Option<crate::ty::Ty> {
        match self {
            Prim::BoolTy => Some(crate::ty::Ty::Bool),
            Prim::UnitTy => Some(crate::ty::Ty::Unit),
            _ => None,
        }
    }
}

/// The resolved meaning of one AST node. Children are referenced by AST `StructId`; a query descends
/// by reading their slots on demand.
#[derive(Clone, PartialEq, Debug)]
pub enum Resolved {
    /// An integer literal at its exact arbitrary precision. Its machine width is a downstream type
    /// decision; the narrowing (and any out-of-range decline) happens at selection.
    Int(IntValue),
    /// A boolean literal.
    Bool(bool),
    /// The unit value (`()`).
    Unit,
    /// A reference to a binding: the name at this occurrence denotes the value at `value` (the
    /// initializer of the nearest enclosing `let`/`def`-parameter binding of that name). `type_of` and
    /// `core_of` follow the ref — a bare name IS its bound value's fact.
    Ref { value: StructId },
    /// A `let` binding form: each `(name init)` pair binds `name` to `init` for the initializers and
    /// body that follow (sequential; a later binding sees earlier ones; a repeat shadows). The whole
    /// form's value is `body`'s value. Bindings are carried as `(binder-name-occ, init-occ)` pairs so
    /// scope resolution finds them by walking here from a reference.
    Let {
        bindings: Vec<(StructId, StructId)>,
        body: StructId,
    },
    /// A two-way conditional. The three children are AST occurrences resolved on demand.
    If {
        cond: StructId,
        then_: StructId,
        else_: StructId,
    },
    /// A `(match scrutinee (pattern body)…)` — the pattern engine's surface. `scrutinee` is the value
    /// examined; each arm is a `(pattern-occ, body-occ)` pair, tried top-to-bottom. A pattern is carried
    /// as its AST occurrence (NOT a `Pattern` enum — `intermediate-representations.md`: patterns are
    /// ordinary nodes classified where consumed), so a literal pattern is an `Int`/`Bool` node and the
    /// wildcard is the name `_`. A SCALAR scrutinee is handled with literal, binder, and wildcard arms:
    /// an arm is a probe `scrutinee == literal` (or always, for a binder/`_`) and its body; the match
    /// lowers to a chain of `if`s (folded when the scrutinee is constant). A sum/tuple/record scrutinee
    /// walks the value heap rather than probing a scalar.
    Match {
        scrutinee: StructId,
        arms: Vec<(StructId, StructId)>,
    },
    /// A record literal: a fixed SET of named fields, each label mapping to its value occurrence. Held
    /// as a `BTreeMap` so the fields are canonically ordered (order-independent equality/projection)
    /// and a field lookup is O(log n), not a linear scan. The labels are symbols (never resolved); the
    /// values resolve on demand. A duplicate label is a `Poison` before construction (a record's field
    /// names are a set — `core-semantics.md` §A Record Has A Fixed Set Of Named Fields).
    /// (`fields` behind an `Arc` so CLONING a `Resolved::Record` — which `resolved_of` does on every
    /// memoized read — is a refcount bump, not a deep map copy. A record read field-by-field
    /// (`member_value` re-clones the operand's resolved form per access) was O(N²) in map clone;
    /// mirrors the `Ty::Record` Arc choice, faithful to Cadenza's ref-counted port target.)
    Record {
        fields: std::sync::Arc<BTreeMap<Symbol, StructId>>,
    },
    /// Member access `(. operand key)` — the ONE generic projection. `key` is a label read from the
    /// key occurrence's spelling, NOT resolved (`prelude-and-resolution.md` §Member Access Is One
    /// Generic Projection That Does Not Inspect Its Key). The projection resolves the field against
    /// the operand's type/value downstream.
    Member { operand: StructId, key: Symbol },
    /// A TUPLE literal `(tuple e0 e1 …)` — a fixed-arity POSITIONAL product. The elements are AST
    /// occurrences in order (resolved on demand); the tuple's ARITY and per-position element types ARE
    /// its type (a tuple of different arity or a differently-typed position is a different type —
    /// `type-system.md` §The Structural Types Are Record, Tuple, And Sum). Distinct from `Record` (named
    /// fields): a tuple is accessed by POSITION (`Proj`), a record by NAME (`Member`).
    /// (`elems` behind an `Arc<[StructId]>` so cloning a `Resolved::Tuple` is O(1) — same rationale as
    /// `Record`; a tuple projected element-by-element re-clones the operand's resolved form per access.)
    Tuple { elems: std::sync::Arc<[StructId]> },
    /// A tuple PROJECTION `(. operand N)` — member access whose key is an INTEGER literal selects the
    /// element at position `index` (0-based). The integer key is what distinguishes a positional tuple
    /// access from a named record field access (`Member`); a name key on a tuple, or an integer key on a
    /// record, is a type error decided downstream. An `index` outside the operand tuple's static arity is
    /// a COMPILE-TIME type error (CDZ0201), never a runtime trap (`type-system.md` §A Tuple Is Split At A
    /// Position Into A Prefix And A Suffix).
    Proj { operand: StructId, index: usize },
    /// The PAYLOAD a sum-variant pattern's binder binds — `(match s ((Some x) x))` resolves the `x`
    /// reference to this. `scrutinee` is the match scrutinee occurrence; `variant_head` is the pattern's
    /// variant-constructor occurrence (`(. Sum Variant)`), which carries the variant's discriminant (for
    /// the payload's type) via its `(meta variant)` + payload arrow. Its type is the variant's payload
    /// type (read from the constructor's `(-> payload Sum)`); at lowering it becomes
    /// `Core::SumPayload { scrutinee }` (a `sum-payload` read + unbox). A pattern binder is scoped to its
    /// arm (resolve Case 6), the sum analogue of the scalar binder-binds-the-scrutinee Case 5 — but here
    /// the binder binds the PAYLOAD, not the whole scrutinee.
    SumPayload {
        scrutinee: StructId,
        variant_head: StructId,
    },
    /// A NATIVE primitive value — what a prelude `(intrinsic …)` node resolves to (an arithmetic
    /// operation or a type constructor). The irreducible bottom a `Meta.apply` names; carried as a
    /// VALUE and reduced/lowered by the machinery that owns it, never special-cased by name
    /// (`reference-compiler.md` §A Built-In Operation Is A First-Class Value, Lowered At Selection).
    Prim(Prim),
    /// Application `(head arg…)` — the ONE application form. `head` and each `arg` are AST occurrences
    /// resolved on demand; to apply, project the head value's `(meta apply)` and use it if applyable
    /// (else reject "not applyable"). One path serves an operator, a type constructor, and (later) a
    /// user function — dispatch is by the head value's meta channel, never its spelling
    /// (`prelude-and-resolution.md` §A Form Whose Head Is Not A Grammar Name Is Dispatched By The Kind
    /// Of Value Its Head Resolves To).
    Apply { head: StructId, args: Vec<StructId> },
    /// A TYPE ANNOTATION `(: expr ty_expr)` — the value of `expr`, with its type CONSTRAINED to the
    /// type `ty_expr` denotes. The annotation is transparent to the value: `(: e T)` evaluates and
    /// lowers exactly as `e` (the annotation ERASES). Its force is on inference — the type `ty_expr`
    /// reduces to is unified into `expr`'s type, so `(: 5 Int64)` pins the literal's width and `(: true
    /// Int64)` is a conflicting-use rejection (CDZ0203). This is what disambiguates an otherwise-
    /// ambiguous type (an integer parameter with no other constraint), which is why it must exist
    /// before a runtime parameter can be given a definite machine width. Both children are AST
    /// occurrences: `expr` the annotated value, `ty_expr` the type EXPRESSION — reduced to a `Ty` by
    /// the evaluator downstream (`typeval_of`), NOT here, since resolve is a pure per-node classify and
    /// reducing a type constructor like `(Int 8)` needs the evaluator.
    Annot { expr: StructId, ty_expr: StructId },
    /// A first-class TYPE value. A type is an ordinary value (mixable, returnable) — using `Bool` in
    /// type position projects a record's `(meta t)` field, which holds one of these; a type
    /// constructor applied (`(Int a)`, `(-> A B)`) reduces through the one evaluator to one of these.
    /// It is compile-time-only: the erasure fence forbids it reaching the runtime boundary.
    TypeVal(crate::ty::Ty),
    /// A lambda PARAMETER occurrence used as a value — a formal not yet substituted. `infer` gives it
    /// a fresh type variable (the parameter's type, to be solved); the evaluator substitutes the
    /// argument here when the lambda is β-reduced at application. `binder` is this parameter's own
    /// occurrence (its identity), so two references to the same parameter share one variable.
    Param { binder: StructId },
    /// A compile-time lambda `(fn (param…) body)` — a value. Its parameters bind in scope for `body`
    /// (the ordinary parameter-scope mechanism); the evaluator β-reduces it when applied. An
    /// operator's `Meta.t` is such a lambda over the width (`(fn (a) (-> (Int a) …))`), so a "type
    /// scheme" is just a compile-time lambda from a type/width to a type — instantiation is applying
    /// it to a fresh variable. Params are the binder-name occurrences; `body` is the body occurrence.
    Lambda {
        // `params` behind an `Arc<[StructId]>` so cloning a `Resolved::Lambda` is a refcount bump; a
        // def name resolving to its lambda is read once per call site, and `resolved_of` clones on
        // every read. Same rationale as `Record`/`Tuple`.
        params: std::sync::Arc<[StructId]>,
        body: StructId,
    },
    /// A produced "no": an unrecognized head, a malformed form, an unbound name, or an unmodeled
    /// literal. Carries its reject/decline so the fault is reported at the node it was found.
    Poison(Reject),
}
