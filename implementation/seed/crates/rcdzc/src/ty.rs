//! The type language + the Hindley-Milner engine (type variables, substitution, unification,
//! occurs-check).
//!
//! Per `inference-plan-learn-from-seed-coarse-kind-mistakes` (ask-75): inference solves over ACTUAL
//! type variables, not a coarse wasm-valtype lattice re-derived at emit. A recursive self-call or a
//! yet-unsolved operand gets a fresh `TVar` that unifies with its concrete sibling REGARDLESS of
//! order — the order-independence the old compiler faked with tie-break tables comes for free from
//! unification. The wasm valtype is a READ-OFF of the fully-solved `Ty` at lower time
//! (`core_valtype`/`comp_valtype`), never the thing inferred.
//!
//! Phase 1 scope: `Int`, `Bool`, `Unit`, and `Var`. Structural types (Tuple/Record/Sum/Fn/List) and
//! let-generalization grow in later phases; the engine (fresh vars + unify + subst + occurs-check) is
//! the whole HM core, so those are new `Ty` variants + unify arms, not a new mechanism.

use crate::ir::ValType;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// A type variable identifier — a fresh integer from the inference context.
pub type TVar = u32;

/// The definition of a SUM type — its name, type parameters, and variants in DECLARATION order (a
/// variant's index in `variants` IS its runtime discriminant). Built ONCE per type: the prelude builds
/// Option/Result/Sign/Ordering (§`prelude_sums`); a user `(type …)` builds one per declaration. Shared
/// by `Arc`, so type IDENTITY is pointer identity (see [`SumRef`]).
#[derive(Debug)]
pub struct SumDef {
    /// The type's name (`"Option"`, `"Result"`, a user `"Sign"`) — the render qualifier + a human label.
    pub name: String,
    /// The type parameters, in order (`["a"]` for Option, `["a","e"]` for Result, `[]` for Sign). A
    /// variant payload template refers to param `i` as `Ty::Param(i)`.
    pub params: Vec<String>,
    /// The variants in DECLARATION order — index = discriminant. Some=0/None=1, Ok=0/Err=1. Held behind
    /// a `OnceLock` so a RECURSIVE type can be built in TWO PHASES: allocate the `Arc<SumDef>` with the
    /// variants UNFILLED, then fill them once the Arc exists — a variant payload (`Neg Expr`, `List Ast`)
    /// can then embed `Ty::Sum { def: <this same Arc>, … }`, and a mutually-recursive sibling's Arc is
    /// likewise in hand before either's variants are set. Read through [`SumDef::variants`].
    variants: OnceLock<Vec<VariantDef>>,
    /// Whether a value of this type renders its variant name QUALIFIED (`Sign.Pos`, a user
    /// `Color.Red`) vs BARE (`Some`, `None`, `Ok`, `Err` — the built-in Option/Result). Matches the
    /// canonical value forms in the corpus.
    pub qualified: bool,
}

impl SumDef {
    /// A fully-defined sum (variants known up front) — the prelude path. Wraps the variants in the
    /// `OnceLock` immediately.
    pub fn new(
        name: String,
        params: Vec<String>,
        variants: Vec<VariantDef>,
        qualified: bool,
    ) -> SumDef {
        let cell = OnceLock::new();
        let _ = cell.set(variants);
        SumDef {
            name,
            params,
            variants: cell,
            qualified,
        }
    }

    /// A FORWARD declaration — the Arc is allocated with variants unset (phase 1 of the two-phase
    /// recursive build). `set_variants` fills them (phase 2).
    pub fn forward(name: String, params: Vec<String>, qualified: bool) -> SumDef {
        SumDef {
            name,
            params,
            variants: OnceLock::new(),
            qualified,
        }
    }

    /// Fill the variants of a forward-declared sum (phase 2). Idempotent-safe: a second call is a
    /// no-op (the `OnceLock` keeps the first). Callers set exactly once.
    pub fn set_variants(&self, variants: Vec<VariantDef>) {
        let _ = self.variants.set(variants);
    }

