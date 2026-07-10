//! The IR ladder — one typed sum per rung, matched exhaustively by the pass below it.
//!
//! Mirrors `implementation/compiler/cdzc/10-ir.cdz`. Each rung is a Rust `enum`; a pass over it is
//! an exhaustive `match`, so a variant a pass does not handle is a *compile error in the compiler*,
//! never a silent fall-through (nanopass discipline). Later phases grow a rung a variant and the
//! pass an arm in lockstep.
//!
//! Phase 2a: a MODULE is a set of top-level functions (and value-defs, desugared to nullary
//! functions the callers reference). A function body is the Phase-1 expression grammar plus
//! parameter references and CALLS. Records/sums/match/the heap grow later.

use crate::diag::Code;
use crate::ty::{SumRef, Ty};

/// A wasm value type — the concrete machine type a value occupies (a function-signature slot, a
/// local, an operand). The compiler works in `ValType`/`BlockType`, never raw wasm encoding bytes;
/// the single byte encoding lives in `serialize` (`ValType::byte`/`BlockType::byte`), so no pass
/// hardcodes an opaque `0x7E`. Phase 1/2a only needs `I64` (Int) and `I32` (Bool); `F64` is here for
/// the float phase so its arm is a serialize change alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValType {
    /// 64-bit integer — an `Int` value.
    I64,
    /// 32-bit integer — a `Bool` value (and, later, a heap handle).
    I32,
    /// 64-bit float — a `Float` value.
    #[allow(dead_code)]
    F64,
}

impl ValType {
    /// The wasm encoding byte for this value type. THIS is the single place the raw byte lives —
    /// every pass above `serialize` speaks `ValType`, not bytes.
    pub fn byte(self) -> u8 {
        match self {
            ValType::I64 => 0x7E,
            ValType::I32 => 0x7F,
            ValType::F64 => 0x7C,
        }
    }
}

/// The type of a wasm structured block (`if`/`block`/`loop`): either it leaves no value (`Empty`) or
/// exactly one value of a given `ValType`. Used by `Lir::If` so a conditional's result type is
/// stated as a type, not an opaque block-type byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    /// Leaves no value on the stack (a stack-neutral guard block).
    Empty,
    /// Leaves one value of this type (a value-producing `if`).
    Val(ValType),
}

impl BlockType {
    /// The wasm block-type encoding byte: `0x40` for empty, else the value type's byte.
    pub fn byte(self) -> u8 {
        match self {
            BlockType::Empty => 0x40,
            BlockType::Val(vt) => vt.byte(),
        }
    }
}

/// A binary arithmetic operator (Int64 → Int64, checked: traps on overflow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
}

/// A binary bitwise / division operator (Int64 → Int64). `And`/`Or`/`Xor` are total raw ops;
/// `Div`/`Rem` are `i64.div_s`/`i64.rem_s`, which TRAP natively on divide-by-zero and on
/// `Int64.min / -1` (numeric-model.md #Overflow Is Defined — a defined trap, not wrap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitOp {
    And,
    Or,
    Xor,
    Div,
    Rem,
}

/// A shift operator (`<<`/`>>`). The shift COUNT is guarded: an out-of-range count (`>= 64`, or
/// negative — a huge unsigned) traps; a left shift also traps on overflow (a bit shifted past the
/// sign), per numeric-model.md #Overflow Is Defined. Emitted as a guarded run over scratch locals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftOp {
    Left,
    Right,
}

/// A comparison operator (both operands one ordered type → Bool).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
}

/// A built-in operation — the first-class VALUE a prelude record field holds (`Int64.wrapping-add` is
/// `Intrinsic(WrappingAdd)`). An intrinsic is projected by ordinary member access and APPLIED like any
/// function value; the translation to wasm instructions happens ONLY at `select` (`emit_intrinsic`), so
/// HIR/MIR carry the intrinsic as an opaque value — LIR is where it becomes opcodes. This is the ONE
/// place a built-in operation is named; a new op is a variant here + a `signature`/`fold_const`/
/// `emit_intrinsic` arm, never a scattered head-name special-case. Intrinsics are NOT all integer —
/// each op declares its own parameter/result types (`signature`) and folds its own value shapes
/// (`fold_const`), so a future `Bool`/`String`/`Bytes` op is a self-contained arm, not a change to a
/// shared integer assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intrinsic {
    /// `Int64.wrapping-add` — two's-complement addition, wraps on overflow (never traps).
    WrappingAdd,
    /// `Int64.wrapping-sub` — two's-complement subtraction, wraps on overflow.
    WrappingSub,
    /// `Int64.wrapping-mul` — two's-complement multiplication, wraps on overflow.
    WrappingMul,
    /// `Bytes.of` — build a byte sequence from a `List Int` of byte values (each range-checked 0..=255
    /// at run time — a value out of range TRAPS). `List Int → Bytes`.
    BytesOf,
    /// `Bytes.len` — the byte count of a `Bytes`. `Bytes → Int`.
    BytesLen,
    /// `Bytes.concat` — append two byte sequences. `Bytes → Bytes → Bytes`.
    BytesConcat,
    /// `Int.to-byte` — truncate an Int64 to its low 8 bits (0..=255, wrapping / two's-complement).
    /// `Int → Int`. The byte a LEB128/section encoder emits. (`= n & 255`.)
    IntToByte,
    /// `List.len` — the element count of a `List a`. PARAMETRIC: `List a → Int` (the `a` is a fresh var
    /// per use, via a `Ty::Param(0)` template — the same instantiation `Ctor` uses). `vec-len`.
    ListLen,
    /// `List.concat` — append two lists. `List a → List a → List a`. `vec-concat`.
    ListConcat,
    /// `Bytes.at` — the byte at an index, FALLIBLE: `Bytes → Int → Option Int` (`(Some byte)` in bounds,
    /// `(None unit)` out of bounds or negative). `vec`/`bytes-get` under a bounds guard, built as a sum.
    BytesAt,
    /// `List.at` — the element at an index, FALLIBLE: `List a → Int → Option a`. PARAMETRIC over `a`.
    ListAt,
    /// `Bytes.slice` — the `length` bytes of a Bytes at `start`, FALLIBLE: `Bytes → Int → Int → Option
    /// Bytes` (`None` if `start`/`length` negative or `start+length > total`). `bytes-slice` under a
    /// bounds guard.
    BytesSlice,
    /// `String.from-bytes` — decode a `Bytes` as UTF-8, TOTAL (never traps): `Bytes → Option String`.
    /// `(Some s)` for well-formed input, `(None unit)` for ill-formed (invalid lead, overlong form,
    /// surrogate, or code point > U+10FFFF — strict UTF-8, matching `str::from_utf8`). Lowers to a call
    /// to the fixed compiler-emitted `utf8-valid` helper (a byte loop — the `putu`/`itoa` precedent),
    /// then `sum-new(Some, buf)` / `sum-new(None, unit)`. The String's payload IS the validated Bytes leaf.
    StrFromBytes,
    /// `Bytes.compact` — a Bytes equal by content whose storage is INDEPENDENT of any parent buffer (a
    /// slice re-based to release the parent). `Bytes → Bytes`, observationally the identity. `bytes-compact`.
    BytesCompact,
    /// `Map.size` — the entry count (O(1)). `Map K V → Int`. `map-size`. (Its arg is a map handle — no
    /// box.) The empty map is the LITERAL `(map)` (`Mir::Map`), not an intrinsic.
    MapSize,
    /// `Set.size` / `Set.len` — the element count (O(1)). `Set E → Int`. `set-size`. (Arg is a handle.)
    SetSize,
    /// `Set.union` / `Set.intersection` / `Set.difference` — the set algebra. `Set E → Set E → Set E`
    /// (parametric in 1). `set-union`/`set-intersection`/`set-difference`. (Both args are set handles.)
    SetUnion,
    SetIntersection,
    SetDifference,
    /// `List : Type → Type` — the parametric list type constructor as a first-class compile-time
    /// value. Applied to a type-value it folds to `TypeVal(Ty::List(elem))`. Layer 2.
    TypeList,
    /// `Map : Type → Type → Type` — key type, value type → `TypeVal(Ty::Map(k, v))`.
    TypeMap,
    /// `Set : Type → Type` — element type → `TypeVal(Ty::Set(elem))`.
    TypeSet,
    /// `Tuple : Type → Type → Type` — 2-arity for L2 → `TypeVal(Ty::Tuple([a, b]))`.
    TypeTuple,
    /// A built-in whose lowering BOXES a scalar argument (a list element / map key+value / set element)
    /// or unboxes by type — carried as a [`HeapIntrinsic`]. `lower` turns its application into a
    /// [`Mir::HeapOp`] that threads each argument's SOLVED type to `select`; the bare `Mir` shape cannot
    /// decide the box (a runtime `Local` could be an Int to box OR a heap handle). Nesting the boxing ops
    /// under ONE variant keeps them a precise closed set (a `Mir::HeapOp` names exactly these), so no
    /// scattered predicate decides "is this a heap op".
    Heap(HeapIntrinsic),
}

/// A built-in op whose lowering must BOX a scalar argument (a list element / map key+value / set
/// element) or unbox by type. Carried on [`Intrinsic::Heap`] and in [`Mir::HeapOp`] with the args'
/// solved types. A SEPARATE enum so the boxing set is closed and precise (a `Mir::HeapOp` cannot name a
/// non-boxing op like `WrappingAdd`); its `signature`/`param_count` delegate through `Intrinsic::Heap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapIntrinsic {
    /// `List.push : List a → a → List a` — `vec-push`; box a scalar element.
    ListPush,
    /// `Map.insert : Map k v → k → v → Map k v` — `map-insert`; box scalar key + value.
    MapInsert,
    /// `Map.lookup : Map k v → k → Option v` — `map-lookup` (NULL when absent → `Option`); box scalar key.
    MapLookup,
    /// `Map.remove : Map k v → k → Map k v` — `map-remove`; box scalar key.
    MapRemove,
    /// `Set.of : List e → Set e` — `set-empty` + `set-insert` per element; box scalar elements.
    SetOf,
    /// `Set.insert : Set e → e → Set e` — `set-insert`; box a scalar element.
    SetInsert,
    /// `Set.contains : Set e → e → Bool` — `set-contains`; box a scalar element for the probe.
    SetContains,
    /// `Set.remove : Set e → e → Set e` — `set-remove`; box a scalar element.
    SetRemove,
}

impl Intrinsic {
    /// How many `Ty::Param` type parameters this intrinsic's signature is generic over (0 for a
    /// monomorphic op). The `Hir::Intrinsic` site instantiates that many fresh vars and substitutes them
    /// into the signature templates — exactly as a `Ctor` instantiates its sum's params. A polymorphic
    /// op (`List.len : ∀a. List a → Int`) uses `Ty::Param(0)` for `a`.
    pub fn param_count(self) -> usize {
        match self {
            Intrinsic::WrappingAdd | Intrinsic::WrappingSub | Intrinsic::WrappingMul
            | Intrinsic::BytesOf | Intrinsic::BytesLen | Intrinsic::BytesConcat | Intrinsic::IntToByte
            | Intrinsic::BytesAt | Intrinsic::BytesSlice | Intrinsic::StrFromBytes
            | Intrinsic::BytesCompact
            // Layer 2 type builders are MONOMORPHIC (their signature is `Fn` over concrete `Ty::Type`, no
            // `Ty::Param`), so they instantiate no fresh vars.
            | Intrinsic::TypeList | Intrinsic::TypeMap | Intrinsic::TypeSet
            | Intrinsic::TypeTuple => 0,
            Intrinsic::ListLen | Intrinsic::ListConcat | Intrinsic::ListAt => 1,
            // Set ops are parametric in ONE element type `E` = `Ty::Param(0)`.
            Intrinsic::SetSize | Intrinsic::SetUnion | Intrinsic::SetIntersection
            | Intrinsic::SetDifference => 1,
            // Map ops are parametric in TWO types: key `K` = `Ty::Param(0)`, value `V` = `Ty::Param(1)`.
            Intrinsic::MapSize => 2,
            // A boxing op delegates to its `HeapIntrinsic` form.
            Intrinsic::Heap(h) => h.param_count(),
        }
    }
}