    /// The variants in declaration order. Empty until `set_variants` runs (a forward decl mid-build);
    /// every consumer reads a fully-built def, so this is the full variant set in practice.
    pub fn variants(&self) -> &[VariantDef] {
        self.variants.get().map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// One variant of a sum: its name and its payload TEMPLATE (a `Ty` over the sum's params via
/// `Ty::Param(i)`; `None` = a nullary variant, whose argument type is Unit). `Some`→`Some(Param(0))`,
/// `None`→`None`, `Ok`→`Some(Param(0))`, `Err`→`Some(Param(1))`.
#[derive(Debug)]
pub struct VariantDef {
    pub name: String,
    pub payload: Option<Ty>,
}

/// A reference to a [`SumDef`], shared by `Arc`. Type IDENTITY is `Arc::ptr_eq` — two sums are the
/// SAME type iff their defs are the same allocation. Rust forges a unique identity via the pointer, so
/// no separate id counter is needed. ⚡PORTABILITY: the Cadenza-authored compiler has no pointer
/// identity, so it will forge an explicit integer id on `SumDef` and compare that; `ptr_eq` here is the
/// Rust spelling of the same "identical iff the same def" rule. This newtype makes `Ty`'s derived
/// `PartialEq`/`Eq` use pointer identity for the `Sum` arm (a re-declared same-shape type is a distinct
/// allocation → a distinct type, matching the spec's nominal-identity rule).
#[derive(Debug, Clone)]
pub struct SumRef(pub Arc<SumDef>);

impl PartialEq for SumRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for SumRef {}

impl std::ops::Deref for SumRef {
    type Target = SumDef;
    fn deref(&self) -> &SumDef {
        &self.0
    }
}

/// Instantiate a variant payload TEMPLATE by substituting each `Ty::Param(i)` with `args[i]` — used to
/// type a constructor's payload and a match arm's payload binder at the sum's instantiated args. A
/// nullary payload (`None` template) instantiates to `Ty::Unit`.
pub fn instantiate(template: &Option<Ty>, args: &[Ty]) -> Ty {
    match template {
        None => Ty::Unit,
        Some(t) => subst_params(t, args),
    }
}

/// Replace every `Ty::Param(i)` in `t` with `args[i]`, recursing into compounds. A `Param` index out of
/// range is left as-is (a compiler-internal inconsistency; never happens for a well-formed `SumDef`).
fn subst_params(t: &Ty, args: &[Ty]) -> Ty {
    match t {
        Ty::Param(i) => args.get(*i).cloned().unwrap_or(Ty::Param(*i)),
        Ty::Tuple(es) => Ty::Tuple(es.iter().map(|e| subst_params(e, args)).collect()),
        Ty::Record(fs) => Ty::Record(
            fs.iter()
                .map(|(n, e)| (n.clone(), subst_params(e, args)))
                .collect(),
        ),
        Ty::List(e) => Ty::List(Box::new(subst_params(e, args))),
        Ty::Map(k, v) => Ty::Map(
            Box::new(subst_params(k, args)),
            Box::new(subst_params(v, args)),
        ),
        Ty::Set(e) => Ty::Set(Box::new(subst_params(e, args))),
        Ty::Sum { def, args: sargs } => Ty::Sum {
            def: def.clone(),
            args: sargs.iter().map(|e| subst_params(e, args)).collect(),
        },
        Ty::Fn(ps, r) => Ty::Fn(
            ps.iter().map(|p| subst_params(p, args)).collect(),
            Box::new(subst_params(r, args)),
        ),
        // Type is a ground leaf — no params to substitute.
        Ty::Type => Ty::Type,
        other => other.clone(),
    }
}

/// The PRELUDE sum types as PROCESS-GLOBAL singletons, so every reference to `Option`/`Result`/… shares
/// ONE `SumDef` allocation and `Arc::ptr_eq` identity holds EVERYWHERE — resolve's bare-name/member
/// resolution, a fallible intrinsic's `signature()` return type (`Bytes.at : … → Option Int`), and a
/// `(match … ((Some x) …))` pattern all name the SAME `Option`. (Before, resolve built a fresh Option
/// per compile and an intrinsic had no access to it; a singleton lets the two agree by identity.)
/// ⚡PORTABILITY: the Cadenza port forges a fixed id per prelude sum; this is the Rust spelling.
static PRELUDE_SUMS: OnceLock<PreludeSums> = OnceLock::new();

struct PreludeSums {
    option: SumRef,
    result: SumRef,
    sign: SumRef,
    ordering: SumRef,
    ast: SumRef,
}

fn prelude() -> &'static PreludeSums {
    PRELUDE_SUMS.get_or_init(|| {
        let nullary = |name: &str| VariantDef {
            name: name.to_string(),
            payload: None,
        };
        let unary = |name: &str, p: usize| VariantDef {
            name: name.to_string(),
            payload: Some(Ty::Param(p)),
        };
        // `Ast` — the built-in metaprogramming sum, an ORDINARY sum whose variants carry typed payloads
        // (type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type). RECURSIVE via
        // `List (List Ast)`, so it is built in two phases: allocate the Arc forward, then fill its
        // variants (one references the same Arc). Qualified (`Ast.Int`, …). Mirrors the old compiler's
        // `PRELUDE_TYPES` Ast: `Int Int64 | Float Float64 | Str String | Bool Bool | Name String | List (List Ast)`.
        // (Float is declared for shape parity; `Ty::Float` is unimplemented, so an `Ast.Float` payload
        // would decline downstream — never miscompiles.)
        let ast_def = Arc::new(SumDef::forward("Ast".to_string(), vec![], true));
        let ast = SumRef(ast_def.clone());
        ast_def.set_variants(vec![
            VariantDef {
                name: "Int".to_string(),
                payload: Some(Ty::Int),
            },
            VariantDef {
                name: "Float".to_string(),
                payload: Some(Ty::Unit),
            }, // Float64 unimplemented; placeholder
            VariantDef {
                name: "Str".to_string(),
                payload: Some(Ty::String),
            },
            VariantDef {
                name: "Bool".to_string(),
                payload: Some(Ty::Bool),
            },
            VariantDef {
                name: "Name".to_string(),
                payload: Some(Ty::String),
            },
            VariantDef {
                name: "List".to_string(),
                payload: Some(Ty::List(Box::new(Ty::Sum {
                    def: ast.clone(),
                    args: vec![],
                }))),
            },
        ]);
        PreludeSums {
            option: SumRef(Arc::new(SumDef::new(
                "Option".to_string(),
                vec!["a".to_string()],
                vec![unary("Some", 0), nullary("None")],
                false,
            ))),
            result: SumRef(Arc::new(SumDef::new(
                "Result".to_string(),
                vec!["a".to_string(), "e".to_string()],
                vec![unary("Ok", 0), unary("Err", 1)],
                false,
            ))),
            sign: SumRef(Arc::new(SumDef::new(
                "Sign".to_string(),
                vec![],
                vec![nullary("Neg"), nullary("Zero"), nullary("Pos")],
                true,
            ))),
            ordering: SumRef(Arc::new(SumDef::new(
                "Ordering".to_string(),
                vec![],
                vec![nullary("Less"), nullary("Equal"), nullary("Greater")],
                true,
            ))),
            ast,
        }
    })
}

/// The shared `Option` sum def — `Some`=variant 0, `None`=variant 1. `Bytes.at`/`List.at` return an
/// `Option` of this exact def, so a match on the result unifies by `Arc::ptr_eq`.
pub fn prelude_option() -> SumRef {
    prelude().option.clone()
}
/// The shared `Result` sum def — `Ok`=0, `Err`=1.
pub fn prelude_result() -> SumRef {
    prelude().result.clone()
}
/// The shared `Sign` sum def.
pub fn prelude_sign() -> SumRef {
    prelude().sign.clone()
}
/// The shared `Ordering` sum def.
pub fn prelude_ordering() -> SumRef {
    prelude().ordering.clone()
}
/// The shared built-in `Ast` sum def (the metaprogramming AST — `Int`/`Float`/`Str`/`Bool`/`Name`/`List`,
/// recursive via `List (List Ast)`). Qualified rendering (`Ast.Int`, …).
pub fn prelude_ast() -> SumRef {
    prelude().ast.clone()
}

/// A Cadenza type. Inference solves STRUCTURE; the wasm valtype falls out of the solved type.
/// (`Unit` is defined now for the `do`/`if`-with-unit forms a later phase adds; it participates in
/// `unify`/`core_valtype` already so those forms are a resolve/infer change, not a `Ty` change.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    /// The 64-bit signed integer — the default integer type.
    Int,
    /// The boolean.
    Bool,
    /// The empty product.
    Unit,
    /// A fixed-arity positional product — a tuple. Its ARITY and element types are part of the type
    /// (a `(tuple Int Int)` is distinct from a `(tuple Int Bool)` and from a 3-tuple). A tuple VALUE
    /// lives on the value heap (an `arr` handle); its wasm valtype is `I32` (the handle).
    Tuple(Vec<Ty>),
    /// A function value's type — parameter types → result type. A function value is a reference to a
    /// CLOSED top-level/module function (no captured environment — not a closure); it exists so a module
    /// field can hold a function that is projected then applied. It is not (yet) a runtime value: the
    /// fold resolves every application to a direct call, and a function value that would cross the run
    /// boundary declines. So `core_valtype`/`comp_valtype` are `None`.
    Fn(Vec<Ty>, Box<Ty>),
    /// A fixed named product — a record. Its field NAME SET and each field's type are part of the type
    /// (a `(Record (a Int) (b Bool))` is distinct from `(Record (a Int))`). The field list is kept
    /// SORTED by name — the canonical form — so structural equality/unify is positional and matches
    /// the value-heap slot order (a record is a positional `arr`, slots = field values sorted by name,
    /// like a tuple). A record VALUE is an `arr` handle; its wasm valtype is `I32`.
    Record(Vec<(String, Ty)>),
    /// A homogeneous, ordered, growable sequence — a `List T`. This is the first genuinely PARAMETRIC
    /// type: `T` (the element type) is a full `Ty`, and a `(list …)`'s every element unifies to that
    /// one `T` (a mixed-element list is a type error — the generic-unification payoff). A list VALUE is
    /// a persistent-sequence handle on the value heap (the runtime's RRB `vec-*`, distinct from the
    /// fixed `arr-*` a tuple/record uses); its wasm valtype is `I32`. Unlike a tuple, its LENGTH is not
    /// part of the type — only the element type is (collections-and-text.md §A List Is An Ordered
    /// Homogeneous Sequence).
    List(Box<Ty>),
    /// An immutable byte sequence — a `Bytes` value. A heap leaf on the runtime's `bytes-*`
    /// representation (himports 13–16/29–31); its wasm valtype is `I32` (the handle). NOT parametric
    /// (its element is always a byte). The value form a self-hosted compiler builds its output wasm
    /// bytes up as (collections-and-text.md; 10-bytes.sexp).
    Bytes,
    /// A text string — a sequence of Unicode scalar values, stored as its NFC UTF-8 bytes. At RUN TIME
    /// a String IS the same `bytes-*` heap leaf a `Bytes` is (collections-and-text.md #A String Is A
    /// Sequence Of Unicode Scalar Values); the ONLY difference from `Bytes` is the static type, which
    /// drives the renderer to quote it `"…"` (not `b"…"`) and equality to compare its bytes. Its wasm
    /// valtype is `I32` (the handle).
    String,
    /// A `Map K V` — a persistent key→value collection (the runtime's CHAMP map). PARAMETRIC in TWO
    /// types (key + value); a map's KEY SET is runtime data, NOT part of its type — two maps with
    /// different keys are the SAME type (05-compound-types.sexp §a list of maps with different keys is
    /// homogeneous). Both keys and values are heap handles (boxed scalars / compounds). A `Map` VALUE is
    /// a CHAMP handle; its wasm valtype is `I32`.
    Map(Box<Ty>, Box<Ty>),
    /// A `Set E` — a persistent unordered collection of elements (the runtime's CHAMP set; NOT a
    /// `Map E Unit`). PARAMETRIC in ONE type. Elements are heap handles. A `Set` VALUE is a CHAMP handle;
    /// its wasm valtype is `I32`.
    Set(Box<Ty>),
    /// A SUM type instance — `def` (the shared variant set, via `Arc`; identity = `Arc::ptr_eq`) applied
    /// to type ARGS (`Option Int` = `Sum{def:<Option>, args:[Int]}`; `Sign` = `Sum{def:<Sign>, args:[]}`).
    /// Parametric like `List`: `args` are the instantiated type parameters, so `Some : a → Option a` is a
    /// fresh `a` per use. A sum VALUE is a `(disc, payload-handle)` heap value; its wasm valtype is `I32`.
    /// The variant set / declaration-order discriminants / names live on `def`, read straight off the type
    /// (no separate registry).
    Sum { def: SumRef, args: Vec<Ty> },
    /// A sum type PARAMETER placeholder — appears ONLY inside a [`SumDef`]'s variant payload templates
    /// (`Some`'s payload is `Param(0)`), never in an inferred type. `instantiate`/`subst_params` replace
    /// it with the sum's instantiated `args` at a constructor / match-binder site. Treated as inert by
    /// `unify`/`apply` (it is not a solvable inference var — it is only ever substituted away first).
    Param(usize),
    /// An unsolved type variable (bound in the substitution once unified).
    Var(TVar),
    /// The type of type-values (kind level). A compile-time-only type: a `Type`-typed value must never
    /// reach runtime (erased before lowering by the fence in `fold`). `core_valtype`/`comp_valtype` are
    /// `None` (no runtime representation). A bare type name (`Int64`, `Bool`) resolves to a `TypeVal(Ty)`
    /// value of this type — the foundation for first-class types and type-fns.
    Type,
}