impl HeapIntrinsic {
    /// Type parameters this heap op is generic over (its `Ty::Param` count) — same contract as
    /// [`Intrinsic::param_count`]. `List`/`Set` ops are parametric in 1 (`Param(0)`); `Map` ops in 2.
    pub fn param_count(self) -> usize {
        match self {
            HeapIntrinsic::ListPush
            | HeapIntrinsic::SetOf | HeapIntrinsic::SetInsert | HeapIntrinsic::SetContains
            | HeapIntrinsic::SetRemove => 1,
            HeapIntrinsic::MapInsert | HeapIntrinsic::MapLookup | HeapIntrinsic::MapRemove => 2,
        }
    }

    /// This heap op's type signature (param types → result), over `Ty::Param` templates — same contract
    /// as [`Intrinsic::signature`], read by inference. `Option`/`Bool` results use the shared prelude sum.
    pub fn signature(self) -> (Vec<Ty>, Ty) {
        let param = |i| Ty::Param(i);
        match self {
            HeapIntrinsic::ListPush => (
                vec![Ty::List(Box::new(param(0))), param(0)],
                Ty::List(Box::new(param(0))),
            ),
            HeapIntrinsic::MapInsert => (
                vec![Ty::Map(Box::new(param(0)), Box::new(param(1))), param(0), param(1)],
                Ty::Map(Box::new(param(0)), Box::new(param(1))),
            ),
            HeapIntrinsic::MapLookup => (
                vec![Ty::Map(Box::new(param(0)), Box::new(param(1))), param(0)],
                Ty::Sum { def: crate::ty::prelude_option(), args: vec![param(1)] },
            ),
            HeapIntrinsic::MapRemove => (
                vec![Ty::Map(Box::new(param(0)), Box::new(param(1))), param(0)],
                Ty::Map(Box::new(param(0)), Box::new(param(1))),
            ),
            HeapIntrinsic::SetOf => (vec![Ty::List(Box::new(param(0)))], Ty::Set(Box::new(param(0)))),
            HeapIntrinsic::SetInsert => (
                vec![Ty::Set(Box::new(param(0))), param(0)],
                Ty::Set(Box::new(param(0))),
            ),
            HeapIntrinsic::SetContains => (
                vec![Ty::Set(Box::new(param(0))), param(0)],
                Ty::Bool,
            ),
            HeapIntrinsic::SetRemove => (
                vec![Ty::Set(Box::new(param(0))), param(0)],
                Ty::Set(Box::new(param(0))),
            ),
        }
    }
}

impl Intrinsic {
    /// The intrinsic's type signature — its parameter types → result type. Read by inference to type an
    /// application; the whole first-class contract of the operation. Each op names its own types.
    pub fn signature(self) -> (Vec<Ty>, Ty) {
        match self {
            // Int64 → Int64 → Int64.
            Intrinsic::WrappingAdd | Intrinsic::WrappingSub | Intrinsic::WrappingMul => {
                (vec![Ty::Int, Ty::Int], Ty::Int)
            }
            // Bytes ops (each names its own shapes — intrinsics are NOT all-integer).
            Intrinsic::BytesOf => (vec![Ty::List(Box::new(Ty::Int))], Ty::Bytes),
            Intrinsic::BytesLen => (vec![Ty::Bytes], Ty::Int),
            Intrinsic::BytesConcat => (vec![Ty::Bytes, Ty::Bytes], Ty::Bytes),
            Intrinsic::IntToByte => (vec![Ty::Int], Ty::Int),
            // PARAMETRIC over `a` = `Ty::Param(0)` (instantiated fresh per use at the `Hir::Intrinsic`
            // site, `param_count`=1). `List.len : List a → Int`; `concat` preserves `List a`. (`push` is
            // a `Heap` op — its signature is on `HeapIntrinsic`.)
            Intrinsic::ListLen => (vec![Ty::List(Box::new(Ty::Param(0)))], Ty::Int),
            Intrinsic::ListConcat => (
                vec![Ty::List(Box::new(Ty::Param(0))), Ty::List(Box::new(Ty::Param(0)))],
                Ty::List(Box::new(Ty::Param(0))),
            ),
            // FALLIBLE reads → `Option elem` (the SHARED prelude `Option`, so a match on the result
            // unifies by `Arc::ptr_eq`). `Bytes.at : Bytes → Int → Option Int`;
            // `List.at : List a → Int → Option a` (parametric).
            Intrinsic::BytesAt => (
                vec![Ty::Bytes, Ty::Int],
                Ty::Sum { def: crate::ty::prelude_option(), args: vec![Ty::Int] },
            ),
            Intrinsic::ListAt => (
                vec![Ty::List(Box::new(Ty::Param(0))), Ty::Int],
                Ty::Sum { def: crate::ty::prelude_option(), args: vec![Ty::Param(0)] },
            ),
            // `Bytes.slice : Bytes → Int → Int → Option Bytes`.
            Intrinsic::BytesSlice => (
                vec![Ty::Bytes, Ty::Int, Ty::Int],
                Ty::Sum { def: crate::ty::prelude_option(), args: vec![Ty::Bytes] },
            ),
            // `String.from-bytes : Bytes → Option String` — the TOTAL UTF-8 decode.
            Intrinsic::StrFromBytes => (
                vec![Ty::Bytes],
                Ty::Sum { def: crate::ty::prelude_option(), args: vec![Ty::String] },
            ),
            // `Bytes.compact : Bytes → Bytes` (content-identity, storage-independent).
            Intrinsic::BytesCompact => (vec![Ty::Bytes], Ty::Bytes),
            // `Map K V` non-boxing ops — parametric in K=`Param(0)`, V=`Param(1)`.
            Intrinsic::MapSize => (
                vec![Ty::Map(Box::new(Ty::Param(0)), Box::new(Ty::Param(1)))],
                Ty::Int,
            ),
            // `Set E` non-boxing ops — parametric in E=`Param(0)`. Size + the set algebra.
            Intrinsic::SetSize => (vec![Ty::Set(Box::new(Ty::Param(0)))], Ty::Int),
            Intrinsic::SetUnion | Intrinsic::SetIntersection | Intrinsic::SetDifference => (
                vec![Ty::Set(Box::new(Ty::Param(0))), Ty::Set(Box::new(Ty::Param(0)))],
                Ty::Set(Box::new(Ty::Param(0))),
            ),
            // Layer 2 type builders: monomorphic `Type(s) → Type`. Applied to type-value args, their
            // `fold_const` builds the compound `Ty`. `List`/`Set`/`Option` take one type; `Map`/`Result`/
            // `Tuple` take two.
            Intrinsic::TypeList | Intrinsic::TypeSet =>
                (vec![Ty::Type], Ty::Type),
            Intrinsic::TypeMap | Intrinsic::TypeTuple =>
                (vec![Ty::Type, Ty::Type], Ty::Type),
            // A boxing op delegates to its `HeapIntrinsic` signature.
            Intrinsic::Heap(h) => h.signature(),
        }
    }

    /// Build the compound `Ty` a type-builder intrinsic produces from its type arguments, or `None`
    /// (arity mismatch / not a type builder). The ONE compound-Ty constructor — called by both
    /// `fold_const` (fold time, wrapping in `Mir::TypeVal`) and `infer::extract_type_value`
    /// (annotation time, before fold) so the two paths cannot drift. Option/Result route through the
    /// process-global `ty::prelude_option()`/`prelude_result()` singletons so identity is `Arc::ptr_eq`.
    pub fn build_compound_ty(self, arg_tys: &[Ty]) -> Option<Ty> {
        match (self, arg_tys) {
            (Intrinsic::TypeList, [e]) => Some(Ty::List(Box::new(e.clone()))),
            (Intrinsic::TypeSet, [e]) => Some(Ty::Set(Box::new(e.clone()))),
            (Intrinsic::TypeMap, [k, v]) => Some(Ty::Map(Box::new(k.clone()), Box::new(v.clone()))),
            (Intrinsic::TypeTuple, [a, b]) => Some(Ty::Tuple(vec![a.clone(), b.clone()])),
            _ => None,
        }
    }

    /// Constant-fold the intrinsic over already-constant arguments (`Mir` value nodes — `Int`/`Bool`/
    /// `Unit`/`Tuple`), returning the constant result value, or `None` when it does not fold (wrong arg
    /// shapes, or a result that is not a compile-time constant — e.g. a heap `String`/`Bytes` op, which
    /// stays for `select`). Each op MATCHES the value shapes it expects, so a non-integer op is a
    /// self-contained arm — no shared "everything is i64" assumption. A wrapping op always folds (it
    /// never traps), matching its runtime lowering exactly.
    pub fn fold_const(self, args: &[Mir]) -> Option<Mir> {
        match self {
            Intrinsic::WrappingAdd | Intrinsic::WrappingSub | Intrinsic::WrappingMul => {
                // Int → Int → Int. Fold only two integer constants.
                if let [Mir::Int(a), Mir::Int(b)] = args {
                    let r = match self {
                        Intrinsic::WrappingAdd => a.wrapping_add(*b),
                        Intrinsic::WrappingSub => a.wrapping_sub(*b),
                        Intrinsic::WrappingMul => a.wrapping_mul(*b),
                        _ => unreachable!(),
                    };
                    Some(Mir::Int(r))
                } else {
                    None
                }
            }
            // Bytes ops build/read a heap value — never a compile-time constant; they stay for `select`.
            Intrinsic::BytesOf | Intrinsic::BytesLen | Intrinsic::BytesConcat => None,
            // `Int.to-byte` = low 8 bits (wrapping). Folds a constant.
            Intrinsic::IntToByte => {
                if let [Mir::Int(n)] = args {
                    Some(Mir::Int(n & 255))
                } else {
                    None
                }
            }
            // List ops build/read a heap value — never a compile-time constant; they stay for `select`.
            Intrinsic::ListLen | Intrinsic::ListConcat => None,
            // Fallible reads build a heap `Option` sum at run time — never fold.
            Intrinsic::BytesAt | Intrinsic::ListAt | Intrinsic::BytesSlice => None,
            // Decodes a heap `Bytes` to a heap `Option String` at run time — never a compile-time
            // constant (the operand is a Bytes leaf, not a `Mir` literal); stays for `select`.
            Intrinsic::StrFromBytes => None,
            // Map/Set/compact ops all build/read a heap CHAMP value (or a Bytes leaf) at run time —
            // never a compile-time constant; they stay for `select`.
            Intrinsic::BytesCompact
            | Intrinsic::MapSize
            | Intrinsic::SetSize | Intrinsic::SetUnion | Intrinsic::SetIntersection
            | Intrinsic::SetDifference => None,
            // Layer 2 type builders: read the `Ty` out of each `Mir::TypeVal` arg and construct the
            // compound type-value. The one place a compound `Ty` is fabricated from type args at fold
            // time; shared with annotation-time extraction via `build_compound_ty` so the two paths
            // cannot drift. If not all args are `TypeVal` (which never happens in well-typed code —
            // mixing a type-value into a value op is a `Ty` mismatch caught at infer), stays residual.
            Intrinsic::TypeList | Intrinsic::TypeMap | Intrinsic::TypeSet
            | Intrinsic::TypeTuple => {
                let arg_tys: Vec<Ty> = args.iter()
                    .filter_map(|a| match a { Mir::TypeVal(t) => Some(t.clone()), _ => None })
                    .collect();
                if arg_tys.len() != args.len() { return None; } // not all TypeVals → stays residual
                self.build_compound_ty(&arg_tys).map(Mir::TypeVal)
            }
            // A boxing heap op builds/reads a CHAMP/vec value at run time — never a const; stays for `select`.
            Intrinsic::Heap(_) => None,
        }
    }
}