impl Ty {
    /// The core wasm valtype byte for this type's values (used in function signatures / the run
    /// The wasm value type this type's values occupy (a function-signature slot, a local, an `if`
    /// result). Returned as a `ValType`, NOT a raw encoding byte — the byte is `serialize`'s concern.
    /// `Unit` has no value slot (`None` here means "no wasm value", distinct from `Var` which is a
    /// bug — an unsolved type — reported by the caller). `Int`→I64, `Bool`→I32.
    pub fn core_valtype(&self) -> Option<ValType> {
        match self {
            Ty::Int => Some(ValType::I64),
            Ty::Bool => Some(ValType::I32),
            Ty::Tuple(_) | Ty::Record(_) => Some(ValType::I32), // a heap handle (an `arr`)
            Ty::List(_) => Some(ValType::I32), // a heap handle (a persistent `vec`)
            Ty::Map(..) | Ty::Set(_) => Some(ValType::I32), // a heap handle (a CHAMP map/set)
            Ty::Sum { .. } => Some(ValType::I32), // a heap handle (a `(disc, payload)`)
            Ty::Bytes => Some(ValType::I32),   // a heap handle (a `bytes-*` leaf)
            Ty::String => Some(ValType::I32),  // a heap handle (a Bytes-backed leaf)
            Ty::Unit => None,                  // no wasm value
            // A function value is not (yet) a runtime value — the fold resolves it to a direct call.
            Ty::Fn(..) => None,
            // A type-value is compile-time-only, erased before runtime (no wasm rep).
            Ty::Type => None,
            Ty::Param(_) | Ty::Var(_) => None,
        }
    }