// A match PATTERN is not a separate type — it is an ordinary `Hir`/`Typed`/`Mir` expression with two
// extra leaf meanings in pattern position: a `Local` is a BINDING occurrence (binds that position),
// and `Wildcard` matches-and-binds-nothing. `(Some x)` is `Apply{Ctor, [Local]}`, `(None _)` is
// `Apply{Ctor, [Wildcard]}`, `(tuple a b)` is `Tuple([Local, Local])`, a literal matches by equality.
// So a pattern lowers EXACTLY like the equivalent construction expression (`Apply(Ctor)` → `Mir::Sum`),
// and the MATCH interpreter at `select` reads the lowered pattern-tree (`Sum`/`Tuple`/`Int`/`Bool`/
// `Local`/`Wildcard`) to emit disc-compares + payload binds. Resolve does NO lookup — a pattern head
// resolves as an ordinary expression (`(. Sign Pos)` → a `RecordProj` the fold reduces to a `Ctor`).

// ─── Hir — resolved, desugared ───────────────────────────────────────────────────────

/// A module export: the public function it exposes, by NAME and by index. There is no separate "ABI
/// kind" — an export's ABI IS the exported function's SIGNATURE (`params: Vec<Ty>, ret: Ty` on its
/// `TypedFunc`), read generically at serialization to derive the component surface. Nothing is
/// inferred from the function's name or body shape (no `main`/`compile` magic, no output sniffing);
/// the signature is the whole contract. Visibility is explicit (modules-and-namespaces.md §Visibility
/// Is Explicit): a definition is exposed iff an `(export …)` names it — never by position or name.
#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    pub name: String,
    pub func: usize,
}

/// A resolved module: its functions in definition order, and its EXPORTS (the public items named by
/// `(export …)` forms). Value-defs `(def k v)` are desugared to nullary functions and referenced by
/// call.
#[derive(Debug, Clone, PartialEq)]
pub struct HirModule {
    pub funcs: Vec<HirFunc>,
    pub exports: Vec<Export>,
}

/// A resolved function: its arity (parameters bind locals `0..arity`) and body. The body references
/// a parameter by `Local(i)` for `i < arity` and a `let`-local by `Local(i)` for `i >= arity`.
#[derive(Debug, Clone, PartialEq)]
pub struct HirFunc {
    pub name: String,
    pub arity: usize,
    pub body: Hir,
}

/// Hir — the resolved, desugared high-level IR of a function body. Names are resolved: a parameter
/// or `let`-local is `Local(id)`; a call names the callee by its function index.
#[derive(Debug, Clone, PartialEq)]
pub enum Hir {
    Int(i64),
    Bool(bool),
    /// A string literal — its NFC text. Built at run time as a Bytes-backed UTF-8 heap leaf (the reader
    /// already normalized it). Type `Ty::String`.
    Str(String),
    /// The unit value — `unit` or the empty tuple `()` (core-semantics.md §The Empty Tuple Is The
    /// Unit Value: they are the same value). Occupies no machine slot; a unit-returning function has
    /// no wasm result.
    Unit,
    /// A parameter or `let`-bound local, by the id `resolve` assigned it.
    Local(u32),
    /// A call to module function `func` with `args`.
    Call { func: usize, args: Vec<Hir> },
    /// A top-level/module function AS A VALUE — a reference to function `func` (its module-function
    /// index). A module field holding a function is a `FuncRef`; projecting it yields the reference, and
    /// applying it (`Apply`) resolves to a direct `Call`. Not a closure (a closed function captures no
    /// environment) and not yet a runtime value — the fold reduces every application to a `Call`.
    FuncRef(usize),
    /// A built-in operation AS A VALUE — a prelude record field (`Int64.wrapping-add`). Projected by
    /// member access, applied like a function value (`Apply`); `select` lowers an applied intrinsic to
    /// wasm instructions.
    Intrinsic(Intrinsic),
    /// A sum CONSTRUCTOR as a first-class single-arity function VALUE (`Some`, `None`, `Sign.Pos`) —
    /// `def`'s variant `index`. Bare in value position it IS the constructor (the prelude binds
    /// Constructor values, not pre-applied sums); applied (`Apply`) it constructs the tagged variant.
    /// Transient like `FuncRef`/`Intrinsic` — the fold reduces every application to a `Mir::Sum`.
    Ctor { def: SumRef, index: usize },
    /// `_` in PATTERN position — matches anything and binds nothing. Distinct from a `Local` binder (a
    /// binding occurrence) so a match interpreter binds nothing and placement is checkable (a `let`
    /// cannot bind a wildcard). Never appears in value position.
    Wildcard,
    /// `(match scrutinee (pattern body)…)` — dispatch on the scrutinee's shape. Each `pattern` is an
    /// ordinary `Hir` (a `Ctor`-application / `Tuple` / literal / `Local` binder / `Wildcard`). Arms in
    /// source order; the first matching pattern's body is the value. Exhaustiveness (CDZ0210) at infer.
    Match { scrutinee: Box<Hir>, arms: Vec<(Hir, Hir)> },
    /// A type-value — `Int64`/`Bool`/`String`/`Bytes`/`Unit` as a compile-time VALUE (not a type
    /// annotation). Bare `Int64` resolves to `TypeVal(Ty::Int)` typed as `Ty::Type`. Used in `(: e T)`:
    /// the annotation infers `T` and extracts its `Ty` to unify with `e`. Compile-time-only: a `TypeVal`
    /// that survives fold is the erasure fence reject (CDZ0305).
    TypeVal(Ty),
    /// `(: e T)` — annotate `e` with type `T`. Infer constrains/checks: `T` must type as `Ty::Type`,
    /// its represented `Ty` unifies with `e`'s type (mismatch → CDZ0203). Lowers to just `e`
    /// (transparent — the constraint already happened). `(: 42 Int64)`→42, `(: 42 Bool)`→CDZ0203.
    Annot(Box<Hir>, Box<Hir>),
    /// `(const e)` — assert `e` fully compile-time-reduces. Infer types `e`, fold reduces it; reject
    /// (the erasure fence code) if not a fully-ground compile-time value. The user-facing surface of
    /// "Compile-Time Evaluation Is One Tier" — an explicit const assertion. A type-position (a sum-decl
    /// payload) is an implicit const context. Lowers to just `fold(e)`.
    Const(Box<Hir>),
    /// A RUNTIME trap — an explicit divergence (`unreachable`), carrying an (unobserved) message. Its
    /// type is Never — it unifies with any sibling (a fresh var at inference), so a trapping match arm
    /// takes the arm's result type. `Option.expect`/`Result.expect` desugar to a match whose absent arm
    /// is a `Trap` (core-semantics.md §Requiring The Value Of An Optional Traps On Absence) — so expect
    /// is NOT a special node, just a match that traps. Distinct from `Error` (a poison / compile-time
    /// diagnostic): a `Trap` is an intended runtime halt, never a build failure.
    Trap(String),
    /// Apply a function-VALUE expression `func` to `args` — distinct from `Call`, whose callee is
    /// already a resolved function index. `(( . m f) x)` reads as `Apply { func: (. m f), args: [x] }`;
    /// the fold reduces `Apply(FuncRef(i), args)` to `Call { func: i, args }`.
    Apply { func: Box<Hir>, args: Vec<Hir> },
    /// `(tuple e0 … en)` — construct a heap tuple from its element expressions.
    Tuple(Vec<Hir>),
    /// `(record (k0 e0) … (kn en))` — construct a heap record. Fields in SOURCE order (sorted by
    /// name during inference/lowering — the canonical value-heap slot order).
    Record(Vec<(String, Hir)>),
    /// `(list e0 … en)` — construct a heap list (a homogeneous `List T`). Unlike a tuple, its length is
    /// not part of its type — every element unifies to the single element type `T`. Backed at run time
    /// by the runtime's persistent sequence (`vec-*`), not the fixed `arr-*` of a tuple/record.
    List(Vec<Hir>),
    /// `(map (k0 v0) … (kn vn))` — construct a heap map (the runtime CHAMP). Entries are `(key, value)`
    /// EXPRESSIONS (a key is a value, not a static name); keys unify to one `K`, values to one `V`. The
    /// empty `(map)` is the empty map. Built via `map-insert` from empty.
    Map(Vec<(Hir, Hir)>),
    /// `(set e0 … en)` — construct a heap set (the runtime CHAMP set). Elements unify to one `E`;
    /// duplicates collapse at run time. The empty `(set)` is the empty set. Built via `set-insert`.
    Set(Vec<Hir>),
    /// `(. t N)` — project element `N` of a tuple (positional, integer key).
    TupleProj(usize, Box<Hir>),
    /// `(. r f)` — project field `f` of a record (named key).
    RecordProj(String, Box<Hir>),
    Arith(ArithOp, Box<Hir>, Box<Hir>),
    Bit(BitOp, Box<Hir>, Box<Hir>),
    Shift(ShiftOp, Box<Hir>, Box<Hir>),
    Cmp(CmpOp, Box<Hir>, Box<Hir>),
    If(Box<Hir>, Box<Hir>, Box<Hir>),
    Let { id: u32, value: Box<Hir>, body: Box<Hir> },
    /// `(fn (p…) body)` — a lambda. `params` are the fresh local ids resolve assigned its parameters
    /// (bound in the body's scope exactly like a function's params bind `Local(0..arity)`). Multi-arity
    /// to match the existing function/`Call` model (NOT curried in the IR — the spec's currying is a
    /// surface semantic realized by multi-arity application + compile-time spine collapse of a completed
    /// partial application into a direct call). A lambda is a transient compile-time value: the `eval`
    /// fold β-reduces `Apply(Lambda, args)` to substitute params into the body; a survivor declines.
    Lambda { params: Vec<u32>, body: Box<Hir> },
    /// An unsupported / rejected form, carrying its diagnostic.
    Error(Reject),
}

// ─── typed-Hir — after inference, every node annotated with its solved Ty ──────────────

/// A typed module: each function typed, plus its exports.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedModule {
    pub funcs: Vec<TypedFunc>,
    pub exports: Vec<Export>,
}

/// A typed function: its parameter types, return type, and typed body.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedFunc {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub body: Typed,
}

/// typed-Hir — `Hir` after inference, every node annotated with its solved `Ty`. Lowering READS
/// `ty`; it never re-derives a type.
#[derive(Debug, Clone, PartialEq)]
pub struct Typed {
    pub node: TypedNode,
    pub ty: Ty,
}

/// The typed counterpart of `Hir` — same shapes, children are `Typed`.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedNode {
    Int(i64),
    Bool(bool),
    Str(String),
    Unit,
    Local(u32),
    Call { func: usize, args: Vec<Typed> },
    FuncRef(usize),
    Intrinsic(Intrinsic),
    /// A sum constructor value (`def`'s variant `index`); its `ty` is the instantiated `Fn([payload],
    /// Sum{def,args})`.
    Ctor { def: SumRef, index: usize },
    /// A type-value — typed as `Ty::Type`. The compile-time value a type name resolves to.
    TypeVal(Ty),
    /// `(: e T)` — typed annotation. The node's `ty` is `e`'s type (unified with `T`'s extracted `Ty`).
    /// Lowers to just `e` (transparent).
    Annot(Box<Typed>, Box<Typed>),
    /// `(const e)` — typed assertion that `e` fully folds. The node's `ty` is `e`'s type. Lowers to
    /// `fold(e)`, then the fence checks it reduced to a const.
    Const(Box<Typed>),
    /// `_` in pattern position (typed to the position's type; binds nothing).
    Wildcard,
    /// `(match …)` — the scrutinee and typed arms; each arm is `(typed-pattern, typed-body)`. The node's
    /// `ty` is the (unified) arm-body result type.
    Match { scrutinee: Box<Typed>, arms: Vec<(Typed, Typed)> },
    /// A runtime trap (Never) — the node's `ty` is whatever it was unified to (its arm's result type).
    Trap(String),
    Apply { func: Box<Typed>, args: Vec<Typed> },
    Tuple(Vec<Typed>),
    /// `(record …)` — typed fields in source order (sorted at lowering to the canonical slot order).
    Record(Vec<(String, Typed)>),
    /// `(list …)` — typed elements, all of the list's single element type (unified during inference).
    List(Vec<Typed>),
    /// `(map …)` — typed `(key, value)` entries; all keys share `K`, all values `V` (unified at infer).
    Map(Vec<(Typed, Typed)>),
    /// `(set …)` — typed elements, all of the set's single element type `E` (unified at infer).
    Set(Vec<Typed>),
    /// `(. t N)` — positional projection: the element index and the tuple operand. (The result type is
    /// the `Typed.ty` on the enclosing node; the operand's type carries the full tuple type.)
    TupleProj(usize, Box<Typed>),
    /// `(. r f)` — named projection: the field name and the record operand (its type carries the field
    /// set, so lowering resolves the name to a slot).
    RecordProj(String, Box<Typed>),
    Arith(ArithOp, Box<Typed>, Box<Typed>),
    Bit(BitOp, Box<Typed>, Box<Typed>),
    Shift(ShiftOp, Box<Typed>, Box<Typed>),
    Cmp(CmpOp, Box<Typed>, Box<Typed>),
    If(Box<Typed>, Box<Typed>, Box<Typed>),
    Let { id: u32, value: Box<Typed>, body: Box<Typed> },
    /// `(fn (p…) body)` — a typed lambda. The node's `ty` is `Ty::Fn(param_tys, ret)`. Transient.
    Lambda { params: Vec<u32>, body: Box<Typed> },
}