    /// The component-model primitive valtype byte the run export presents at the boundary. This IS a
    /// component-model encoding (distinct from the core `ValType`): `Int`→s64 (0x78), `Bool`→bool
    /// (0x7F). `Unit` uses a distinct envelope (no unnamed result); `Var` is a bug.
    pub fn comp_valtype(&self) -> Option<u8> {
        match self {
            Ty::Int => Some(0x78),  // s64
            Ty::Bool => Some(0x7F), // bool
            // A compound (tuple/record) crosses the boundary via the runtime-compound resource/render
            // envelope (rendered to a string), NOT as an unnamed scalar result — no primitive
            // comp valtype.
            Ty::Tuple(_) | Ty::Record(_) => None,
            // A list crosses the boundary via the runtime-compound render envelope (rendered `(list
            // …)`), NOT as a primitive scalar — no comp valtype.
            Ty::List(_) => None,
            // A map/set crosses via the render envelope (rendered `(map …)` / `(set …)`), not a scalar.
            Ty::Map(..) | Ty::Set(_) => None,
            // A sum crosses via the render envelope (rendered `(Variant payload)`), not a scalar.
            Ty::Sum { .. } => None,
            // Bytes/String cross via the render envelope (rendered `b"…"` / `"…"`), not a scalar.
            Ty::Bytes | Ty::String => None,
            Ty::Unit => None,
            Ty::Fn(..) => None,
            // A type-value is compile-time-only, erased before runtime (no boundary rep).
            Ty::Type => None,
            Ty::Param(_) | Ty::Var(_) => None,
        }
    }

    /// Is this a compound (heap) type — one whose VALUE lives on the value heap and crosses the run
    /// boundary via the runtime-compound render envelope rather than as a scalar? A tuple/record/list/
    /// sum is; later strings/bytes are too. (Consumed by the heap path.)
    pub fn is_compound(&self) -> bool {
        matches!(
            self,
            Ty::Tuple(_)
                | Ty::Record(_)
                | Ty::List(_)
                | Ty::Map(..)
                | Ty::Set(_)
                | Ty::Sum { .. }
                | Ty::Bytes
                | Ty::String
        )
    }
}

/// Is this type compile-time-only — a value of this type must never reach runtime? True if `Ty::Type`
/// (or a `Ty::Fn` returning a comptime-only type) appears ANYWHERE inside `ty` — recurses through
/// Tuple/List/Map/Set/Record fields, Sum args, Fn params+result. The post-fold erasure fence applies
/// this structurally to every surviving Mir node to catch a type-value smuggled inside a compound (a
/// `(tuple TypeVal TypeVal)`, a list element, a sum payload, a record field). A comptime-only value
/// that survives fold and would cross the runtime boundary is the erasure fence reject (CDZ0305). This
/// subsumes the old "default Fn-internal vars to Unit" into one named property.
pub fn is_comptime_only(ty: &Ty) -> bool {
    match ty {
        Ty::Type => true,
        Ty::Fn(params, ret) => params.iter().any(is_comptime_only) || is_comptime_only(ret),
        Ty::Tuple(elems) => elems.iter().any(is_comptime_only),
        Ty::Record(fields) => fields.iter().any(|(_, t)| is_comptime_only(t)),
        Ty::List(elem) => is_comptime_only(elem),
        Ty::Map(k, v) => is_comptime_only(k) || is_comptime_only(v),
        Ty::Set(elem) => is_comptime_only(elem),
        Ty::Sum { args, .. } => args.iter().any(is_comptime_only),
        _ => false,
    }
}