// ─── Mir — resolved mid rung ───────────────────────────────────────────────────────────

/// A lowered module: each function lowered, plus its exports.
#[derive(Debug, Clone, PartialEq)]
pub struct MirModule {
    pub funcs: Vec<MirFunc>,
    pub exports: Vec<Export>,
}

/// A lowered function: parameter valtypes, return type, and body.
#[derive(Debug, Clone, PartialEq)]
pub struct MirFunc {
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub body: Mir,
}

/// Mir — the resolved mid rung. Phase 2a mirrors typed-Hir with types resolved to ground `Ty`; the
/// passes that make it distinct (fold/monomorphize) grow later.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum Mir {
    Int(i64),
    Bool(bool),
    /// A string literal — its NFC bytes. `select` builds it as a `bytes-*` heap leaf (like `Bytes.of`
    /// of a literal, no range guard — UTF-8 bytes are 0..=255). A const, but a HEAP one (not scalar).
    Str(String),
    Unit,
    Local(u32),
    Call { func: usize, args: Vec<Mir> },
    /// TRANSIENT — a function value / its application, present only between `lower` and `eval`. The fold
    /// reduces `Apply(FuncRef(i), args)` to a `Call` (β-reducing const args) and folds a module
    /// `Proj`→`FuncRef` away; a `FuncRef`/`Apply` that SURVIVES folding is a function value that cannot
    /// (yet) become runtime code — `select` declines it, never emits. Never reaches serialize.
    FuncRef(usize),
    /// A built-in operation value (transient like `FuncRef`) — folded away when applied (`select`
    /// lowers `Apply(Intrinsic, args)` to wasm); a bare survivor declines.
    Intrinsic(Intrinsic),
    /// A sum constructor VALUE (transient like `FuncRef`/`Intrinsic`) — `def`'s variant `index`. Lowered
    /// from a bare `Ctor`; `lower` turns `Apply(Ctor,arg)` directly into `Mir::Sum`, so a bare `Ctor`
    /// only survives when used as a value then applied (`Apply(Ctor, [unit])` — the bare-`None` case),
    /// which the fold reduces to `Mir::Sum`. A bare survivor declines in `select`.
    Ctor { def: SumRef, index: usize },
    /// A type-value (NON-SERIALIZED) — exists only so the post-fold erasure fence can point at a leaked
    /// type-value in diagnostic. A `TypeVal` that reaches `lower` IS the erasure fence error; it should
    /// never reach `select` or serialize. `Annot` lowers to just its expr; `Const` lowers to `fold(e)`.
    TypeVal(Ty),
    /// Construct a heap SUM value `(disc, payload-handle)` via `sum-new`. `disc` = the variant's
    /// declaration index; `payload_ty` types the payload for boxing (Unit → the empty-`arr` unit
    /// payload). Distinct from `Tuple`/`List` — a two-field (tag, payload) heap object.
    Sum { def: SumRef, disc: u32, payload_ty: Ty, payload: Box<Mir> },
    /// `_` in a lowered pattern — matches anything, binds nothing.
    Wildcard,
    /// `(match …)` lowered — the scrutinee, its solved type, the arms (each a `(lowered-pattern,
    /// body)`), and the result type. Each pattern is a `Mir` tree (`Sum{disc,payload}` / `Tuple` /
    /// `Int`/`Bool` literal / `Local` binder / `Wildcard`) — `select` walks it to emit disc-compares +
    /// payload binds; the fold selects a known-disc arm. `scrut_ty` carries the `Arc<SumDef>`.
    Match { scrutinee: Box<Mir>, scrut_ty: Ty, arms: Vec<(Mir, Mir)>, ty: Ty },
    /// A runtime trap — `select` emits `unreachable`. The intended runtime halt for a match's absent
    /// arm (e.g. an `expect` desugar). NOT a poison — it never fails the build.
    Trap(String),
    Apply { func: Box<Mir>, args: Vec<Mir> },
    /// Construct a heap product — a tuple, OR a record whose fields are already sorted into slot order.
    /// Each element carries its type (to box it correctly). The named/positional distinction is gone by
    /// this rung: a record is just a product whose slot order was fixed (by field-name sort) at
    /// lowering, so construction is identical to a tuple's.
    Tuple(Vec<(Ty, Mir)>),
    /// Construct a heap LIST — a homogeneous sequence built on the runtime's persistent `vec` (NOT the
    /// fixed `arr` a `Tuple` uses). Each element carries its (shared) type so `select` boxes it by the
    /// right accessor. A distinct variant from `Tuple` because it lowers to `vec-empty`/`vec-push` and
    /// renders `(list …)` — the length is runtime data (grown by `vec-push`), not a fixed arity.
    List(Vec<(Ty, Mir)>),
    /// Construct a heap MAP (`(map …)` literal) on the runtime CHAMP — `map-empty` then `map-insert` per
    /// entry. Each entry carries its key + value SOLVED types so `select` boxes each scalar by the right
    /// accessor (a compound is already a handle). The empty `(map)` is just `map-empty`.
    Map(Vec<((Ty, Mir), (Ty, Mir))>),
    /// Construct a heap SET (`(set …)` literal) on the runtime CHAMP set — `set-empty` then `set-insert`
    /// per element. Each element carries its solved type for boxing. The empty `(set)` is `set-empty`.
    Set(Vec<(Ty, Mir)>),
    /// A [`HeapIntrinsic`] application carrying each argument's SOLVED type — the ops that must BOX a
    /// scalar argument (a list element / map key+value / set element) or unbox by type, which the
    /// argument `Mir`'s shape alone cannot decide (a runtime `Local` could be an Int to box OR a heap
    /// handle). `select` reads `op` + `args[i].0` to emit the right box/guard sequence. Distinct from
    /// `Apply(Intrinsic, …)` (which loses the types at lowering) — `lower` builds this for exactly the
    /// [`HeapIntrinsic`] ops. (`Mir::Sum`/`Mir::Tuple`/`Mir::List` carry solved types the same way.)
    HeapOp { op: HeapIntrinsic, args: Vec<(Ty, Mir)> },
    /// Project the value at `slot` (of type `elem_ty`) from a heap product operand — the ONE projection
    /// for both tuple (`slot` = literal index) and record (`slot` = the field's position in the sorted
    /// field list, resolved at lowering). This is the unified access the record generalization buys:
    /// `select` has ONE arr-get+unbox arm, not one per compound shape.
    Proj { slot: usize, elem_ty: Ty, operand: Box<Mir> },
    Arith(ArithOp, Box<Mir>, Box<Mir>),
    Bit(BitOp, Box<Mir>, Box<Mir>),
    Shift(ShiftOp, Box<Mir>, Box<Mir>),
    /// A comparison, carrying its OPERAND type (`operand_ty`) — both operands share it (inference
    /// unified them). Selection needs it to pick the machine comparison: an `Int` operand uses the
    /// signed i64 ops; a `Bool` operand uses the unsigned i32 ops (false=0 < true=1). The RESULT is
    /// always `Bool`; this is the input type.
    Cmp { op: CmpOp, operand_ty: Ty, a: Box<Mir>, b: Box<Mir> },
    If { cond: Box<Mir>, then_: Box<Mir>, else_: Box<Mir>, ty: Ty },
    Let { id: u32, value_ty: Ty, value: Box<Mir>, body: Box<Mir> },
    /// `(fn (p…) body)` — a lowered lambda. TRANSIENT — present only between `lower` and `eval`. The
    /// fold reduces `Apply(Lambda, args)` by β-reduction (substitute params into body, α-renamed); a
    /// `Lambda` that SURVIVES folding is a runtime closure → `select` declines it (Increment B). Sibling
    /// to `FuncRef`/`Ctor`/`Intrinsic` — a compile-time function value.
    Lambda { params: Vec<u32>, body: Box<Mir> },
    Error(Reject),
}