/// A substitution: type-variable → its solved type. `apply` walks a `Ty` replacing every bound var
/// with its solution transitively (a var may resolve to another var). This is the HM solution state.
#[derive(Debug, Default, Clone)]
pub struct Subst {
    map: HashMap<TVar, Ty>,
}

impl Subst {
    pub fn new() -> Subst {
        Subst {
            map: HashMap::new(),
        }
    }

    /// Fully resolve `ty` under the current substitution: follow var→type chains to a ground type or
    /// an unbound var, recursing into COMPOUND types so a var bound INSIDE a tuple/record element is
    /// resolved too (e.g. a record `(a Var0)` whose `Var0` later unifies to `Int` resolves to
    /// `(a Int)`). (No cycles: `unify`'s occurs-check prevents a var mapping through itself.)
    pub fn apply(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Var(v) => match self.map.get(v) {
                Some(t) => self.apply(t),
                None => Ty::Var(*v),
            },
            Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|e| self.apply(e)).collect()),
            Ty::Record(fields) => Ty::Record(
                fields
                    .iter()
                    .map(|(n, t)| (n.clone(), self.apply(t)))
                    .collect(),
            ),
            Ty::Fn(params, ret) => Ty::Fn(
                params.iter().map(|p| self.apply(p)).collect(),
                Box::new(self.apply(ret)),
            ),
            Ty::List(elem) => Ty::List(Box::new(self.apply(elem))),
            Ty::Map(k, v) => Ty::Map(Box::new(self.apply(k)), Box::new(self.apply(v))),
            Ty::Set(elem) => Ty::Set(Box::new(self.apply(elem))),
            Ty::Sum { def, args } => Ty::Sum {
                def: def.clone(),
                args: args.iter().map(|a| self.apply(a)).collect(),
            },
            // Type is a ground leaf — no substitution needed.
            Ty::Type => Ty::Type,
            other => other.clone(),
        }
    }

    /// Bind `v := ty` (caller has already occurs-checked). Records the solution.
    fn bind(&mut self, v: TVar, ty: Ty) {
        self.map.insert(v, ty);
    }
}