// ─── Lir — flat stock-wasm instruction ─────────────────────────────────────────────────

/// Lir — a single flat stock-wasm instruction. A function body is a `Vec<Lir>` (a sequence over the
/// stack + locals model). `serialize` maps each to bytes by an exhaustive match. The checked-
/// arithmetic overflow guard is a run of these flat instructions over scratch locals — no tree.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum Lir {
    ConstI64(i64),
    ConstI32(i32),
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    /// `call <func-index>` — an ABSOLUTE wasm function index. `select` resolves every callee (user
    /// function OR value-heap import/helper) to its final wasm index up front using the module
    /// `Layout`, so `Lir` is concrete: there is one call instruction and one function-index space
    /// (imports first, then defined funcs), exactly as wasm has. No index is remapped downstream.
    Call(u32),
    /// `drop` — discard the top stack value (e.g. the array handle `arr-set` returns).
    Drop,
    /// i32 arithmetic (heap-pointer / cursor math in the renderer).
    I32Add,
    I32Sub,
    /// `i32.and` — combine two i32 boolean results (the fallible-read bounds guard ANDs two comparisons).
    I32And,
    /// `i32.store` / `i32.store8` (memarg align+offset both 0 here) — the renderer's ret-pair + bytes.
    I32Store,
    I32Store8,
    I64Add,
    I64Sub,
    I64Mul,
    I64DivS,
    I64Xor,
    I64And,
    I64Or,
    I64RemS,
    I64Shl,
    I64ShrS,
    I64GeU,
    I64Eqz,
    I64Eq,
    I64Ne,
    I64LtS,
    I64GtS,
    I64LeS,
    I64GeS,
    I32Eqz,
    /// `i32.wrap_i64` — narrow an i64 to i32 (the Bytes path: a range-checked byte value → the i32
    /// `bytes-set` takes).
    I32WrapI64,
    /// `i64.extend_i32_u` — widen an unsigned i32 to i64 (the Bytes path: `bytes-len` returns an i32
    /// count, extended to the Int64 result).
    I64ExtendI32U,
    /// i32 comparisons for Bool ordering (Bool is i32, false=0/true=1, so its total order is the
    /// UNSIGNED comparison) and Bool equality (`i32.eq`).
    I32Eq,
    I32LtU,
    I32GtU,
    I32LeU,
    I32GeU,
    /// `if <blocktype>` … `else` … `end` — the block's result type stated as a `BlockType`, not a
    /// raw byte (the encoding is `serialize`'s concern).
    If(BlockType),
    Else,
    End,
    Unreachable,
}

/// A rejection: an optional machine-readable [`Code`] plus a human message. A `None` code is an
/// uncoded DECLINE (a not-yet-supported construct); a `Some` code is a real diagnostic classified by
/// the single `Code` taxonomy (never a bare `CDZ####` string — see [`crate::diag`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Reject {
    pub code: Option<Code>,
    pub message: String,
}

impl Reject {
    pub fn decline(message: impl Into<String>) -> Reject {
        Reject { code: None, message: message.into() }
    }
    pub fn coded(code: Code, message: impl Into<String>) -> Reject {
        Reject { code: Some(code), message: message.into() }
    }
}