/// A unification failure: the two ground types that could not be made equal, in solved form. Carried
/// up to a diagnostic (a type error, CDZ0201) — reported at both sites per type-system.md:62-64.
#[derive(Debug, Clone)]
pub struct UnifyError {
    pub left: Ty,
    pub right: Ty,
}

/// Unify two types under `subst`, extending the substitution so they become equal. Standard HM:
/// - a `Var` unifies with anything (after occurs-check) by binding it;
/// - two identical ground types succeed with no change;
/// - two different ground types fail (a type error).
/// Order-independent: `unify(Var, Int)` and `unify(Int, Var)` both bind the var to `Int`.
pub fn unify(a: &Ty, b: &Ty, subst: &mut Subst) -> Result<(), UnifyError> {
    let a = subst.apply(a);
    let b = subst.apply(b);
    match (&a, &b) {
        (Ty::Int, Ty::Int)
        | (Ty::Bool, Ty::Bool)
        | (Ty::Unit, Ty::Unit)
        | (Ty::Bytes, Ty::Bytes)
        | (Ty::String, Ty::String)
        | (Ty::Type, Ty::Type) => Ok(()),
        (Ty::Var(x), Ty::Var(y)) if x == y => Ok(()),
        (Ty::Var(v), t) | (t, Ty::Var(v)) => {
            if occurs(*v, t, subst) {
                return Err(UnifyError {
                    left: a.clone(),
                    right: b.clone(),
                });
            }
            subst.bind(*v, t.clone());
            Ok(())
        }
        // Two tuples unify iff same arity and element-wise unifiable (arity is part of the type).
        (Ty::Tuple(xs), Ty::Tuple(ys)) if xs.len() == ys.len() => {
            for (x, y) in xs.iter().zip(ys) {
                unify(x, y, subst)?;
            }
            Ok(())
        }
        // Two records unify iff the same field-NAME set and each field's types unify. Both lists are
        // sorted by name (the canonical form), so equal names line up positionally.
        (Ty::Record(xs), Ty::Record(ys)) if xs.len() == ys.len() => {
            for ((xn, xt), (yn, yt)) in xs.iter().zip(ys) {
                if xn != yn {
                    return Err(UnifyError {
                        left: a.clone(),
                        right: b.clone(),
                    });
                }
                unify(xt, yt, subst)?;
            }
            Ok(())
        }
        // Two lists unify iff their ELEMENT types unify (the parametric-type rule: `List a` unifies
        // with `List b` exactly when `a` unifies with `b` — so `(list 1 2)`'s `List Int` clashes with
        // `(list true)`'s `List Bool` at the element, a type error).
        (Ty::List(x), Ty::List(y)) => unify(x, y, subst),
        // Two maps unify iff their KEY types unify and their VALUE types unify (parametric in 2). A
        // map's key SET is runtime data, not part of the type — only the key/value TYPES are, so
        // `(map (5 1))` and `(map (6 2))` are both `Map Int Int`, one type.
        (Ty::Map(xk, xv), Ty::Map(yk, yv)) => {
            unify(xk, yk, subst)?;
            unify(xv, yv, subst)
        }
        // Two sets unify iff their ELEMENT types unify (parametric in 1).
        (Ty::Set(x), Ty::Set(y)) => unify(x, y, subst),
        // Two sums unify iff they are the SAME def (Arc::ptr_eq — nominal identity) and their type ARGS
        // unify element-wise. Different def → type error (`(= (Some 1) (Ok 1))` — disjoint variant
        // sets); same def different args → the args clash (`(Some true)` vs `Option Int` → the payload
        // arg Bool≠Int). Two variants of the SAME sum (Some/None) share the def, so they unify — a
        // well-typed comparison yielding false, not a type error.
        (Ty::Sum { def: d1, args: a1 }, Ty::Sum { def: d2, args: a2 })
            if Arc::ptr_eq(&d1.0, &d2.0) && a1.len() == a2.len() =>
        {
            for (x, y) in a1.iter().zip(a2) {
                unify(x, y, subst)?;
            }
            Ok(())
        }
        // Two function types unify iff same arity and element-wise params + result unify.
        (Ty::Fn(xps, xr), Ty::Fn(yps, yr)) if xps.len() == yps.len() => {
            for (x, y) in xps.iter().zip(yps) {
                unify(x, y, subst)?;
            }
            unify(xr, yr, subst)
        }
        _ => Err(UnifyError { left: a, right: b }),
    }
}

/// Occurs-check: does type variable `v` appear in `ty` (under the substitution)? Prevents an
/// infinite type (`a = a → a`). Phase 1 has no type constructors that contain types, so this can
/// only fire on `v` itself; it is written to recurse so later structural `Ty` variants are covered
/// by construction.
fn occurs(v: TVar, ty: &Ty, subst: &Subst) -> bool {
    match subst.apply(ty) {
        Ty::Var(w) => v == w,
        Ty::Int | Ty::Bool | Ty::Unit | Ty::Bytes | Ty::String | Ty::Type => false,
        Ty::Tuple(elems) => elems.iter().any(|e| occurs(v, e, subst)),
        Ty::Record(fields) => fields.iter().any(|(_, t)| occurs(v, t, subst)),
        Ty::Fn(params, ret) => params.iter().any(|p| occurs(v, p, subst)) || occurs(v, &ret, subst),
        Ty::List(elem) => occurs(v, &elem, subst),
        Ty::Map(k, val) => occurs(v, &k, subst) || occurs(v, &val, subst),
        Ty::Set(elem) => occurs(v, &elem, subst),
        // A sum's args may carry the var; the def itself (a name + variant templates) never does.
        Ty::Sum { args, .. } => args.iter().any(|a| occurs(v, a, subst)),
        // A param placeholder is not a solvable var and contains none.
        Ty::Param(_) => false,
    }
}

/// A supply of fresh type variables — a monotonic counter. The inference context holds one.
#[derive(Debug, Default)]
pub struct TVarSupply {
    next: TVar,
}

impl TVarSupply {
    pub fn new() -> TVarSupply {
        TVarSupply { next: 0 }
    }
    /// A fresh, never-before-issued type variable.
    pub fn fresh(&mut self) -> Ty {
        let v = self.next;
        self.next += 1;
        Ty::Var(v)
    }
}
