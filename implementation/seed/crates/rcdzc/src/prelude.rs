//! The prelude — the ONE map of built-in bindings the resolver consults by name.
//!
//! A built-in is NOT a special value world. A built-in module is *just a record*, and a record is
//! already an ordinary node in the AST arena — so the prelude is installed as real AST nodes appended
//! to the program's arena, and a built-in binding is simply a `StructId` like any program node. There
//! is no `Module` type and no parallel "builtin value" enum: `Int64` binds to a `(record (max …)
//! (min …))` node, and `Int64.max` reaches its field through the EXACT same member-access-and-fold
//! path as a program's `p.x` (`resolve` returns a `Ref` to the prelude record; `type_of`/`core_of`
//! already follow a `Ref` into a record). This is "records everywhere" taken to the representation:
//! nothing is privileged by name (`reference-compiler.md` §Nothing Is Privileged By Name) OR by shape.
//!
//= spec/capabilities/core-semantics.md#a-built-in-module-is-a-record-of-its-operations
//# A built-in module — a collection of operations the language provides rather than a program defines — MUST be a record whose fields name those operations, indistinguishable in form from a module a program defines.
//!
//= spec/capabilities/core-semantics.md#a-built-in-module-is-a-record-of-its-operations
//# A built-in module and a program-defined module MUST be accessed by the identical mechanism; the language MUST NOT recognize a built-in module's name in any position a program-defined module's name would not be recognized.
//!
//! Resolving a name is one ordered lookup — the lexical scope, then this map (`prelude-and-
//! resolution.md` §The Prelude Is A Single Map The Resolver Consults By Name Alone). A program binding
//! shadows a built-in of the same name because `resolve` searches the scope FIRST — no special case:
//!
//= spec/capabilities/core-semantics.md#a-built-in-module-is-a-record-of-its-operations
//# A reference to a built-in module's name MUST resolve to that record under the same scope and shadowing rules as any other binding, and an operation MUST be reached by member access on that record, so that projecting a built-in operation (`Mod.op` denoting `(.
//!
//! No open-vs-closed rule. A built-in module carries EVERY field it will ever have; a field for an
//! operation not yet realized is filled with an `(unrealized …)` node that resolves to a DECLINE. So
//! projecting an unrealized field declines through the ordinary member-access-then-poison path — the
//! same CLOSED record projection every user record takes — and there is no "the module is open"
//! branch anywhere. An unimplemented built-in is a capability the compiler lacks (a decline), reached
//! by exactly the mechanism a program record uses.
//!
//! What is realized on an integer-width module: `max`/`min` (folding bound constants), `wrap` (the
//! truncating conversion). What remains an `unrealized` field, declining cleanly rather than reading
//! as unbound: `of` (the checked conversion — it returns `Option<T>`, so it waits on sum types) and
//! `checked-*`/`wrapping-*`. The binary arithmetic and comparison operators (`+ - * / …`, `< = …`)
//! are realized as top-level prelude operators, not module fields.

use crate::ast::{Arenas, CompoundCtor, IntValue, Leaf, LeafId, Radix, Struct, StructId};
use std::collections::BTreeMap;

/// Install the prelude into `ast`, appending its built-in bindings as ordinary AST nodes and
/// returning the `name → node` map. The appended nodes take `StructId`s AFTER the program's, so no
/// program id shifts (byte identity of program-derived facts is preserved). Deterministic: the
/// prelude is a fixed function of nothing.
pub fn install(ast: &mut Arenas) -> BTreeMap<String, StructId> {
    let mut names = BTreeMap::new();

    // Ground types — a record whose META channel `(meta t)` holds the type-value. Using `Bool` in
    // type position projects `(meta t)`; it is not applyable (no `(meta apply)`).
    names.insert("Bool".to_string(), ground_type_record(ast, "Bool"));
    names.insert("Unit".to_string(), ground_type_record(ast, "Unit"));
    // `BigInt` — the arbitrary-precision integer (a NULLARY type like `String`/`Symbol`). The module
    // record carries `(meta t) = (intrinsic "BigInt")` (so `BigInt` in type position reduces to
    // `Ty::BigInt`) PLUS the `of` widening conversion (`∀a. (Int a) → BigInt`, B1 — constant-folds; a
    // runtime source declines until the runtime limb ops). Arithmetic + the reverse checked narrowing
    // arrive in later increments.
    names.insert("BigInt".to_string(), bigint_module(ast));

    // `Rational` — the exact-rational type (a NULLARY type like `BigInt`). The module record carries
    // `(meta t) = (intrinsic "Rational")` (so `Rational` in type position reduces to `Ty::Rational`) PLUS
    // the `of`/`of-int`/`value` construction/conversion fields (B4-1 — constant-folds a normalized pair; a
    // runtime operand declines until the runtime rational compound). Arithmetic (`+`/`-`/`*`/`/`) +
    // comparison over rationals are the ordinary operators, dispatched on a `Ty::Rational` operand.
    names.insert("Rational".to_string(), rational_module(ast));

    // `Num` — the number-shape capability namespace. Carries the generic `neg : ∀a. a → a` (backed by
    // `Prim::Neg`); v-inference attaches the `Num` constraint (admit signed-int/float/bigint/rational,
    // reject unsigned at compile time). Grows the full numeric-capability set as the operator scopes it.
    names.insert("Num".to_string(), num_module(ast));

    // The unit VALUE, bound to the bare name `unit` — an alias for the empty list `()`, the other
    // spelling of the same value (core-semantics.md #Unit And The Empty Tuple Are The Same Value;
    // 01-literals "unit and the empty tuple are the same value"). An empty-list node resolves to
    // `Resolved::Unit` exactly as a source `()` does, so `unit` and `()` are interchangeable in value
    // position — this is what lets the pervasive nullary-variant idiom `(None unit)` / `(Sign.Pos unit)`
    // and the direct `(input unit)` case RUN, rather than declining "unbound name `unit`". (`Unit`,
    // capitalized, is the TYPE above; `unit` is the value.)
    names.insert("unit".to_string(), push_list(ast, vec![]));

    // Type constructors — a record whose META channel `(meta apply)` holds the native builder. `(Int
    // a)` / `(-> A B)` are ORDINARY applications: project `(meta apply)`, apply it. `Int`/`UInt` build
    // a width-specialized integer MODULE; `->` builds a function type-value.
    //
    // A generic type constructor (`List`/`Map`/`Set`/`Tuple` below, `Option`/`Result` from the sum
    // synthesis) is a compile-time function from TYPES to a type, applied by the SAME ordinary application
    // as any value — `(List Int64)` projects `(meta apply)` and applies it, no special syntax — so a
    // parameterized type is the RESULT of applying a type constructor. Generics reuse this first-class
    // -type machinery (`Meta.apply` over type-valued arguments), not a separate parametric-polymorphism
    // construct; a type argument is a compile-time value, never runtime data.
    //= spec/capabilities/type-system.md#generics-are-type-valued-parameters-not-a-separate-polymorphism-mechanism
    //# A generic definition MUST be expressed as an ordinary definition that takes type-valued parameters, so that generics reuse the first-class-type machinery rather than introducing a separate parametric-polymorphism construct.
    //= spec/capabilities/type-system.md#generics-are-type-valued-parameters-not-a-separate-polymorphism-mechanism
    //# A generic type constructor — a type parameterized by another type, such as a list of a given element type or an optional of a given type — MUST be a compile-time function from types to a type, applied by ordinary application, so that a parameterized type like an optional integer is the result of applying a type constructor rather than special syntax.
    names.insert("Int".to_string(), ctor_record(ast, "Int"));
    names.insert("UInt".to_string(), ctor_record(ast, "UInt"));
    names.insert("->".to_string(), ctor_record(ast, "->"));
    // `Tuple` — BOTH the tuple-type constructor (`(meta apply)` = the `Tuple` builder: `(Tuple Int64
    // Bool)` builds the tuple type-value, variadic over element types) AND a module of POSITIONAL row
    // operations reached by member access (`(. Tuple cat)`). Same dual shape as `Record`/`List`/`Map`.
    let tuple_mod = tuple_module(ast);
    names.insert("Tuple".to_string(), tuple_mod);
    // `Record` — BOTH the record-type constructor (`(meta apply)` = the `Record` builder: `(Record (a
    // Int64) (b Bool))` builds the record type-value, reading each arg as a `(name type)` pair) AND a
    // module of record ROW OPERATIONS reached by member access (`(. Record project)`). Same dual shape as
    // `List`/`Map` (a type ctor + operation fields on one record).
    let record_mod = record_module(ast);
    names.insert("Record".to_string(), record_mod);

    // The compound-VALUE constructors as SHADOWABLE aliases. The primitive is a symbol head (`(,)`
    // builds a tuple, `{}` builds a record — dispatched structurally in `resolve`), but the ordinary
    // names `tuple`/`record` are prelude records here so `(tuple 1 2)` / `(record (x 1))` written with
    // the NAME are ordinary applications: their `(meta apply)` holds the value-constructor intrinsic,
    // and being ordinary names they are SHADOWABLE (a local `(let ((tuple …)) …)` wins via the
    // scope-first lookup, never reaching this entry). This is what removes the head-vs-value resolution
    // split — the name is looked up, the symbol is the unspellable primitive.
    //
    // The alias is an ORDINARY prelude name (like any built-in module), so it obeys Binding Is Lexical:
    // a program binding named `tuple`/`record` shadows it in scope, and the name resolves identically in
    // head and value position (never recognized as the constructor where a program name would not be).
    //= spec/capabilities/core-semantics.md#a-compound-value-has-a-symbol-constructor-and-a-shadowable-alias
    //# Each such primitive MUST ALSO be reachable through an ordinary **alias name** — `tuple` for `("tuple" …)`, `record` for `("record" …)` — bound in the prelude exactly as any other built-in name, and therefore subject to *Binding Is Lexical* and *A Built-In Module Is A Record Of Its Operations*: a reference to the alias MUST resolve to the nearest enclosing binding of that name.
    //= spec/capabilities/core-semantics.md#a-compound-value-has-a-symbol-constructor-and-a-shadowable-alias
    //# Consequently a program binding named `tuple` or `record` (by `let`, a definition, or a parameter) MUST shadow the built-in alias for the extent of its scope — an application `(tuple a b)` in that scope MUST apply the bound value, not construct a tuple — precisely as a binding named `list` shadows the list constructor.
    //= spec/capabilities/core-semantics.md#a-compound-value-has-a-symbol-constructor-and-a-shadowable-alias
    //# The alias name MUST resolve identically in application-head position and in value position: the language MUST NOT recognize the alias name as the built-in constructor in a position a program-defined name would not be, so that one name never resolves two ways by syntactic position (the resolution split *Binding Is Lexical* forbids).
    names.insert("tuple".to_string(), ctor_record(ast, "tuple-new"));
    names.insert("record".to_string(), ctor_record(ast, "record-new"));
    // `list` — the list-VALUE constructor alias (`(list 1 2 3)`), variadic + homogeneous → `Ty::List`.
    names.insert("list".to_string(), ctor_record(ast, "list-new"));
    // `set` — the SHADOWABLE name alias for set construction (operator ruling 2026-08-31: keep a shadowable
    // name constructor for set as the string-form `("set" …)` is dropped at the M3 reader-flip). `(set 1 2)`
    // reduces (via `set-new`) to the native `#set(1 2)`, so it denotes the same set as the unshadowable
    // ctor-leaf literal — the set companion of the tuple/record/list/map aliases (set uniquely lacked one).
    names.insert("set".to_string(), ctor_record(ast, "set-new"));
    // `map` — the map-VALUE constructor alias (`(map (k v) …)`), whose `(meta apply)` = `Prim::MapNew`.
    // A bare `(map …)` NAME head reduces via this alias (`reduce_ctor` rewrites it to the symbol-headed
    // `("map" …)`, resolved by `resolve_map`), exactly as `list`/`record` do — so `map` is a shadowable
    // prelude name, not a reserved grammar word (the string `"map"` head IS the unshadowable primitive).
    names.insert("map".to_string(), ctor_record(ast, "map-new"));
    // `List` — BOTH the list-TYPE constructor (`(List Int64)` in type position → `(meta apply)=List`) AND
    // the module of list OPERATIONS (its `len`/… fields, reached by member access `(. List len)`). One
    // record carries both roles: applying it builds the type, projecting a field gives an operation.
    names.insert("List".to_string(), list_module(ast));
    // `Map` — BOTH the map-TYPE constructor (`(Map Int64 Int64)` in type position → `(meta apply)=Map`,
    // TWO parameters) AND the module of map OPERATIONS (`empty`/`insert`/`lookup`/`remove`/`size`, reached
    // by member access `(. Map insert)`). One record carries both roles, exactly like `List`.
    names.insert("Map".to_string(), map_module(ast));
    // `Set` — BOTH the set-TYPE constructor (`(Set Int64)` in type position → `(meta apply)=Set`, ONE
    // parameter like `List`) AND the module of set OPERATIONS (`of`/`contains`/`len`/`insert`/`remove`/
    // `union`/`intersection`/`difference`, reached by member access `(. Set of)`). One record, both roles.
    names.insert("Set".to_string(), set_module(ast));
    // `Bytes` — the module of byte-sequence OPERATIONS (`of`/`len` fields, reached by member access
    // `(. Bytes of)`). Unlike `List` it is NOT also a type constructor: `Bytes` is a ground type-VALUE
    // (a non-parametric leaf), so the module ALSO carries `(meta t) = Bytes` — bare `Bytes` in type
    // position IS the type, and `(. Bytes of)` projects the constructor operation.
    names.insert("Bytes".to_string(), bytes_module(ast));

    // `String` — the module of string OPERATIONS (`scalar-len`/`byte-len`, reached by member access `(.
    // String scalar-len)`). Unlike `List`, `String` is a NULLARY type (it takes no parameter), so the
    // module has no `(meta apply)` type-constructor channel — `(: x String)` decodes the bare name
    // directly (`resolve::decode_ty`), and this record only carries the operation fields.
    names.insert("String".to_string(), string_module(ast));

    // ---- Units of measure (the optional, compile-time-only dimensional-analysis layer) ----
    // `Unit` — the ground type `Ty::Unit` (registered above with `(meta t) = Unit`) EXTENDED with the
    // unit-BUILDER fields `one` (the dimensionless group identity) and `base` (a base dimension named by
    // a symbol), reached by member access `(. Unit one)` / `(. Unit base)`. WARNING: `Unit` is BOTH a type (the
    // `unit` value's type) AND the units module — a record carries both a `(meta t)` and member fields
    // (exactly as `Bytes`/`String` do), so the two roles coexist and using `Unit` as a type (`(-> Unit
    // Int64)`) still reduces to `Ty::Unit`. This REPLACES the plain ground-type record inserted above
    // (adding the fields), it does NOT change `Unit`'s type role. A unit is a compile-time VALUE reduced
    // away by `eval` (it indexes `Ty::Qty` and never reaches the backend). `Unit.*`/`Unit./`/`Unit.^` are
    // NOT fields here — the reader leaves them as bare names (`^`/`*`/`/` aren't alphabetic, so `(Unit.*
    // a b)` does not desugar to member access), so they are registered as top-level names below.
    names.insert("Unit".to_string(), unit_module(ast));
    // The unit group OPERATORS as top-level names (the reader keeps `Unit.*`/`Unit./`/`Unit.^` as bare
    // atoms). Each is an operator-shaped record whose `(meta apply)` is the builder prim; they take units
    // and build a unit, reduced by `eval`. No `(meta t)` scheme (a unit is not typed by HM — it is a
    // compile-time value, like a type-constructor argument).
    names.insert("Unit.*".to_string(), unit_op_ctor(ast, "unit-mul"));
    names.insert("Unit./".to_string(), unit_op_ctor(ast, "unit-div"));
    names.insert("Unit.^".to_string(), unit_op_ctor(ast, "unit-pow"));

    // The PREFIXES — each a prelude record carrying its exact scale factor `(num den)` on a `(meta
    // scale)` channel, applied to a unit by `(Unit.prefix P u)`. SI decimal prefixes are powers of ten
    // (a negative power is a `1/10^k` ratio — `milli` = 1/1000, no float/int rounding); IEC binary
    // prefixes are powers of two (`kibi` = 1024, `mebi` = 2²⁰), the distinct scales `information` uses.
    // All fit `i64` with headroom (`tera` = 10¹², `tebi` = 2⁴⁰). Ordinary shadowable names.
    for (name, num, den) in [
        // SI decimal, positive powers.
        ("kilo", 1_000i64, 1i64),
        ("mega", 1_000_000, 1),
        ("giga", 1_000_000_000, 1),
        ("tera", 1_000_000_000_000, 1),
        // SI decimal, negative powers (an exact `1/10^k` ratio).
        ("milli", 1, 1_000),
        ("micro", 1, 1_000_000),
        ("nano", 1, 1_000_000_000),
        ("pico", 1, 1_000_000_000_000),
        // IEC binary (powers of two).
        ("kibi", 1_024, 1),
        ("mebi", 1_048_576, 1),
        ("gibi", 1_073_741_824, 1),
        ("tebi", 1_099_511_627_776, 1),
    ] {
        names.insert(name.to_string(), prefix_record(ast, num, den));
    }

    // `Qty` — the module of QUANTITY operations: `of` (attach a unit to a numeric value) and `value`
    // (recover the numeric value, discarding the unit). Reached by member access `(. Qty of)` / `(. Qty
    // value)`. A `(Qty T u)` is checked then ERASED before emission (units-of-measure.md §Dimensions Are
    // Checked Then Erased), so `Qty.of`/`Qty.value` erase to their value argument's lowering.
    names.insert("Qty".to_string(), qty_module(ast));

    // `Type` — the module of TYPE-REFLECTION operations. `of` reduces `(Type.of e)` to the type-VALUE of
    // `e`'s inferred type, so `(: x (Type.of y))` gives `x` the same type as `y`. Reached by member
    // access `(. Type of)`. A `Type` value is compile-time-only (erased before the boundary), so
    // `Type.of` is a type-level operation, not a runtime one.
    names.insert("Type".to_string(), type_module(ast));

    // `Blake3` — the module of blake3 hashing operations. `of` hashes a `Bytes` to its 32-byte digest as
    // a `Bytes` (`(. Blake3 of)`). NAMES THE ALGORITHM (a future digest is a different named module, not a
    // silent change), and is entirely generic (raw bytes → digest, no domain tag — that is userspace's
    // job). NOT a type — no `(meta t)`. The compile-time half of the contract-primitives blake3 (folds a
    // constant `Bytes` via `blake3::hash`; a runtime `Bytes` awaits the runtime lowering to heap op 91).
    names.insert("Blake3".to_string(), blake3_module(ast));

    // `Char` — the module of char OPERATIONS (`to-int`/`from-int`), a NULLARY type like `String`. Its
    // `(meta t)` is the ground `Ty::Char`, so bare `Char` in type position IS the type; the operation
    // fields project via member access `(. Char to-int)`.
    names.insert("Char".to_string(), char_module(ast));

    // `Symbol` — an interned NAME value with O(1) equality (17-symbols), a nominal over `String`. The
    // module record carries `(meta t) = Ty::Symbol` (so bare `Symbol` in type position IS the type) plus
    // `of`/`to-string` operation fields. Like `Bytes`/`String`/`Char`, a `(meta t)` type-value and member
    // access coexist.
    names.insert("Symbol".to_string(), symbol_module(ast));
    names.insert("Value".to_string(), value_module(ast));

    // The binary ARITHMETIC operators — records whose META channel carries their type (`(meta t)`, a
    // compile-time type-lambda) and their reduction (`(meta apply)`, the intrinsic). `(+ a b)` is the
    // application of the value `+` resolves to — the SAME mechanism every application uses, dispatched
    // by the head's meta channel, never by an operator name the resolver special-cases. Arithmetic,
    // division, shift, and bitwise all share the width-generic `∀a. (Int a) → (Int a) → (Int a)` type.
    // `+`/`-`/`*`/`/` are the ONE arithmetic-operator spelling across EVERY numeric type: the `(Int a)`
    // scheme is the fixed-width-integer case, and `infer`/`lower` route a `Float`/`BigInt`/`Rational`/
    // `Qty` operand to that type's arithmetic by the SOLVED operand type (a dedicated `apply_type` arm +
    // a `lower` dispatch, never an operator-name special-case). Both operands must be ONE numeric type —
    // a mix (`(+ 2 2.0)`) is CDZ0301 from that dispatch, not a coercion (numeric-model.md §An Arithmetic
    // Operator Requires Both Operands To Be One Numeric Type). `%`/`<<`/`>>`/`&`/`|`/`^` are integer-only.
    //= spec/capabilities/numeric-model.md#an-arithmetic-operator-requires-both-operands-to-be-one-numeric-type
    //# An arithmetic operator MUST be a single symbol whose result type and operation are resolved from its operand types, rather than a set of type-specific symbols the author selects by hand — the same operator writes integer, arbitrary-precision, exact-rational, and floating-point arithmetic, dispatched on what its operands are.
    for op in ["+", "-", "*", "/", "%", "<<", ">>", "&", "|", "^"] {
        names.insert(op.to_string(), operator_record(ast, op, OpShape::IntBinary));
    }

    // The relational comparisons — `∀a. a → a → Bool`. The operand is a BARE type variable (it relates
    // `Bool` and structurally any value, not only integers) and the result is `Bool`. Same operator-
    // record mechanism; only the `(meta t)` type-lambda differs.
    for op in ["<", ">", "<=", ">=", "="] {
        names.insert(
            op.to_string(),
            operator_record(ast, op, OpShape::Comparison),
        );
    }

    // `compare` — the THREE-WAY comparison `∀a. a → a → Ordering`. The PRIMITIVE the boolean `<`/`>`/…
    // agree with; its result is the `Ordering` sum (Less/Equal/Greater), not a Bool — an ordinary closed
    // three-variant sum deconstructed by the same exhaustive match as any other sum. Same operator-record
    // mechanism as the relational comparisons; only the result type differs. The relational operators
    // above and this `compare` are two surfaces of the SAME total order (`OpShape::Comparison` vs
    // `OpShape::Compare` over one comparison), so the boolean operators cannot disagree with the three-way.
    //= spec/capabilities/core-semantics.md#a-total-order-is-observed-through-a-three-way-comparison
    //# A type that offers a total order MUST offer a three-way comparison that yields an ordering value with exactly three variants — less, equal, and greater — so that a single comparison reports the full relation between two values rather than a single boolean bit of it.
    //= spec/capabilities/core-semantics.md#a-total-order-is-observed-through-a-three-way-comparison
    //# The ordering value's type MUST be an ordinary closed sum type of the language, so that a comparison result is deconstructed by the same exhaustive match as any other sum and every consumer handles all three cases.
    //= spec/capabilities/core-semantics.md#a-total-order-is-observed-through-a-three-way-comparison
    //# The boolean ordering operators MUST agree with the three-way comparison, so that a type has one total order surfaced two ways that cannot disagree.
    // `compare` is NO LONGER a bare top-level name — it is NAMESPACED as `Ordering.of` (operator directive:
    // prelude records with associated functions, not bare globals). Its op-record is attached as the `of`
    // field on the built-in `Ordering` record via `TypeDecl.associated` (set in `sums::prelude_decls` at the
    // Ordering declaration, `ordering_of_field`), reached as `(. Ordering of)` / `Ordering.of a b`.

    // `trap` — the DIVERGING primitive `∀a. String → a` (core-semantics.md §A Trap Occurs Only Where Its
    // Computation Is Observed; type-system.md §Never Is The Empty Sum — a diverging expression has type
    // `Never`, which unifies with any expected type). A bare-name operator record whose `(meta t)` is the
    // type-lambda `(fn (a) (-> String a))`: the RESULT is the quantified variable `a`, so ordinary HM
    // instantiates it fresh at each use and unifies it with whatever the position demands — `(trap "x")`
    // fits an Int64 branch, a Float operand, a match scrutinee alike, with NO dedicated `Never` type. Its
    // `(meta apply)` is the `trap` intrinsic → `Prim::Trap` → `Core::Trap` (an unconditional `unreachable`).
    // The quantified result IS how the language realizes the empty-sum type of a diverging expression: it
    // has no dedicated `Never`, so the divergence's type is a fresh variable, which unifies with any
    // expected type exactly as the empty sum must:
    //= spec/capabilities/type-system.md#never-is-the-empty-sum
    //# The type of an expression that diverges rather than producing a value — a trap, or requiring the value of an absent optional — MUST be the empty sum, and that type MUST unify with any expected type, because a diverging expression yields no value that could be of the wrong type.
    {
        let lambda = trap_type_lambda(ast);
        names.insert("trap".to_string(), list_op_record(ast, "trap", lambda));
    }

    // `print` / `read` are NO LONGER bare top-level names — they are NAMESPACED as `Ast.print` / `Ast.read`
    // (operator directive: prelude records with associated functions, not bare globals). Their op-records are
    // attached as the `print` / `read` fields on the built-in `Ast` record via `TypeDecl.associated` (set in
    // `sums::prelude_decls` at the Ast declaration, `ast_associated_fields`), reached as `(. Ast print)` /
    // `Ast.print v : String` and `(. Ast read)` / `Ast.read s : Ast`. See `ast_print_field` / `ast_read_field`.

    // Floating-point arithmetic reuses the ONE `+`/`-`/`*`/`/` operator above — there is no distinct
    // float operator. A `Float`-typed operand routes the `Add`/`Sub`/`Mul`/`Div` prim to the machine
    // float op by the SOLVED operand type (`infer::apply_type` types it `Float`; `lower` remaps to
    // `Prim::FAdd`… and folds/emits `f64.add`…), exactly as a `BigInt`/`Rational`/`Qty` operand routes to
    // its own arithmetic. A mixed integer/float application is CDZ0301 (numeric-model.md §An Arithmetic
    // Operator Requires Both Operands To Be One Numeric Type) — the mismatch follows from the operands
    // disagreeing, not from the operator naming a type.

    // `Float` — the float-TYPE constructor: `(Float 32)` / `(Float 64)` in type position builds the
    // float type-value, applied via `(meta apply)` exactly as `Int`/`UInt`. A width outside the admitted
    // set {32,64} is rejected CDZ0302 (numeric-model.md §A Floating-Point Type Is Indexed By A
    // Compile-Time Width). Same `ctor_record` mechanism as `Int`; only the builder prim differs.
    names.insert("Float".to_string(), ctor_record(ast, "Float"));

    // The named fixed-width integer modules — `Int8`/`Int16`/`Int32`/`Int64` and
    // `UInt8`/`UInt16`/`UInt32`/`UInt64`. Each is an ALIAS for the module `(Int N)` / `(UInt N)`
    // reduces to: a record whose `(meta t)` is that width's concrete type-value and whose `max`/`min`
    // fields are that width's bounds (`UInt64.max = 2^64-1`, exact). Built by the SAME width-generic
    // builder the constructor uses, so a named width and `(Int N)` denote the same module — nothing is
    // special-cased per name. `(Int N)` for any other width (odd ones like `(UInt 7)`) is built on
    // demand by the constructor; these are just the commonly-written names pre-installed.
    for (name, signed, width) in [
        ("Int8", true, 8u32),
        ("Int16", true, 16),
        ("Int32", true, 32),
        ("Int64", true, 64),
        ("UInt8", false, 8),
        ("UInt16", false, 16),
        ("UInt32", false, 32),
        ("UInt64", false, 64),
    ] {
        names.insert(name.to_string(), int_module_record(ast, signed, width));
    }

    // The named float modules — `Float32`/`Float64`, ALIASES for `(Float 32)`/`(Float 64)`. Each is a
    // record whose `(meta t)` is that width's concrete float type-value + operation fields (`of-int`
    // and the width conversions, realized as they land). Built by the SAME width builder the `Float`
    // constructor uses (`eval::build_float_module`), so a named float width and `(Float N)` denote one
    // module — nothing special-cased per name, exactly like the integer widths.
    for (name, width) in [("Float32", 32u32), ("Float64", 64)] {
        names.insert(name.to_string(), float_module_record(ast, width));
    }

    // Attach a `(meta doc "…")` channel to a starter set of built-in module records, so the `DocOf`
    // query can surface a built-in's documentation the SAME way it surfaces a user def's — as data on the
    // record, read back generically (`eval::project_meta(_, "doc")`), never by matching the name in the
    // query. A built-in module is just a record, so its documentation is just another meta field on it —
    // exactly as its type (`(meta t)`) and its apply behaviour (`(meta apply)`) already are. Grammar
    // KEYWORDS (`if`/`let`/…) are not bindings in this map, so their docs live in a small table the query
    // consults as its final fallback (`sidecar::grammar_keyword_doc`), the doc analogue of the already-
    // hardcoded `resolve::GRAMMAR` set.
    attach_builtin_docs(ast, &mut names);

    names
}

/// The documentation text for each built-in NAME carrying a `(meta doc)` channel — one `(name, text)`
/// row per documented module/operator. A starter set (the common collections, text, and arithmetic
/// surface); more rows are added as their docs are written, never a behaviour change. The text is a
/// single line (a doc summary), matching a user `(doc "…")` form's one-string shape.
fn builtin_doc_table() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "List",
            "A persistent, immutable sequence indexed from 0. Operations (len, push, concat, update, at) each yield a new list and leave the operand unchanged.",
        ),
        (
            "Map",
            "A persistent, immutable map associating each key with at most one value. Operations (empty, insert, lookup, remove, size) each yield a new map; lookup and remove are total.",
        ),
        (
            "Set",
            "A persistent, immutable set of distinct elements. Operations (of, contains, len, insert, remove, union, intersection, difference) each yield a new set.",
        ),
        (
            "String",
            "An immutable sequence of Unicode scalar values. Reachable operations project by member access (String.scalar-len, String.byte-len).",
        ),
        (
            "Bytes",
            "An immutable sequence of bytes. Reachable operations project by member access (Bytes.of, Bytes.len).",
        ),
        (
            "+",
            "Integer addition. Width-generic: (+ a b) : (Int a) -> (Int a) -> (Int a).",
        ),
        (
            "-",
            "Integer subtraction. Width-generic: (- a b) : (Int a) -> (Int a) -> (Int a).",
        ),
        (
            "*",
            "Integer multiplication. Width-generic: (* a b) : (Int a) -> (Int a) -> (Int a).",
        ),
        (
            "compare",
            "Three-way comparison yielding an Ordering (Less/Equal/Greater) — the total order the boolean operators (<, >, =, ...) agree with.",
        ),
    ]
}

/// Append a `(meta doc "text")` channel to each record named in [`builtin_doc_table`], IN PLACE. Uses the
/// same [`meta_field`] helper the `(meta t)`/`(meta apply)` channels use, so a doc is one more meta field
/// on the record — read back by `eval::project_meta(_, "doc")` and INVISIBLE to member access (a `(meta
/// doc)` key is namespaced, so `(. List doc)` does not project it) and to record typing (which reads
/// `(meta t)`/`(meta apply)`, not the field set). A name absent from `names`, or whose occurrence is not a
/// `(record …)` list, is skipped — the table stays a pure addition that cannot break a record's shape.
fn attach_builtin_docs(ast: &mut Arenas, names: &mut BTreeMap<String, StructId>) {
    for (name, text) in builtin_doc_table() {
        let Some(&rec) = names.get(*name) else {
            continue;
        };
        // Only a `(record …)` list carries meta channels; a bare atom (e.g. `unit`) has none to extend.
        if !matches!(ast.get(rec), Struct::List(_)) {
            continue;
        }
        let text_node = push_atom(ast, Leaf::Str((*text).into()));
        let doc_field = meta_field(ast, "doc", text_node);
        let Struct::List(children) = ast.get(rec) else {
            continue;
        };
        let mut kids = children.clone();
        kids.push(doc_field);
        ast.structure[rec.0 as usize] = Struct::List(kids);
    }
}

/// An `(intrinsic NAME)` node — the arena form a native primitive value takes. `resolve` turns it
/// into a `Resolved::Prim`; the name selects which primitive.
fn intrinsic_node(ast: &mut Arenas, name: &str) -> StructId {
    let head = push_atom(ast, Leaf::Name("intrinsic".into()));
    let who = push_atom(ast, Leaf::Name(name.into()));
    push_list(ast, vec![head, who])
}

/// A meta field `((meta KEY) VALUE)` — a record field whose key is the `meta`-namespaced symbol
/// `KEY`. This is how the reserved meta channel is written as ordinary record structure. `pub(crate)`
/// so the program-driven sum-record synthesis (`sum_synth`) writes its `(meta t)`/`(meta variant)`
/// channels the same way the prelude writes its built-in records.
pub(crate) fn meta_field(ast: &mut Arenas, key: &str, value: StructId) -> StructId {
    // seq-276: canonical FieldPair `(= (meta key) value)` form (not a bare `((meta k) v)` pair), so the
    // prelude's synthesized module records satisfy the value-entry require-`=` rule. `read_record_fields`
    // reads it via `field_pair` into the identical fields map — consumers (`project_field`) are unaffected.
    let eq = push_atom(ast, Leaf::Name("=".into()));
    let meta_head = push_atom(ast, Leaf::Name("meta".into()));
    let key_name = push_atom(ast, Leaf::Name(key.into()));
    let meta_key = push_list(ast, vec![meta_head, key_name]);
    push_list(ast, vec![eq, meta_key, value])
}

/// A ground-type record `(record ((meta t) (intrinsic PRIM)))` — `Bool`/`Unit`. Its `(meta t)` holds
/// the ground type-value; it carries no `(meta apply)`, so it is not applyable.
fn ground_type_record(ast: &mut Arenas, prim: &str) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let ty_val = intrinsic_node(ast, prim);
    let t_field = meta_field(ast, "t", ty_val);
    push_list(ast, vec![head, t_field])
}

/// A type-constructor record `(record ((meta apply) (intrinsic PRIM)))` — `Int`/`UInt`/`->`. Applying
/// it (`(Int a)`) projects `(meta apply)` and applies the native builder.
fn ctor_record(ast: &mut Arenas, prim: &str) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let builder = intrinsic_node(ast, prim);
    let apply_field = meta_field(ast, "apply", builder);
    push_list(ast, vec![head, apply_field])
}

/// The `Record` module record — carrying BOTH `(meta apply)` = the `Record` TYPE constructor (so
/// `(Record (a Int64) (b Bool))` in type position builds `Ty::Record`) AND a field per record ROW
/// OPERATION (reached by member access `(. Record project)`). The dual shape `list_module`/`map_module`
/// use: a type ctor + op fields on one record. This increment realizes `project` — narrowing a record to
/// a named field set (`type-system.md` §A Record Is Restricted To A Named Set Of Its Fields); `without`/
/// `merge`/`with`/`pop`/`extend` follow. A row op's SECOND operand is a LITERAL field-name list `(a c)`
/// (labels, not a value), and its result shape is row-polymorphic, so it has no ordinary HM `(meta t)`
/// arrow — `infer::apply_type` computes the result type and `check_application`/`collect_node` special-
/// case the label operand, bypassing the scheme. The `(meta t)` is thus a PERMISSIVE placeholder
/// (`∀a. a → a`) present only so `project` resolves as an applyable op; it is never unified.
fn record_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta apply)` = the `Record` TYPE constructor (`(Record (a Int64) …)` builds the record type-value).
    let builder = intrinsic_node(ast, "Record");
    let apply_field = meta_field(ast, "apply", builder);
    // The row operations, each an operator record whose `(meta t)` is the permissive `∀a. a → a`
    // placeholder (bypassed — `infer::apply_type` computes the result type, `check_application` skips the
    // scheme-unify). `project`/`without` take a record + a LITERAL field-name list; `merge` takes two
    // record VALUES; `extend`/`with` a record + a `(name value)` pair; `pop` a record + a bare field name.
    let mut children = vec![head, apply_field];
    for (name, prim) in [
        ("project", "record-project"),
        ("without", "record-without"),
        ("merge", "record-merge"),
        ("extend", "record-extend"),
        ("with", "record-with"),
        ("pop", "record-pop"),
    ] {
        let lambda = row_op_placeholder_type(ast);
        let op = list_op_record(ast, prim, lambda);
        let key = push_atom(ast, Leaf::Name(name.into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, key, op])
        });
    }
    push_list(ast, children)
}

/// The `Tuple` module record — carrying BOTH `(meta apply)` = the `Tuple` TYPE constructor (`(Tuple
/// Int64 Bool)` in type position builds `Ty::Tuple`) AND a field per POSITIONAL row operation (`(. Tuple
/// cat)`), the tuple analogue of `record_module`. Realizes `cat` (concatenate two tuples), `split-at`
/// (split at a compile-time position → a prefix/suffix pair), `pop` (element 0 off). A tuple op's result
/// arity is fixed statically from the operands', and a position `k` is a compile-time literal — so, like
/// the record row ops, there is no ordinary HM arrow: `infer::apply_type` computes the result type and
/// `check_application` skips the scheme-unify. The `(meta t)` is the same permissive `∀a. a → a`
/// placeholder (never unified), present only so member access resolves the op as applyable.
fn tuple_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta apply)` = the `Tuple` TYPE constructor (`(Tuple Int64 Bool)` builds the tuple type-value).
    let builder = intrinsic_node(ast, "Tuple");
    let apply_field = meta_field(ast, "apply", builder);
    let mut children = vec![head, apply_field];
    for (name, prim) in [
        ("concat", "tuple-cat"),
        ("split-at", "tuple-split-at"),
        ("remove", "tuple-pop"),
        // `Tuple.size t : Int64` — the tuple's arity (a static property of its type; folds to a
        // constant Int in `lower`). Result type computed in `infer::apply_type` like the other tuple ops.
        ("size", "tuple-size"),
    ] {
        let lambda = row_op_placeholder_type(ast);
        let op = list_op_record(ast, prim, lambda);
        let key = push_atom(ast, Leaf::Name(name.into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, key, op])
        });
    }
    push_list(ast, children)
}

/// The `Num` namespace record — the number-shape capability that carries the generic numeric ops. B1
/// (v-compiler-primitives) provides `neg : ∀a. a → a` (a GENERIC unary negation over any number type),
/// backed by the `neg` intrinsic (`Prim::Neg`), which lowers through `lower_negate` — folding a constant
/// and dispatching Int (`0 - e`) / Float (`-1.0 * e`) / BigInt / Rational. The scheme is unconstrained
/// `∀a. a → a` here; v-inference attaches the `Num` CONSTRAINT (admit signed-int / float / bigint /
/// rational; REJECT an unsigned integer at COMPILE time with a coded diagnostic, and a non-number). `Num`
/// is an op namespace (no `(meta t)`): it is reached only by member access `(. Num neg)`, not used as a
/// type. The full `Num` trait (the remaining numeric capabilities) grows here as the operator scopes it.
fn num_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `neg : ∀a. a → a` — the generic negate. `(fn (a) (-> a a))`: a real unifiable scheme (unlike
    // `row_op_placeholder_type`, which is never unified against), so `(Num.neg (: x T))` solves `a = T`.
    let neg_ty = {
        let a1 = push_atom(ast, Leaf::Name("a".into()));
        let a2 = push_atom(ast, Leaf::Name("a".into()));
        let body = arrow_type(ast, a1, a2);
        let param = push_atom(ast, Leaf::Name("a".into()));
        let fn_head = push_atom(ast, Leaf::Name("fn".into()));
        let params = push_list(ast, vec![param]);
        push_list(ast, vec![fn_head, params, body])
    };
    let neg_op = list_op_record(ast, "neg", neg_ty);
    let neg_key = push_atom(ast, Leaf::Name("neg".into()));
    let neg_field = {
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, neg_key, neg_op])
    };
    push_list(ast, vec![head, neg_field])
}

/// The permissive placeholder type-lambda `(fn (a) (-> a a))` a record row operation carries as its
/// `(meta t)`. A row op (`Record.project`) has no ordinary HM arrow — its label-list operand is not a
/// typed value and its result is row-polymorphic — so `infer::apply_type` computes the result type and
/// `check_application` skips the generic scheme-unify. This scheme exists ONLY so member access resolves
/// the op as applyable; it is NEVER unified against, so `∀a. a → a` (the identity arrow) suffices.
fn row_op_placeholder_type(ast: &mut Arenas) -> StructId {
    let a1 = push_atom(ast, Leaf::Name("a".into()));
    let a2 = push_atom(ast, Leaf::Name("a".into()));
    let body = arrow_type(ast, a1, a2); // (-> a a)
    let param = push_atom(ast, Leaf::Name("a".into()));
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![param]);
    push_list(ast, vec![fn_head, params, body])
}

/// The `List` module record — a record carrying BOTH `(meta apply)` = the `List` type constructor (so
/// `(List Int64)` in type position builds `Ty::List`) AND a field per list OPERATION (reached by member
/// access `(. List len)`). Each operation is an operator record: its `(meta t)` is a type-lambda over
/// the element type, its `(meta apply)` the runtime op. Realizes `len : ∀a. (List a) → Int64`, `push`
/// (functional append → a new list), `concat` (two same-element-type lists → their ordered
/// concatenation), `update` (replace-at-index → a new list), and `at : ∀a. (List a) → Int64 → (Option
/// a)` (the FALLIBLE indexed read — `None` out of bounds, never a trap). Each construction op yields a
/// NEW list and leaves its operand unchanged (immutable growth, on the persistent `vec-*` heap).
//= spec/capabilities/collections-and-text.md#a-list-is-grown-by-functional-construction
//# A list MUST offer an operation that appends an element and an operation that replaces the element at an index, each of which MUST produce a new list value and leave its operand list unchanged, so that a list is immutable under growth exactly as it is under reading.
//= spec/capabilities/collections-and-text.md#a-list-is-grown-by-functional-construction
//# A list MUST also offer an operation that concatenates two lists, producing a new list whose elements are those of the first list in order followed by those of the second, and leaving both operand lists unchanged.
//= spec/capabilities/collections-and-text.md#a-list-is-grown-by-functional-construction
//# The replace-at-index operation MUST be defined only for an index that is in bounds, consistent with the fallible reading rule below, so that growth never observes an element at a position the list does not have.
//= spec/capabilities/collections-and-text.md#indexing-and-lookup-are-fallible-not-trapping
//# An operation that reads an element of a sequence by position — indexing a list, a string (by scalar or byte offset), or a `Bytes` value, or taking a sub-sequence slice — MUST be total, yielding an optional value that is present when the position is in bounds and absent when it is out of bounds, rather than trapping or producing an unspecified value.
fn list_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta apply)` = the `List` TYPE constructor (`(List Int64)` reduces to `Ty::List(Int64)`).
    let builder = intrinsic_node(ast, "List");
    let apply_field = meta_field(ast, "apply", builder);
    // One field per realized operation — each an operator record `(name <op-record>)`. `len : ∀a. (List
    // a) → Int64`; `push : ∀a. (List a) → a → (List a)`; `concat : ∀a. (List a) → (List a) → (List a)`.
    // Each lambda is built first (a `&mut ast` borrow) then handed to `list_op_record`.
    let len_lambda = list_len_type_lambda(ast);
    let push_lambda = list_push_type_lambda(ast);
    // `prepend` shares `push`'s scheme exactly — `∀a. (List a) → a → (List a)` — so it reuses the
    // `list_push_type_lambda` shape (a second, independent build to avoid sharing one StructId across
    // two module fields).
    let prepend_lambda = list_push_type_lambda(ast);
    let concat_lambda = list_concat_type_lambda(ast);
    let update_lambda = list_update_type_lambda(ast);
    let at_lambda = list_at_type_lambda(ast);
    let mut children = vec![head, apply_field];
    for (name, prim, lambda) in [
        ("len", "list-len", len_lambda),
        ("push", "list-push", push_lambda),
        ("prepend", "list-prepend", prepend_lambda),
        ("concat", "list-concat", concat_lambda),
        ("update", "list-update", update_lambda),
        ("at", "list-at", at_lambda),
    ] {
        let op = list_op_record(ast, prim, lambda);
        let k = push_atom(ast, Leaf::Name(name.into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, k, op])
        });
    }
    push_list(ast, children)
}

/// The `Map` module record — a record carrying BOTH `(meta apply)` = the `Map` type constructor (so
/// `(Map Int64 Int64)` in type position builds `Ty::Map`, TWO parameters) AND a field per map OPERATION
/// (reached by member access `(. Map insert)`). Each operation is an operator record: its `(meta t)` is
/// a TWO-parameter type-lambda `(fn (k v) …)` over the key and value types, its `(meta apply)` the
/// runtime op. Realizes `empty : ∀k v. (Map k v)`, `insert : ∀k v. (Map k v) → k → v → (Map k v)`,
/// `lookup : ∀k v. (Map k v) → k → (Option v)`, `remove : ∀k v. (Map k v) → k → (Map k v)`, `size :
/// ∀k v. (Map k v) → Int64`. Mirrors `list_module`, but the type constructor and every scheme take two
/// parameters instead of one. `empty`/`insert`/`remove` are the functional-construction surface (each
/// yields a NEW map, operand unchanged, on the persistent CHAMP heap): `insert` is add-OR-REPLACE (a key
/// already present has its value replaced, never a second entry — the CHAMP holds each key once), and
/// `remove` of an ABSENT key returns a map equal to the operand rather than trapping (removal is total).
/// `lookup` is total (`Option v`, `None` for an absent key, never a trap); `size` reports the key count.
/// `swap`/`take` are the
/// VALUE-YIELDING second forms of insert/remove — each returns `(tuple <prior value as Option> <new
/// map>)`, agreeing with the plain form on the resulting map and additionally reporting what the key
/// held before, so a program observes a replaced/dropped value without a separate `lookup`.
//= spec/capabilities/collections-and-text.md#a-map-is-built-by-functional-construction
//# A map MUST offer an empty map value, an operation that adds or replaces the association for a key, and an operation that removes the association for a key.
//= spec/capabilities/collections-and-text.md#a-map-is-built-by-functional-construction
//# Each MUST produce a new map value and leave its operand map unchanged, so that a map is immutable under update exactly as a list is under growth (*A List Is Grown By Functional Construction*).
//= spec/capabilities/collections-and-text.md#a-map-is-built-by-functional-construction
//# Adding a key already present MUST replace that key's value rather than introduce a second entry, preserving the *A Map Associates Keys With Values* rule that a map contains each key at most once.
//= spec/capabilities/collections-and-text.md#a-map-is-built-by-functional-construction
//# Removing a key the map does not contain MUST yield a map equal to the operand rather than trapping, so that removal is total.
//= spec/capabilities/collections-and-text.md#a-map-is-built-by-functional-construction
//# A map MUST report the number of keys it associates, and that count MUST equal the number of distinct keys added and not since removed.
//= spec/capabilities/collections-and-text.md#a-map-is-built-by-functional-construction
//# The add-or-replace and the remove operations MUST each come in two forms: a plain form yielding only the new map, and a form that additionally yields the value the key held before the operation as an optional — present when the key was associated beforehand and absent when it was not — paired with the new map. The plain form is the common case that discards the prior value; the value-yielding form lets a program observe what an add replaced or a remove dropped in a single operation, without a separate lookup. The two forms MUST agree on the resulting map, so that the only difference is whether the prior value is reported.
//= spec/capabilities/collections-and-text.md#indexing-and-lookup-are-fallible-not-trapping
//# Looking a key up in a map MUST likewise be total, yielding an optional value that is present when the map contains the key and absent when it does not.
fn map_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta apply)` = the `Map` TYPE constructor (`(Map Int64 Int64)` reduces to `Ty::Map(Int64, Int64)`).
    let builder = intrinsic_node(ast, "Map");
    let apply_field = meta_field(ast, "apply", builder);
    // One field per realized operation — each an operator record `(name <op-record>)` whose `(meta t)`
    // is a two-parameter `(fn (k v) …)` type-lambda. Each lambda is built first (a `&mut ast` borrow)
    // then handed to `list_op_record` (the shared operator-record builder — nothing list-specific in it).
    let empty_lambda = map_empty_type_lambda(ast);
    let insert_lambda = map_insert_type_lambda(ast);
    let merge_lambda = map_merge_type_lambda(ast);
    let lookup_lambda = map_lookup_type_lambda(ast);
    let remove_lambda = map_remove_type_lambda(ast);
    let size_lambda = map_size_type_lambda(ast);
    let to_list_lambda = map_to_list_type_lambda(ast);
    let swap_lambda = map_swap_type_lambda(ast);
    let take_lambda = map_take_type_lambda(ast);
    let mut children = vec![head, apply_field];
    for (name, prim, lambda) in [
        ("empty", "map-empty", empty_lambda),
        ("insert", "map-insert", insert_lambda),
        ("merge", "map-merge", merge_lambda),
        ("lookup", "map-lookup", lookup_lambda),
        ("remove", "map-remove", remove_lambda),
        ("len", "map-size", size_lambda),
        ("to-list", "map-to-list", to_list_lambda),
        ("swap", "map-swap", swap_lambda),
        ("take", "map-take", take_lambda),
    ] {
        let op = list_op_record(ast, prim, lambda);
        let k = push_atom(ast, Leaf::Name(name.into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, k, op])
        });
    }
    push_list(ast, children)
}

/// The `Set` module record — carries BOTH `(meta apply)` = the `Set` type constructor (`(Set Int64)`
/// builds `Ty::Set`, ONE parameter) AND a field per set OPERATION (reached by member access). Each op is
/// an operator record whose `(meta t)` is a one-parameter `(fn (a) …)` type-lambda (like `List`). Realizes
/// `of : ∀a. (List a) → (Set a)`, `contains : ∀a. (Set a) → a → Bool`, `len : ∀a. (Set a) → Int64`,
/// `insert`/`remove : ∀a. (Set a) → a → (Set a)`, and `union`/`intersection`/`difference : ∀a. (Set a) →
/// (Set a) → (Set a)`. Mirrors `list_module` (one type parameter, unlike `map_module`'s two). `contains`
/// yields a plain `Bool` (total membership, never a trap); there is NO positional-access field (no `at`)
/// — a set is unordered, so it has no element to address by position.
//= spec/capabilities/collections-and-text.md#set-membership-is-total
//# Testing whether a set contains an element MUST be total, yielding a boolean rather than trapping.
//= spec/capabilities/collections-and-text.md#set-membership-is-total
//# A set MUST NOT offer access to an element by position, because a set is unordered and has no positional element to address.
fn set_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta apply)` = the `Set` TYPE constructor (`(Set Int64)` reduces to `Ty::Set(Int64)`).
    let builder = intrinsic_node(ast, "Set");
    let apply_field = meta_field(ast, "apply", builder);
    let of_lambda = set_of_type_lambda(ast);
    let to_list_lambda = set_to_list_type_lambda(ast);
    let contains_lambda = set_contains_type_lambda(ast);
    let len_lambda = set_len_type_lambda(ast);
    let insert_lambda = set_elem_to_set_type_lambda(ast); // (Set a) → a → (Set a)
    let remove_lambda = set_elem_to_set_type_lambda(ast);
    let union_lambda = set_binary_type_lambda(ast); // (Set a) → (Set a) → (Set a)
    let intersection_lambda = set_binary_type_lambda(ast);
    let difference_lambda = set_binary_type_lambda(ast);
    let mut children = vec![head, apply_field];
    for (name, prim, lambda) in [
        ("of", "set-of", of_lambda),
        ("to-list", "set-to-list", to_list_lambda),
        ("contains", "set-contains", contains_lambda),
        ("len", "set-len", len_lambda),
        ("insert", "set-insert", insert_lambda),
        ("remove", "set-remove", remove_lambda),
        ("union", "set-union", union_lambda),
        ("intersection", "set-intersection", intersection_lambda),
        ("difference", "set-difference", difference_lambda),
    ] {
        let op = list_op_record(ast, prim, lambda);
        let k = push_atom(ast, Leaf::Name(name.into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, k, op])
        });
    }
    push_list(ast, children)
}

/// Build `(Set a)` — the set type applied to the element parameter `a`, the shared shape in the `Set`
/// operation type-lambdas (a fresh occurrence per use, referencing the same param name `a`).
fn set_a_type(ast: &mut Arenas) -> StructId {
    let set = push_atom(ast, Leaf::Name("Set".into()));
    let a = push_atom(ast, Leaf::Name("a".into()));
    push_list(ast, vec![set, a])
}

/// `(fn (a) (-> (List a) (Set a)))` for `Set.of` — `∀a. (List a) → (Set a)`: construct a set from a list.
fn set_of_type_lambda(ast: &mut Arenas) -> StructId {
    let set_a = set_a_type(ast);
    let list_a = list_a_type(ast);
    let body = arrow_type(ast, list_a, set_a); // (-> (List a) (Set a))
    list_type_lambda(ast, body)
}

/// `(fn (a) (-> (Set a) (List a)))` for `Set.to-list` — `∀a. (Set a) → (List a)`: enumerate the set's
/// elements as a list in CANONICAL element-value order (the inverse of `Set.of`; realizes
/// collections-and-text.md §Map/Set iteration is deterministic).
fn set_to_list_type_lambda(ast: &mut Arenas) -> StructId {
    let set_a = set_a_type(ast);
    let list_a = list_a_type(ast);
    let body = arrow_type(ast, set_a, list_a); // (-> (Set a) (List a))
    list_type_lambda(ast, body)
}

/// `(fn (a) (-> (Set a) (-> a Bool)))` for `Set.contains` — `∀a. (Set a) → a → Bool`: total membership.
fn set_contains_type_lambda(ast: &mut Arenas) -> StructId {
    let a = push_atom(ast, Leaf::Name("a".into()));
    let bool_t = push_atom(ast, Leaf::Name("Bool".into()));
    let elem_arrow = arrow_type(ast, a, bool_t); // (-> a Bool)
    let set_a = set_a_type(ast);
    let body = arrow_type(ast, set_a, elem_arrow); // (-> (Set a) (-> a Bool))
    list_type_lambda(ast, body)
}

/// `(fn (a) (-> (Set a) Int64))` for `Set.len` — `∀a. (Set a) → Int64`: the distinct-element count.
fn set_len_type_lambda(ast: &mut Arenas) -> StructId {
    let set_a = set_a_type(ast);
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let body = arrow_type(ast, set_a, int64); // (-> (Set a) Int64)
    list_type_lambda(ast, body)
}

/// `(fn (a) (-> (Set a) (-> a (Set a))))` for `Set.insert`/`Set.remove` — `∀a. (Set a) → a → (Set a)`.
fn set_elem_to_set_type_lambda(ast: &mut Arenas) -> StructId {
    let set_r = set_a_type(ast);
    let a = push_atom(ast, Leaf::Name("a".into()));
    let elem_arrow = arrow_type(ast, a, set_r); // (-> a (Set a))
    let set_l = set_a_type(ast);
    let body = arrow_type(ast, set_l, elem_arrow); // (-> (Set a) (-> a (Set a)))
    list_type_lambda(ast, body)
}

/// `(fn (a) (-> (Set a) (-> (Set a) (Set a))))` for `Set.union`/`intersection`/`difference` — `∀a. (Set
/// a) → (Set a) → (Set a)`: the binary set-algebra ops.
fn set_binary_type_lambda(ast: &mut Arenas) -> StructId {
    let set_r = set_a_type(ast);
    let set_2 = set_a_type(ast);
    let inner = arrow_type(ast, set_2, set_r); // (-> (Set a) (Set a))
    let set_1 = set_a_type(ast);
    let body = arrow_type(ast, set_1, inner); // (-> (Set a) (-> (Set a) (Set a)))
    list_type_lambda(ast, body)
}

/// Build `(Map k v)` — the map type applied to the key parameter `k` and value parameter `v`, the shared
/// shape in the `Map` operation type-lambdas (a fresh occurrence per use, referencing the same param names).
fn map_k_v_type(ast: &mut Arenas) -> StructId {
    let map = push_atom(ast, Leaf::Name("Map".into()));
    let k = push_atom(ast, Leaf::Name("k".into()));
    let v = push_atom(ast, Leaf::Name("v".into()));
    push_list(ast, vec![map, k, v])
}

/// Wrap `body` in `(fn (k v) body)` — the two-parameter type-lambda over the key type `k` and value type
/// `v`, shared by the `Map` operation schemes (the map analogue of `list_type_lambda`).
fn map_type_lambda(ast: &mut Arenas, body: StructId) -> StructId {
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let k_param = push_atom(ast, Leaf::Name("k".into()));
    let v_param = push_atom(ast, Leaf::Name("v".into()));
    let params = push_list(ast, vec![k_param, v_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type-lambda `(fn (k v) (Map k v))` for `Map.empty` — `∀k v. (Map k v)`: the empty map value, a
/// bare (non-arrow) polymorphic type. The `(fn (k v) …)` wrapper makes `scheme_of` read a SCHEME over
/// both parameters, so each use of `Map.empty` instantiates a fresh `(Map ?k ?v)` its keys/values solve.
fn map_empty_type_lambda(ast: &mut Arenas) -> StructId {
    let map_k_v = map_k_v_type(ast);
    map_type_lambda(ast, map_k_v)
}

/// The type-lambda `(fn (k v) (-> (Map k v) (-> k (-> v (Map k v)))))` for `Map.insert` — `∀k v. (Map k
/// v) → k → v → (Map k v)`: add-or-replace `key ↦ val`, returning the new map.
fn map_insert_type_lambda(ast: &mut Arenas) -> StructId {
    let map_r = map_k_v_type(ast);
    let v = push_atom(ast, Leaf::Name("v".into()));
    let val_arrow = arrow_type(ast, v, map_r); // (-> v (Map k v))
    let k = push_atom(ast, Leaf::Name("k".into()));
    let key_arrow = arrow_type(ast, k, val_arrow); // (-> k (-> v (Map k v)))
    let map_l = map_k_v_type(ast);
    let body = arrow_type(ast, map_l, key_arrow); // (-> (Map k v) (-> k (-> v (Map k v))))
    map_type_lambda(ast, body)
}

/// The type-lambda `(fn (k v) (-> (Map k v) (-> k (Option v))))` for `Map.lookup` — `∀k v. (Map k v) →
/// k → (Option v)`: the fallible keyed read (`Some v` present, `None` absent). `(Option v)` reduces via
/// the built-in `Option` sum ctor, exactly as `List.at`'s `(Option a)` does.
fn map_lookup_type_lambda(ast: &mut Arenas) -> StructId {
    let option_v = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let v = push_atom(ast, Leaf::Name("v".into()));
        push_list(ast, vec![option, v])
    };
    let k = push_atom(ast, Leaf::Name("k".into()));
    let key_arrow = arrow_type(ast, k, option_v); // (-> k (Option v))
    let map_l = map_k_v_type(ast);
    let body = arrow_type(ast, map_l, key_arrow); // (-> (Map k v) (-> k (Option v)))
    map_type_lambda(ast, body)
}

/// The type-lambda `(fn (k v) (-> (Map k v) (-> k (Map k v))))` for `Map.remove` — `∀k v. (Map k v) →
/// k → (Map k v)`: drop a key's association, returning the new map (total — an absent key yields an
/// equal map).
fn map_remove_type_lambda(ast: &mut Arenas) -> StructId {
    let map_r = map_k_v_type(ast);
    let k = push_atom(ast, Leaf::Name("k".into()));
    let key_arrow = arrow_type(ast, k, map_r); // (-> k (Map k v))
    let map_l = map_k_v_type(ast);
    let body = arrow_type(ast, map_l, key_arrow); // (-> (Map k v) (-> k (Map k v)))
    map_type_lambda(ast, body)
}

/// The type-lambda `(fn (k v) (-> (Map k v) (-> (Map k v) (Map k v))))` for `Map.merge` — `∀k v. (Map k
/// v) → (Map k v) → (Map k v)`: the union of two maps, LAST-WRITER (right operand) wins on an overlapping
/// key. The map analogue of `List.concat`/`Record.merge`; the runtime `map-merge` CHAMP union (both
/// operands consumed → new map). Backs the value-position map construction spread `#map((= k v) (.. m))`.
fn map_merge_type_lambda(ast: &mut Arenas) -> StructId {
    let map_r = map_k_v_type(ast); // result (Map k v)
    let map_b = map_k_v_type(ast); // second operand (Map k v)
    let snd_arrow = arrow_type(ast, map_b, map_r); // (-> (Map k v) (Map k v))
    let map_a = map_k_v_type(ast); // first operand (Map k v)
    let body = arrow_type(ast, map_a, snd_arrow); // (-> (Map k v) (-> (Map k v) (Map k v)))
    map_type_lambda(ast, body)
}

/// The type-lambda `(fn (k v) (-> (Map k v) Int64))` for `Map.size` — `∀k v. (Map k v) → Int64`: the
/// count of distinct keys. The map companion of `List.len`.
fn map_size_type_lambda(ast: &mut Arenas) -> StructId {
    let map_l = map_k_v_type(ast);
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let body = arrow_type(ast, map_l, int64); // (-> (Map k v) Int64)
    map_type_lambda(ast, body)
}

/// `(fn (k v) (-> (Map k v) (List (Tuple k v))))` for `Map.to-list` — `∀k v. (Map k v) → (List (Tuple k
/// v))`: enumerate the map's entries as a list of `(key, value)` tuples in CANONICAL KEY order
/// (collections-and-text.md §A Map Renders As Its Entries In Canonical Key Order). The map companion of
/// `Set.to-list`.
fn map_to_list_type_lambda(ast: &mut Arenas) -> StructId {
    let map_l = map_k_v_type(ast);
    // (Tuple k v)
    let tuple = push_atom(ast, Leaf::Name("Tuple".into()));
    let k = push_atom(ast, Leaf::Name("k".into()));
    let v = push_atom(ast, Leaf::Name("v".into()));
    let tuple_kv = push_list(ast, vec![tuple, k, v]);
    // (List (Tuple k v))
    let list = push_atom(ast, Leaf::Name("List".into()));
    let list_tuple = push_list(ast, vec![list, tuple_kv]);
    let body = arrow_type(ast, map_l, list_tuple); // (-> (Map k v) (List (Tuple k v)))
    map_type_lambda(ast, body)
}

/// Build `(Tuple (Option v) (Map k v))` — the value-yielding form's result: the prior/removed value as
/// an optional PAIRED with the new map. Shared by `Map.swap`/`Map.take` (collections-and-text.md §A Map
/// Is Built By Functional Construction — the two-form rule).
fn map_optional_and_map_tuple(ast: &mut Arenas) -> StructId {
    let option_v = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let v = push_atom(ast, Leaf::Name("v".into()));
        push_list(ast, vec![option, v])
    };
    let map_k_v = map_k_v_type(ast);
    let tuple = push_atom(ast, Leaf::Name("Tuple".into()));
    push_list(ast, vec![tuple, option_v, map_k_v]) // (Tuple (Option v) (Map k v))
}

/// The type-lambda `(fn (k v) (-> (Map k v) (-> k (-> v (Tuple (Option v) (Map k v))))))` for
/// `Map.swap` — `∀k v. (Map k v) → k → v → (Tuple (Option v) (Map k v))`: add-or-replace, reporting the
/// value the key held before (present when it was associated) paired with the new map.
fn map_swap_type_lambda(ast: &mut Arenas) -> StructId {
    let result = map_optional_and_map_tuple(ast);
    let v = push_atom(ast, Leaf::Name("v".into()));
    let val_arrow = arrow_type(ast, v, result); // (-> v (Tuple …))
    let k = push_atom(ast, Leaf::Name("k".into()));
    let key_arrow = arrow_type(ast, k, val_arrow); // (-> k (-> v (Tuple …)))
    let map_l = map_k_v_type(ast);
    let body = arrow_type(ast, map_l, key_arrow); // (-> (Map k v) (-> k (-> v (Tuple …))))
    map_type_lambda(ast, body)
}

/// The type-lambda `(fn (k v) (-> (Map k v) (-> k (Tuple (Option v) (Map k v)))))` for `Map.take` —
/// `∀k v. (Map k v) → k → (Tuple (Option v) (Map k v))`: remove, reporting the value the key held
/// (present when it was associated) paired with the new map. The remove companion of `Map.swap`.
fn map_take_type_lambda(ast: &mut Arenas) -> StructId {
    let result = map_optional_and_map_tuple(ast);
    let k = push_atom(ast, Leaf::Name("k".into()));
    let key_arrow = arrow_type(ast, k, result); // (-> k (Tuple …))
    let map_l = map_k_v_type(ast);
    let body = arrow_type(ast, map_l, key_arrow); // (-> (Map k v) (-> k (Tuple …)))
    map_type_lambda(ast, body)
}

/// The `Bytes` module record — a record carrying `(meta t) = Bytes` (the ground type-value, so bare
/// `Bytes` in type position is `Ty::Bytes`) AND a field per byte-sequence OPERATION (reached by member
/// access `(. Bytes of)`). Unlike `List`, `Bytes` is NOT a type constructor (it is a non-parametric
/// leaf), so its operations are MONOMORPHIC — each `(meta t)` is a plain arrow type, not a `(fn (a) …)`
/// type-lambda. This increment realizes `of : (List (UInt 8)) → Bytes` and `len : Bytes → Int64`; concat/
/// at/slice/compact arrive in later increments (a projected-but-unrealized field DECLINES, the closed-
/// module rule every prelude module follows).
fn bytes_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta t)` = the ground type-value `Bytes` (`(intrinsic bytes-ty)` → `Ty::Bytes`), so bare `Bytes`
    // resolves as a TYPE and `(. Bytes of)` projects the constructor operation.
    let ty_val = intrinsic_node(ast, "bytes-ty");
    let t_field = meta_field(ast, "t", ty_val);
    // One field per realized operation — each an operator record `(name <op-record>)` whose `(meta t)`
    // is a monomorphic arrow type. `of : (List (UInt 8)) → Bytes`; `len : Bytes → Int64`; `at : Bytes →
    // Int64 → (Option Int64)` (the FALLIBLE indexed byte read — the byte companion of `List.at`).
    let of_type = bytes_of_type(ast);
    let len_type = bytes_len_type(ast);
    let at_type = bytes_at_type(ast);
    let concat_type = bytes_concat_type(ast);
    let slice_type = bytes_slice_type(ast);
    let compact_type = bytes_compact_type(ast);
    let mut children = vec![head, t_field];
    for (name, prim, ty) in [
        ("of", "bytes-of", of_type),
        ("len", "bytes-len", len_type),
        ("at", "bytes-at", at_type),
        ("concat", "bytes-concat", concat_type),
        ("slice", "bytes-slice", slice_type),
        ("compact", "bytes-compact", compact_type),
    ] {
        let op = list_op_record(ast, prim, ty);
        let k = push_atom(ast, Leaf::Name(name.into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, k, op])
        });
    }
    push_list(ast, children)
}

/// The `String` module record — a record with one field per string OPERATION (reached by member access
/// `(. String scalar-len)`). Each operation is an operator record: its `(meta t)` is the operation's
/// type (`String → Int64`), its `(meta apply)` the native prim. This increment realizes the two LENGTH
/// queries; concat/at/slice arrive with the runtime byte-rope ops. The two lengths are SEPARATELY NAMED
/// (`scalar-len` / `byte-len`) — there is NO unqualified `len` field, so a length query always names
/// which count it means; and `byte-len` is a direct `str-byte-len` prim, not a count over a materialized
/// UTF-8 byte value.
//= spec/capabilities/collections-and-text.md#a-string-offers-both-a-scalar-length-and-a-byte-length
//# A string MUST offer a length counted in Unicode scalar values and a length counted in the bytes of its UTF-8 encoding as two separately-named operations, so that neither meaning is the unqualified default an author could confuse for the other.
//= spec/capabilities/collections-and-text.md#a-string-offers-both-a-scalar-length-and-a-byte-length
//# A string MUST NOT offer an unqualified length operation, so that every length query names whether it counts scalar values or bytes.
//= spec/capabilities/collections-and-text.md#a-string-offers-both-a-scalar-length-and-a-byte-length
//# The byte length MUST be obtainable without materializing the UTF-8 encoding as a separate value, so that a size query an author expects to be cheap is not defined only in terms of an intermediate byte sequence.
fn string_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta t)` = the ground type-value `String` (`(intrinsic "String")` → `Ty::String`), so bare
    // `String` in type position IS the type — `(: x String)` reduces it, and a variant payload `(Named
    // String)` reads it as the payload type — exactly as `Bytes` carries `(meta t) = bytes-ty`. Member
    // access `(. String scalar-len)` still works: a record carrying `(meta t)` stays a record whose
    // FIELDS project (the `Bytes` module proves both — a `(meta t)` type-value AND member access
    // coexist; the earlier "a `(meta t)` breaks projection" note was mistaken, and left bare `String`
    // un-usable as a type: `(: s String)` faulted "found a non-type" and a String-payload variant was
    // misjudged nullary). The op schemes still use `(intrinsic "String")` for their `String` positions
    // (a bare name would mis-resolve inside the module being built).
    let ty_val = intrinsic_node(ast, "String");
    let t_field = meta_field(ast, "t", ty_val);
    let mut children = vec![head, t_field];
    // The LENGTH queries: each a `String → Int64` scheme (built fresh per field — a shared occurrence
    // must not be).
    for (name, prim) in [
        ("scalar-len", "str-scalar-len"),
        ("byte-len", "str-byte-len"),
    ] {
        let ty = string_to_int64_type(ast);
        let op = list_op_record(ast, prim, ty);
        let k = push_atom(ast, Leaf::Name(name.into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, k, op])
        });
    }
    // `at : String → Int64 → (Option String)` — the fallible scalar-indexed read.
    let at_ty = str_at_type(ast);
    let at_op = list_op_record(ast, "str-at", at_ty);
    let at_key = push_atom(ast, Leaf::Name("at".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, at_key, at_op])
    });
    // `scalar-at : String → Int64 → (Option Char)` — the fallible read of the CHAR (single Unicode scalar)
    // at a scalar position (the char-typed companion of `at`, which yields a one-scalar String). In range
    // → `(Some #\c)`, out → `None`. Addresses SCALAR values, not bytes. A constant string FOLDS.
    //= spec/capabilities/collections-and-text.md#a-string-s-scalars-are-addressable
    //# Reading a string's scalar at a position MUST be total, yielding an optional char that is present when the position is in bounds and absent when it is out of bounds, so that scalar access is fallible in the same way list and byte indexing are rather than trapping.
    let scalar_at_ty = str_scalar_at_type(ast);
    let scalar_at_op = list_op_record(ast, "str-scalar-at", scalar_at_ty);
    let scalar_at_key = push_atom(ast, Leaf::Name("scalar-at".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, scalar_at_key, scalar_at_op])
    });
    // `concat : String → String → String` — the total binary join (the compiler builds error messages
    // and export names this way). On two constant strings it FOLDS to their concatenation.
    let concat_ty = string_concat_type(ast);
    let concat_op = list_op_record(ast, "str-concat", concat_ty);
    let concat_key = push_atom(ast, Leaf::Name("concat".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, concat_key, concat_op])
    });
    // `slice : String → Int64 → Int64 → (Option String)` — the fallible sub-range read by SCALAR offsets
    // (`start`, `end`, half-open). In range (`0 <= start <= end <= scalar-len`) → `Some substring`, else
    // `None`. A constant string + constant bounds FOLD.
    let slice_ty = string_slice_type(ast);
    let slice_op = list_op_record(ast, "str-slice", slice_ty);
    let slice_key = push_atom(ast, Leaf::Name("slice".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, slice_key, slice_op])
    });
    // `to-bytes : String → Bytes` — the UTF-8 encoding of the string's scalars (the compiler encodes
    // export names as UTF-8 for wasm sections). A constant string FOLDS to a constant `Bytes` of its
    // UTF-8 bytes; consumed by `Bytes.len`/`Bytes.at`. Monomorphic (no type param).
    let to_bytes_ty = string_to_bytes_type(ast);
    let to_bytes_op = list_op_record(ast, "str-to-bytes", to_bytes_ty);
    let to_bytes_key = push_atom(ast, Leaf::Name("to-bytes".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, to_bytes_key, to_bytes_op])
    });
    // `from-bytes : Bytes → (Option String)` — the TOTAL UTF-8 DECODE (the inverse of `to-bytes`): a
    // well-formed byte sequence → `Some string`, ill-formed (invalid/overlong/surrogate) → `None`, never
    // a trap. A constant `Bytes` FOLDS via strict UTF-8 validation. `Some`/`None` distinguishes a
    // successful decode from ill-formed bytes (an ordinary value the program handles), and `from-bytes`
    // is the inverse of `to-bytes` on a well-formed sequence (decode-then-re-encode yields those bytes).
    //= spec/capabilities/collections-and-text.md#decoding-bytes-to-a-string-is-total-not-trapping
    //# Decoding a byte sequence to a string MUST yield a result that distinguishes a successful decode from a byte sequence that is not well-formed UTF-8, rather than trapping on ill-formed input, so that ill-formed bytes are an ordinary value a program handles rather than a halt.
    //= spec/capabilities/collections-and-text.md#decoding-bytes-to-a-string-is-total-not-trapping
    //# Encoding a string to its UTF-8 byte sequence MUST be the inverse of decoding a well-formed byte sequence, so that a string decoded from bytes and re-encoded yields those same bytes.
    let from_bytes_ty = string_from_bytes_type(ast);
    let from_bytes_op = list_op_record(ast, "str-from-bytes", from_bytes_ty);
    let from_bytes_key = push_atom(ast, Leaf::Name("from-bytes".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, from_bytes_key, from_bytes_op])
    });
    push_list(ast, children)
}

/// The `Char` module record — a record with one field per char OPERATION (`to-int`/`from-int`), reached
/// by member access `(. Char to-int)`. A NULLARY type like `String`: its `(meta t)` is the ground
/// `Ty::Char` (`(intrinsic "Char")`), so bare `Char` in type position IS the type, while the operation
/// fields still project. `to-int : Char → Int64` (total); `from-int : Int64 → (Option Char)` (fallible —
/// `None` for a surrogate / out-of-range integer). Both FOLD on a constant operand.
//= spec/capabilities/collections-and-text.md#a-char-converts-to-and-from-an-integer-totally
//# Converting a char to its integer scalar value MUST be total, because every char is a scalar value that has an integer code point.
//= spec/capabilities/collections-and-text.md#a-char-converts-to-and-from-an-integer-totally
//# Converting an integer to a char MUST yield an optional char that is absent when the integer is not a Unicode scalar value — outside `U+0000..=U+10FFFF` or within the surrogate range — so that an out-of-range integer is handled as data rather than producing a char that is not a valid scalar.
fn char_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let ty_val = intrinsic_node(ast, "Char");
    let t_field = meta_field(ast, "t", ty_val);
    let mut children = vec![head, t_field];
    // `to-int : Char → Int64` — the total scalar-value read.
    let to_int_ty = char_to_int_type(ast);
    let to_int_op = list_op_record(ast, "char-to-int", to_int_ty);
    let to_int_key = push_atom(ast, Leaf::Name("to-int".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, to_int_key, to_int_op])
    });
    // `from-int : Int64 → (Option Char)` — the fallible integer→char conversion.
    let from_int_ty = char_from_int_type(ast);
    let from_int_op = list_op_record(ast, "char-from-int", from_int_ty);
    let from_int_key = push_atom(ast, Leaf::Name("from-int".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, from_int_key, from_int_op])
    });
    push_list(ast, children)
}

/// The `Value` module record — the in-fold canonical encoder/decoder surface (R2). A plain record (no
/// `(meta t)` type-value: `Value` is a namespace, not a type) whose members project to the `value-encode`
/// / `value-decode` prims (`Core::ValueEncode`/`ValueDecode`, reusing the runtime `value-encode`/
/// `value-decode` heap ops IN-FOLD). `encode : ∀a. a → Bytes` is TOTAL — every value has a binary-AST
/// value-form, so encode never fails. `decode : ∀a. Bytes → Option a` is PARTIAL — the bytes may not
/// decode to `a`, so `Some` on success / `None` on a shape/type mismatch; `a` is grounded by the call-site
/// expected type (annotation / param / downstream), and an UNSOLVED `a` at the decode node DECLINES (the
/// emit needs a concrete descriptor — mirrors the empty-collection-needs-annotation discipline). This is
/// the operator-ruled binary-AST encoder (R2); `Value.encode`(Ast) equals the internal `ast-encode` bytes,
/// so it is THE single public canonical encoder (concierge ruling — no parallel `ast-encode` surface).
fn value_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let mut children = vec![head];
    // `encode : ∀a. a → Bytes` — total.
    let encode_ty = value_encode_type_lambda(ast);
    let encode_op = list_op_record(ast, "value-encode", encode_ty);
    let encode_key = push_atom(ast, Leaf::Name("encode".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, encode_key, encode_op])
    });
    // `decode : ∀a. Bytes → (Option a)` — partial; `a` grounded at the call site.
    let decode_ty = value_decode_type_lambda(ast);
    let decode_op = list_op_record(ast, "value-decode", decode_ty);
    let decode_key = push_atom(ast, Leaf::Name("decode".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, decode_key, decode_op])
    });
    push_list(ast, children)
}

/// The type-lambda `(fn (a) (-> a Bytes))` for `Value.encode` — `∀a. a → Bytes`, total.
fn value_encode_type_lambda(ast: &mut Arenas) -> StructId {
    let a = push_atom(ast, Leaf::Name("a".into()));
    let bytes = push_atom(ast, Leaf::Name("Bytes".into()));
    let body = arrow_type(ast, a, bytes); // (-> a Bytes)
    list_type_lambda(ast, body)
}

/// The type-lambda `(fn (a) (-> Bytes (Option a)))` for `Value.decode` — `∀a. Bytes → (Option a)`,
/// partial. `(Option a)` reduces via the generic `Option` prelude sum, carrying the target `a` the emit
/// reads to build the decode descriptor; an unsolved `a` at the decode node declines.
fn value_decode_type_lambda(ast: &mut Arenas) -> StructId {
    let bytes = push_atom(ast, Leaf::Name("Bytes".into()));
    let option_a = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let a = push_atom(ast, Leaf::Name("a".into()));
        push_list(ast, vec![option, a]) // (Option a)
    };
    let body = arrow_type(ast, bytes, option_a); // (-> Bytes (Option a))
    list_type_lambda(ast, body)
}

/// The `Symbol` module record (17-symbols) — a record whose `(meta t)` is the ground `Ty::Symbol`
/// (`(intrinsic "Symbol")`), so bare `Symbol` in type position IS the type (a NULLARY type like
/// `String`/`Char`), plus the operation fields reached by member access. `of : String → Symbol` interns
/// a string into a symbol; `to-string : Symbol → String` recovers its content. Both FOLD on a constant
/// operand (a constant symbol shares the underlying `Core::ConstStr` rep at type `Ty::Symbol`).
fn symbol_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let ty_val = intrinsic_node(ast, "Symbol");
    let t_field = meta_field(ast, "t", ty_val);
    let mut children = vec![head, t_field];
    // `of : String → Symbol` — intern a String into a Symbol.
    let of_ty = symbol_of_type(ast);
    let of_op = list_op_record(ast, "symbol-of", of_ty);
    let of_key = push_atom(ast, Leaf::Name("of".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, of_key, of_op])
    });
    // `to-string : Symbol → String` — recover a Symbol's content String.
    let to_string_ty = symbol_to_string_type(ast);
    let to_string_op = list_op_record(ast, "symbol-to-string", to_string_ty);
    let to_string_key = push_atom(ast, Leaf::Name("to-string".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, to_string_key, to_string_op])
    });
    push_list(ast, children)
}

/// The type `(fn () (-> String Symbol))` for `Symbol.of` — a ZERO-PARAM (monomorphic) `fn` wrapper so
/// `scheme_of` reads a SCHEME. Both type positions are the `(intrinsic …)` type node (→ the ground
/// `Ty`), not the module NAME.
fn symbol_of_type(ast: &mut Arenas) -> StructId {
    let string = intrinsic_node(ast, "String");
    let symbol = intrinsic_node(ast, "Symbol");
    let body = arrow_type(ast, string, symbol);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The `BigInt` module record — carries `(meta t) = (intrinsic "BigInt")` (so bare `BigInt` in type
/// position IS `Ty::BigInt`, like `Symbol`/`String`) PLUS the `of` conversion field. B1 adds `of`
/// (`∀a. (Int a) → BigInt`, the widening from any fixed-width integer); arithmetic + the reverse
/// checked narrowing arrive later.
fn bigint_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let ty_val = intrinsic_node(ast, "BigInt");
    let t_field = meta_field(ast, "t", ty_val);
    let mut children = vec![head, t_field];
    // `of : ∀a. (Int a) → BigInt` — the EXACT widening from a fixed-width integer (never traps; every
    // fixed-width value fits the unbounded type). The `(fn (a) …)` wrapper makes it a SCHEME generic over
    // the source width, exactly as `wrap`/the arithmetic operators are.
    let of_ty = bigint_of_type(ast);
    let of_op = list_op_record(ast, "bigint-of", of_ty);
    let of_key = push_atom(ast, Leaf::Name("of".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, of_key, of_op])
    });
    // `neg : BigInt → BigInt` — unary negation, TOTAL (arbitrary precision never overflows). The named
    // first-class form of prefix `(- e)`, lowered through the same `lower_negate` (`0 - e`).
    let neg_ty = bigint_neg_type(ast);
    let neg_op = list_op_record(ast, "neg", neg_ty);
    let neg_key = push_atom(ast, Leaf::Name("neg".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, neg_key, neg_op])
    });
    push_list(ast, children)
}

/// The `Rational` module record — `(meta t) = (intrinsic "Rational")` (so bare `Rational` in type
/// position IS `Ty::Rational`) PLUS the construction/conversion fields (B4-1):
///   `of      : ∀a b. (Int a) → (Int b) → Rational`  (numerator, denominator → normalized rational)
///   `of-int  : ∀a.   (Int a) → Rational`            (the whole rational `n/1`)
///   `value   : Rational → Rational`                 (identity — names the rational, symmetry with Qty)
/// A constant application folds in `lower` to a normalized `Core::ConstRational`; `of` traps on a zero
/// denominator. Arithmetic + comparison over rationals ride the ordinary `+`/`-`/`*`/`/`/`<`/`=` operators
/// (dispatched on a `Ty::Rational` operand in `lower`/`infer`), not module fields.
fn rational_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let ty_val = intrinsic_node(ast, "Rational");
    let t_field = meta_field(ast, "t", ty_val);
    let mut children = vec![head, t_field];
    // `of : ∀a b. (Int a) → (Int b) → Rational`.
    let of_ty = rational_of_type(ast);
    let of_op = list_op_record(ast, "rational-of", of_ty);
    let of_key = push_atom(ast, Leaf::Name("of".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, of_key, of_op])
    });
    // `of-int : ∀a. (Int a) → Rational`.
    let of_int_ty = rational_of_int_type(ast);
    let of_int_op = list_op_record(ast, "rational-of-int", of_int_ty);
    let of_int_key = push_atom(ast, Leaf::Name("of-int".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, of_int_key, of_int_op])
    });
    // `value : Rational → Rational` (identity).
    let value_ty = rational_value_type(ast);
    let value_op = list_op_record(ast, "rational-value", value_ty);
    let value_key = push_atom(ast, Leaf::Name("value".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, value_key, value_op])
    });
    // `neg : Rational → Rational` — unary negation, TOTAL (negates the numerator; exact, never traps).
    // The named first-class form of prefix `(- e)`, lowered through the same `lower_negate` (`0 - e`).
    // Reuses the `(fn () (-> Rational Rational))` shape of `value`.
    let neg_ty = rational_value_type(ast);
    let neg_op = list_op_record(ast, "neg", neg_ty);
    let neg_key = push_atom(ast, Leaf::Name("neg".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, neg_key, neg_op])
    });
    // `numerator : Rational → BigInt` / `denominator : Rational → BigInt` — read the components of the
    // normalized (lowest-terms, denominator > 0) pair. BigInt-valued (either can exceed i64); floor/round/
    // integer-projection compose in Cadenza on top.
    let numerator_ty = rational_to_bigint_type(ast);
    let numerator_op = list_op_record(ast, "rational-num", numerator_ty);
    let numerator_key = push_atom(ast, Leaf::Name("numerator".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, numerator_key, numerator_op])
    });
    let denominator_ty = rational_to_bigint_type(ast);
    let denominator_op = list_op_record(ast, "rational-den", denominator_ty);
    let denominator_key = push_atom(ast, Leaf::Name("denominator".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, denominator_key, denominator_op])
    });
    // `truncate : Rational → Int64` — the exact integer part TOWARD ZERO (`7/2 → 3`, `-7/2 → -3`). Unlike
    // `numerator`/`denominator` (BigInt-valued, since either component can exceed i64), the integer part of
    // a rational is a single value that MUST land in a fixed width to be useful (MIDI ticks, indices), so
    // this narrows to `Int64` with the checked-narrow TRAP on overflow (never a silent truncation). It is
    // NOT a new runtime op: it lowers as a DERIVATION over the existing `numerator`/`denominator` +
    // BigInt truncating-division + the checked `Int64.of` narrowing (all hash-neutral). floor/ceil/round
    // (which add a conditional ±1 off the remainder sign) compose on top in later increments.
    let truncate_ty = rational_to_int64_type(ast);
    let truncate_op = list_op_record(ast, "rational-truncate", truncate_ty);
    let truncate_key = push_atom(ast, Leaf::Name("truncate".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, truncate_key, truncate_op])
    });
    // `floor : Rational → Int64` (toward −∞) and `ceil : Rational → Int64` (toward +∞) — the other two
    // exact integer projections, each `truncate` adjusted by ±1 off the remainder sign. Like `truncate`,
    // NOT new runtime ops: they lower as DERIVATIONS over numerator/denominator + BigInt divmod + a
    // remainder-sign conditional + the checked `Int64.of` narrowing (hash-neutral). `floor(-7/2) = -4`
    // (toward −∞, distinct from `truncate`'s −3); `ceil(7/2) = 4` (toward +∞).
    let floor_ty = rational_to_int64_type(ast);
    let floor_op = list_op_record(ast, "rational-floor", floor_ty);
    let floor_key = push_atom(ast, Leaf::Name("floor".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, floor_key, floor_op])
    });
    let ceil_ty = rational_to_int64_type(ast);
    let ceil_op = list_op_record(ast, "rational-ceil", ceil_ty);
    let ceil_key = push_atom(ast, Leaf::Name("ceil".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, ceil_key, ceil_op])
    });
    // `round : Rational → Int64` — round to the NEAREST integer, ties HALF-AWAY-FROM-ZERO (`1/2 → 1`,
    // `-1/2 → -1`, `3/2 → 2`, `5/2 → 3`). The last of the exact integer projections. Like the others, NOT a
    // new runtime op: a DERIVATION over numerator/denominator + BigInt divmod + a `2·|rem| ≥ denominator`
    // tie test + the checked `Int64.of` narrowing (hash-neutral). The half-away tie rule is the settled
    // ruling (symmetric snapping for MIDI/ticks).
    let round_ty = rational_to_int64_type(ast);
    let round_op = list_op_record(ast, "rational-round", round_ty);
    let round_key = push_atom(ast, Leaf::Name("round".into()));
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, round_key, round_op])
    });
    push_list(ast, children)
}

/// `(fn (a b) (-> (Int a) (-> (Int b) Rational)))` for `Rational.of` — generic over BOTH the numerator
/// and denominator source widths, curried, with the fixed result `Rational`. A non-integer operand fails
/// to unify with `(Int _)` (CDZ0301).
fn rational_of_type(ast: &mut Arenas) -> StructId {
    let int_a = {
        let int = push_atom(ast, Leaf::Name("Int".into()));
        let a = push_atom(ast, Leaf::Name("a".into()));
        push_list(ast, vec![int, a])
    };
    let int_b = {
        let int = push_atom(ast, Leaf::Name("Int".into()));
        let b = push_atom(ast, Leaf::Name("b".into()));
        push_list(ast, vec![int, b])
    };
    let rational = intrinsic_node(ast, "Rational");
    let inner = arrow_type(ast, int_b, rational);
    let body = arrow_type(ast, int_a, inner);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let b_param = push_atom(ast, Leaf::Name("b".into()));
    let params = push_list(ast, vec![a_param, b_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// `(fn (a) (-> (Int a) Rational))` for `Rational.of-int` — the whole rational `n/1` from a fixed-width
/// integer, generic over the source width.
fn rational_of_int_type(ast: &mut Arenas) -> StructId {
    let int_a = {
        let int = push_atom(ast, Leaf::Name("Int".into()));
        let a = push_atom(ast, Leaf::Name("a".into()));
        push_list(ast, vec![int, a])
    };
    let rational = intrinsic_node(ast, "Rational");
    let body = arrow_type(ast, int_a, rational);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// `(fn () (-> Rational Rational))` for `Rational.value` — the identity that names a rational's type.
fn rational_value_type(ast: &mut Arenas) -> StructId {
    let rational = intrinsic_node(ast, "Rational");
    let rational2 = intrinsic_node(ast, "Rational");
    let body = arrow_type(ast, rational, rational2);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// `(fn () (-> BigInt BigInt))` for `BigInt.neg` — unary negation, TOTAL (arbitrary precision never
/// overflows). The zero-param `fn` wrapper makes `scheme_of` read a monomorphic SCHEME (like
/// `rational_value_type`); `(meta apply)` = the `neg` intrinsic, lowered through `lower_negate` (`0 - e`).
fn bigint_neg_type(ast: &mut Arenas) -> StructId {
    let a = intrinsic_node(ast, "BigInt");
    let b = intrinsic_node(ast, "BigInt");
    let body = arrow_type(ast, a, b);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// `(fn () (-> Rational BigInt))` for `Rational.numerator` / `Rational.denominator` — read the numerator /
/// denominator of a rational as a `BigInt` (NOT `Int64`: either can exceed i64, since a Rational is a
/// numerator/denominator pair of big-integers). The zero-param `fn` wrapper makes `scheme_of` read it as a
/// SCHEME over the monomorphic arrow (like `rational_value_type` / `string_to_int64_type`), so member
/// access `(. Rational numerator)` resolves as an applyable op, not a bare type-value.
fn rational_to_bigint_type(ast: &mut Arenas) -> StructId {
    let rational = intrinsic_node(ast, "Rational");
    let bigint = intrinsic_node(ast, "BigInt");
    let body = arrow_type(ast, rational, bigint);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// `(fn () (-> Rational Int64))` for `Rational.truncate` (and the later `floor`/`ceil`/`round`) — the
/// integer PROJECTION of a rational to a fixed `Int64`, checked-narrow (traps on overflow). Unlike
/// `rational_to_bigint_type` (num/den can exceed i64), the integer part is a single small value that
/// narrows to `Int64`. The zero-param `fn` wrapper makes `scheme_of` read a SCHEME over the monomorphic
/// arrow (like `string_to_int64_type`), so `(. Rational truncate)` resolves as an applyable op. The result
/// `Int64` is spelled by the NAME node (like `string_to_int64_type`), not an intrinsic.
fn rational_to_int64_type(ast: &mut Arenas) -> StructId {
    let rational = intrinsic_node(ast, "Rational");
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let body = arrow_type(ast, rational, int64);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type-lambda `(fn (a) (-> (Int a) BigInt))` for `BigInt.of` — generic over the SOURCE integer
/// width `a` (like `wrap`'s source), with the fixed result `BigInt`. `infer` reads it as the scheme
/// `∀a. (Int a) → BigInt`, so any fixed-width integer converts and the generic application rule fills
/// the result `Ty::BigInt`; a non-integer source fails to unify with `(Int a)` (CDZ0301).
fn bigint_of_type(ast: &mut Arenas) -> StructId {
    let int_a = {
        let int = push_atom(ast, Leaf::Name("Int".into()));
        let a = push_atom(ast, Leaf::Name("a".into()));
        push_list(ast, vec![int, a])
    };
    let bigint = intrinsic_node(ast, "BigInt");
    let body = arrow_type(ast, int_a, bigint);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> Symbol String))` for `Symbol.to-string` — the inverse of `Symbol.of`.
fn symbol_to_string_type(ast: &mut Arenas) -> StructId {
    let symbol = intrinsic_node(ast, "Symbol");
    let string = intrinsic_node(ast, "String");
    let body = arrow_type(ast, symbol, string);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The `Unit` module record — the ground type `Ty::Unit` (via `(meta t) = (intrinsic Unit)`, so bare
/// `Unit` in TYPE position IS `Ty::Unit`, e.g. `(-> Unit Int64)`) EXTENDED with the unit-BUILDER fields
/// `one` and `base`, reached by member access `(. Unit one)` / `(. Unit base)`. WARNING: `Unit` plays TWO roles
/// — the `unit` value's TYPE and the units module — which coexist because a record carries both a `(meta
/// t)` and member fields (exactly as `Bytes`/`String` do; the `Bytes` module proved a `(meta t)` and
/// member access coexist). `Unit.one` is the dimensionless unit (the group identity); `(Unit.base
/// #"meter")` names a base dimension. Each field is a builder record whose `(meta apply)` is the builder
/// prim; a unit is a compile-time value reduced by `eval`. `Unit.*`/`Unit./`/`Unit.^` are registered as
/// TOP-LEVEL names, not fields (the reader keeps them bare — `^`/`*`/`/` aren't alphabetic).
fn unit_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta t) = Unit` — the ground type-value, so `Unit` in type position stays `Ty::Unit` (the
    // effect-op `(-> Unit Int64)` and every other `Unit`-as-type use keep resolving). This is what makes
    // the module a superset of the plain ground-type record it replaces (it ADDS fields, keeps the type).
    let ty_val = intrinsic_node(ast, "Unit");
    let t_field = meta_field(ast, "t", ty_val);
    let mut children = vec![head, t_field];
    // `one` — the dimensionless unit (applying it, or using it bare, yields the group identity).
    let one_field = push_atom(ast, Leaf::Name("one".into()));
    let one_op = unit_op_ctor(ast, "unit-one");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, one_field, one_op])
    });
    // `base` — a base dimension named by a symbol: `(Unit.base #"meter")`.
    let base_field = push_atom(ast, Leaf::Name("base".into()));
    let base_op = unit_op_ctor(ast, "unit-base");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, base_field, base_op])
    });
    // `prefix` — scale a unit by a prefix's factor: `(Unit.prefix kilo (Unit.base #"meter"))`. Member
    // access (`prefix` is alphabetic → `(. Unit prefix)`), so a field, not a top-level name (unlike
    // `Unit.*`/`^`).
    let prefix_field = push_atom(ast, Leaf::Name("prefix".into()));
    let prefix_op = unit_op_ctor(ast, "unit-prefix");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, prefix_field, prefix_op])
    });
    // `of` — name a FAMILY unit from the registry: `(Unit.of #"foot")` = length at foot's scale to
    // meter. Member access (`(. Unit of)`), a field. Consults `Db::unit_families`.
    let of_field = push_atom(ast, Leaf::Name("of".into()));
    let of_op = unit_op_ctor(ast, "unit-of");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, of_field, of_op])
    });
    // `in` — EXPLICIT conversion of a quantity to a chosen unit: `(Unit.in meter (Qty.of 3.0 km))`.
    // Member access (`(. Unit in)`), a field. Takes a target unit + a quantity.
    let in_field = push_atom(ast, Leaf::Name("in".into()));
    let in_op = unit_op_ctor(ast, "unit-in");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, in_field, in_op])
    });
    // `define` — DECLARE a family unit: `(Unit.define #"furlong" (Unit.of #"foot") 660 1)`. As a value it
    // reduces to the defined unit (`base` scaled by num/den); its registration is a load-time scan.
    let define_field = push_atom(ast, Leaf::Name("define".into()));
    let define_op = unit_op_ctor(ast, "unit-define");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, define_field, define_op])
    });
    push_list(ast, children)
}

/// A PREFIX record — a prelude binding (`kilo`, `milli`, `mebi`, …) carrying its exact scale factor as a
/// `(meta scale)` channel holding `(num den)` (a machine-integer ratio: `kilo`=1000/1, `milli`=1/1000,
/// `mebi`=1048576/1). `Unit.prefix` reads this ratio and applies it to a unit via `Unit::scaled`. The
/// scale is compile-time metadata, NOT a runtime `Rational` — so prefixes (and the conversions they
/// drive) need no arbitrary-precision arithmetic. A prefix is an ordinary shadowable name. The factor is
/// an EXACT `(num, den)` ratio (a decimal multiple like `kilo`/`milli` or a binary one like `mebi`), so a
/// prefixed unit scales to its base without approximation.
//= spec/capabilities/units-of-measure.md#a-scaled-unit-is-a-unit-scaled-by-an-exact-factor
//# A unit prefixed or otherwise scaled by an exact factor — a decimal multiple such as kilo or milli, or a binary multiple such as kibi or mebi — MUST itself be a unit of the same dimension as the unit it scales, differing only by that exact factor.
//= spec/capabilities/units-of-measure.md#a-scaled-unit-is-a-unit-scaled-by-an-exact-factor
//# A scale factor MUST be an exact value, so that a prefixed unit converts to its base without approximation.
fn prefix_record(ast: &mut Arenas, num: i64, den: i64) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let n = push_atom(
        ast,
        Leaf::Int {
            value: IntValue::from_i64(num),
            radix: Radix::Dec,
        },
    );
    let d = push_atom(
        ast,
        Leaf::Int {
            value: IntValue::from_i64(den),
            radix: Radix::Dec,
        },
    );
    let ratio = push_list(ast, vec![n, d]);
    let scale_field = meta_field(ast, "scale", ratio);
    push_list(ast, vec![head, scale_field])
}

/// A family unit's conversion: its DIMENSION (a `(base-name, exponent)` list — one entry atomic, several
/// derived) and its exact scale `(num, den)` to that dimension's reference. The value in the family
/// registry.
///
/// The scale is an EXACT machine-integer ratio to the dimension's reference unit, so a conversion between
/// two units of one dimension is an exact ratio (`(qn·td)/(qd·tn)`, see `lower_unit_in`) — exact when the
/// inner numeric type is exact (Int), losing precision only where the inner type is itself inexact (Float).
//= spec/capabilities/units-of-measure.md#a-unit-carries-an-exact-scale-to-its-dimension-s-reference
//# Each unit of a dimension MUST carry an exact scale relating it to that dimension's reference unit, so that a conversion between two units of one dimension is an exact ratio rather than an approximation.
//= spec/capabilities/units-of-measure.md#a-unit-carries-an-exact-scale-to-its-dimension-s-reference
//# A conversion between two units of the same dimension MUST preserve the exact value when the underlying numeric type is exact, losing precision only where the underlying numeric type is itself inexact.
pub type UnitConversion = (Vec<(String, i64)>, i128, i128);

/// One family-registration ROW: `(name, dimension, scale-num, scale-den)`, where dimension is a borrowed
/// `(base, exponent)` slice. The input shape [`register_families`] consumes (a built-in table row or a
/// future user declaration).
pub type FamilyRow<'a> = (&'a str, &'a [(&'a str, i64)], i128, i128);

/// The FAMILY-OF-MEASURE registry — the ONE place the built-in family vocabulary lives (the analogue of
/// `Prim::from_name` for prim spellings). Maps each named unit to its [`UnitConversion`] (dimension +
/// exact machine-integer scale to that dimension's reference). `(Unit.of #"foot")` looks its name up
/// here and builds `Unit.base("meter").scaled(381, 1250)`; `(Unit.of #"mbps")` builds the derived
/// `byte/second` dimension scaled by 10⁶/8. Every scale fits a machine int, so a family unit
/// auto-converts over Float/Int with NO bignum. A build MAY supply its own families; this is the SI +
/// common imperial/information set plus common data-rate and frequency units.
///
/// A dimension GROUPS its named units: several rows share one dimension (meter, millimeter, foot, inch
/// are all `length`), so they name measures of one dimension rather than each being distinct. Two units
/// of the same dimension are interconvertible (each carries an exact scale to the shared reference, so
/// `Unit.in` composes their ratio); two units of DIFFERENT dimension are not (the `same_dimension` gate
/// rejects — `meter + second` is CDZ0501).
//= spec/capabilities/units-of-measure.md#a-dimension-groups-interconvertible-units
//# A dimension MUST admit more than one named unit, so that several units — such as a meter, a millimeter, and an inch — name measures of one dimension rather than each being a distinct dimension.
//= spec/capabilities/units-of-measure.md#a-dimension-groups-interconvertible-units
//# Two units of the same dimension MUST be interconvertible, and two units of different dimension MUST NOT be.
pub fn unit_families() -> BTreeMap<String, UnitConversion> {
    // Each row: `(name, dimension, num, den)`. The DIMENSION is a `(base, exponent)` list — one entry for
    // an ATOMIC dimension (length/time/information), several for a DERIVED one (a rate = information/time
    // is `[("byte", 1), ("second", -1)]`). The scale `num/den` is the unit's exact ratio to that
    // dimension's REFERENCE (meter/second/byte for the atomic ones; byte/second for a rate). A named unit
    // can thus denote a DERIVED dimension — `mbps` is a unit of `information/time` you convert to/from
    // `byte/second` — not only an atomic one (`units-of-measure.md` §A Dimension Groups Interconvertible
    // Units). All scales fit a machine int, so conversion is bignum-free over Float/Int.
    let rows: &[FamilyRow] = &[
        // length — reference `meter`.
        ("meter", &[("meter", 1)], 1, 1),
        ("millimeter", &[("meter", 1)], 1, 1000),
        ("centimeter", &[("meter", 1)], 1, 100),
        ("kilometer", &[("meter", 1)], 1000, 1),
        ("inch", &[("meter", 1)], 127, 5000),
        ("foot", &[("meter", 1)], 381, 1250),
        ("mile", &[("meter", 1)], 201168, 125),
        // time — reference `second`.
        ("second", &[("second", 1)], 1, 1),
        ("millisecond", &[("second", 1)], 1, 1000),
        ("minute", &[("second", 1)], 60, 1),
        ("hour", &[("second", 1)], 3600, 1),
        // information — reference `byte`. Decimal (kB/MB/GB) and binary (KiB/MiB/GiB) are DISTINCT scales.
        ("byte", &[("byte", 1)], 1, 1),
        ("bit", &[("byte", 1)], 1, 8), // a bit is 1/8 byte
        ("kilobyte", &[("byte", 1)], 1000, 1),
        ("megabyte", &[("byte", 1)], 1_000_000, 1),
        ("gigabyte", &[("byte", 1)], 1_000_000_000, 1),
        ("kibibyte", &[("byte", 1)], 1024, 1),
        ("mebibyte", &[("byte", 1)], 1_048_576, 1),
        ("gibibyte", &[("byte", 1)], 1_073_741_824, 1),
        // DERIVED: DATA RATE = information / time, reference `byte/second`. `bps`/`kbps`/`mbps` are
        // BIT-per-second (the networking convention), so their byte/second scale carries the 1/8. A named
        // rate unit you convert to/from `byte/second` — the "bytes over time as its own family" case.
        ("byte-per-second", &[("byte", 1), ("second", -1)], 1, 1),
        ("bps", &[("byte", 1), ("second", -1)], 1, 8),
        ("kbps", &[("byte", 1), ("second", -1)], 1000, 8),
        ("mbps", &[("byte", 1), ("second", -1)], 1_000_000, 8),
        ("gbps", &[("byte", 1), ("second", -1)], 1_000_000_000, 8),
        // DERIVED: FREQUENCY = 1 / time, reference `hertz` (= second⁻¹). Its scale is 1/1 at the reference.
        ("hertz", &[("second", -1)], 1, 1),
        ("kilohertz", &[("second", -1)], 1000, 1),
        ("megahertz", &[("second", -1)], 1_000_000, 1),
        ("gigahertz", &[("second", -1)], 1_000_000_000, 1),
        // ANGLE — `radian` and `degree` are SEPARATE base dimensions, NOT one angle dimension, because
        // their conversion is IRRATIONAL (180° = π rad, and π has no exact Rational). Every unit here keys
        // to an EXACT rational ratio to its dimension reference (inch = 127/5000 meter); forcing rad↔deg
        // into one dimension would need an approximate π ratio and BREAK that exact-Rational invariant. As
        // distinct dimensions each is exact WITHIN itself (`5 degree + 90 degree = 95 degree`; `1 radian +
        // 1 radian = 2 radian`), and mixing them (`degree + radian`) correctly rejects CDZ0501 — honest,
        // since they are not exactly interconvertible. A program needing rad↔deg does it explicitly at the
        // f64/approximate boundary (where `sin`/`cos` already live), never silently through this registry.
        // First-class per the operator ruling (CAD revolve/rotate angles get their own unit family, like
        // meter/km — v-cad's Vec3→Qty[radian|degree] retype). See the reply on the v-cad ask + option (a).
        ("radian", &[("radian", 1)], 1, 1),
        ("degree", &[("degree", 1)], 1, 1),
    ];
    let mut m = match register_families(rows.iter().copied()) {
        Ok(m) => m,
        // A conflict in the BUILT-IN table is a compiler invariant violation (a typo/duplicate in the
        // list above), not a user error — fail loudly at construction. When a USER family-declaration
        // surface lands, it routes through `register_families` too and a conflict THERE is the
        // user-facing rejection (`register_families` returns the offending name for a coded diagnostic).
        Err(name) => {
            panic!("built-in unit family `{name}` registered with conflicting conversions")
        }
    };
    // Common ENGLISH PLURAL spellings of the atomic units — AND the standard SI/metric ABBREVIATIONS —
    // resolve to the SAME conversion as their canonical singular. The ML quantity-literal surface is
    // written for natural language AND for the terse symbols a calculator user reaches for (`4 feet`,
    // `5 meters`, `3 inches`, but equally `5 km`, `100 m`, `250 ms`; the parser's own
    // `quantity_literal_desugars` test builds `(Unit.of #"feet")`, and `5 km` desugars to `(Unit.of
    // #"km")` identically), so a plural OR an abbreviation MUST name the same unit as its canonical
    // spelling rather than fail as unknown. Both plurals (IRREGULAR — `foot`→`feet`, `inch`→`inches`;
    // `hertz` invariant) and abbreviations (`km`, `cm`, `Hz`) are explicit DATA here, not a computed
    // stemming rule. Each alias REUSES its canonical row's `UnitConversion` (one source of truth —
    // editing `foot`'s scale flows to `feet` AND `ft` for free); an alias never collides with a
    // canonical name, so the uniqueness invariant holds.
    //
    // Abbreviations OMITTED and why: `in` (inch) is the `in` KEYWORD — it lexes as a reserved word, not
    // a unit ident, so `5 in` is a parse error, not an unknown unit; use `inch`/`inches`. MASS symbols
    // (`kg`, `g`, `mg`) have NO canonical row — there is no mass dimension in the built-in families yet
    // — so they cannot alias anything; adding them needs a `gram` family first (deferred, told v-guide).
    const ALIASES: &[(&str, &str)] = &[
        // length — plurals
        ("meters", "meter"),
        ("millimeters", "millimeter"),
        ("centimeters", "centimeter"),
        ("kilometers", "kilometer"),
        ("inches", "inch"),
        ("feet", "foot"),
        ("miles", "mile"),
        // length — abbreviations
        ("m", "meter"),
        ("mm", "millimeter"),
        ("cm", "centimeter"),
        ("km", "kilometer"),
        ("ft", "foot"),
        ("mi", "mile"),
        // time — plurals
        ("seconds", "second"),
        ("milliseconds", "millisecond"),
        ("minutes", "minute"),
        ("hours", "hour"),
        // time — abbreviations (`min`/`h` are the conventional short forms; `hr` also common)
        ("s", "second"),
        ("ms", "millisecond"),
        ("min", "minute"),
        ("h", "hour"),
        ("hr", "hour"),
        // information — plurals
        ("bytes", "byte"),
        ("bits", "bit"),
        ("kilobytes", "kilobyte"),
        ("megabytes", "megabyte"),
        ("gigabytes", "gigabyte"),
        ("kibibytes", "kibibyte"),
        ("mebibytes", "mebibyte"),
        ("gibibytes", "gibibyte"),
        // information — abbreviations (decimal `kB/MB/GB` vs binary `KiB/MiB/GiB`, matching the rows)
        ("B", "byte"),
        ("kB", "kilobyte"),
        ("MB", "megabyte"),
        ("GB", "gigabyte"),
        ("KiB", "kibibyte"),
        ("MiB", "mebibyte"),
        ("GiB", "gibibyte"),
        // frequency — abbreviations (`hertz` has no plural alias; `Hz` is the SI symbol)
        ("Hz", "hertz"),
        ("kHz", "kilohertz"),
        ("MHz", "megahertz"),
        ("GHz", "gigahertz"),
        // angle — plurals + the conventional `rad`/`deg` abbreviations (the `°` glyph is NOT added: a
        // quantity-literal unit is a bare-safe identifier the parser re-lexes as one `Ident`, and `°` is
        // not an identifier char — a program wanting it would need surface-lexer support, out of scope here).
        ("radians", "radian"),
        ("degrees", "degree"),
        ("rad", "radian"),
        ("deg", "degree"),
    ];
    for (alias, canonical) in ALIASES {
        let conv = m
            .get(*canonical)
            .unwrap_or_else(|| panic!("plural alias `{alias}` names missing unit `{canonical}`"))
            .clone();
        m.insert((*alias).to_string(), conv);
    }
    m
}

/// Register a set of family units into the name → `(dimension, num, den)` map, ENFORCING that a name
/// maps to ONE conversion: registering the same name TWICE with a DIFFERENT dimension or scale is an
/// error (returns the offending name), so a name's conversion is a well-defined function
/// (`units-of-measure.md` §A Named Unit's Conversion Is Unique). A duplicate that AGREES is idempotent.
/// This is the gate the built-in table is validated through, and the one a future user-declared family
/// flows through — a conflicting user registration becomes the coded rejection `Code::UnitConflict`
/// (CDZ0502). The dimension is normalized (a `BTreeMap`-backed comparison), so two spellings of one
/// dimension agree.
pub fn register_families<'a>(
    rows: impl Iterator<Item = FamilyRow<'a>>,
) -> Result<BTreeMap<String, UnitConversion>, String> {
    let mut m: BTreeMap<String, UnitConversion> = BTreeMap::new();
    for (name, dim, num, den) in rows {
        // Canonicalize the dimension (sorted, zero-exponent bases dropped) so the conflict compare is by
        // VALUE not spelling order.
        let mut dim_map: std::collections::BTreeMap<String, i64> =
            std::collections::BTreeMap::new();
        for (base, exp) in dim {
            if *exp != 0 {
                dim_map.insert(base.to_string(), *exp);
            }
        }
        let dim_vec: Vec<(String, i64)> = dim_map.into_iter().collect();
        let entry = (dim_vec, num, den);
        match m.get(name) {
            // Already registered with a DIFFERENT dimension or scale — a genuine conflict.
            Some(existing) if *existing != entry => return Err(name.to_string()),
            // Absent, or a duplicate that AGREES (idempotent) — record it.
            _ => {
                m.insert(name.to_string(), entry);
            }
        }
    }
    Ok(m)
}

/// A UNIT-builder record `(record ((meta apply) (intrinsic PRIM)))` — `Unit.one`/`Unit.base`/`Unit.*`/…
/// A unit is a compile-time value the evaluator BUILDS (reduced by `eval`), so a unit builder carries
/// ONLY a `(meta apply)` builder prim — no `(meta t)` scheme (a unit is not an HM-typed runtime value,
/// like a type constructor is not). The same shape as [`ctor_record`], named distinctly for clarity.
fn unit_op_ctor(ast: &mut Arenas, prim: &str) -> StructId {
    ctor_record(ast, prim)
}

/// The `Qty` module record — `of` (attach a unit) and `value` (recover the numeric, discard the unit),
/// reached by member access. Each field is a record whose `(meta apply)` is the quantity prim; their
/// RESULT type is unit-dependent (`Qty.of`'s result unit is the VALUE of its 2nd argument), so it is
/// computed in `infer::apply_type`'s dedicated arms rather than by a static `(meta t)` scheme — hence no
/// `(meta t)` here (like the compound value constructors `tuple`/`record`/`list`, whose type is their
/// arguments' shape, not a fixed scheme).
///
/// The dimension a `(Qty T u)` carries is a TYPE-LEVEL fact, checked during inference then ERASED before
/// emission: `Qty.of`/`Qty.value` lower to their numeric value argument (the unit index leaves no
/// runtime trace, `eval` never reaches the backend with a `Ty::Qty`), so attaching a unit changes
/// neither the numeric byte form nor the runtime representation, and no unit/dimension is in the emitted
/// component.
//= spec/capabilities/units-of-measure.md#dimensions-are-checked-then-erased
//# Dimensional consistency MUST be checked at compile time.
//= spec/capabilities/units-of-measure.md#dimensions-are-checked-then-erased
//# A unit or dimension MUST NOT appear in the emitted component, being erased after checking.
//= spec/capabilities/units-of-measure.md#dimensional-analysis-does-not-alter-the-numeric-core
//# Attaching a unit to a numeric value MUST NOT change the value's numeric byte form.
//= spec/capabilities/units-of-measure.md#dimensional-analysis-does-not-alter-the-numeric-core
//# Attaching a unit to a numeric value, or combining values that already share a unit, MUST NOT change the value's runtime behavior.
fn qty_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta apply) = Qty` — the quantity-TYPE constructor, so `(Qty Float64 u)` in TYPE position builds
    // `Ty::Qty` via the ordinary `typeval_of` path (an annotation `(: e (Qty T u))`), exactly as `(List
    // T)` reduces via `List`'s `(meta apply)`. The value constructor is `Qty.of` (a field); this channel
    // is the type-constructor role.
    let ctor = intrinsic_node(ast, "Qty");
    let apply_field = meta_field(ast, "apply", ctor);
    let mut children = vec![head, apply_field];
    let of_field = push_atom(ast, Leaf::Name("of".into()));
    let of_op = ctor_record(ast, "qty-of");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, of_field, of_op])
    });
    let value_field = push_atom(ast, Leaf::Name("value".into()));
    let value_op = ctor_record(ast, "qty-value");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, value_field, value_op])
    });
    // `pow` — raise a quantity to a compile-time non-negative integer power, composing the unit like
    // `Unit.^`: `(Qty.pow (Qty.of 3.0 meter) 2)` = `9.0 : (Qty Float64 meter²)`. The exponent is read
    // off the second argument at type/lower time (not an HM variable), so `pow` is a plain field op.
    let pow_field = push_atom(ast, Leaf::Name("pow".into()));
    let pow_op = ctor_record(ast, "qty-pow");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, pow_field, pow_op])
    });
    // `unit` — extract a quantity's UNIT as a compile-time unit value: `(Qty.of new (Qty.unit y))` makes
    // a new quantity in `y`'s unit without re-spelling it. It IS a unit expression (reduces via
    // `unit_of`, reading `y`'s solved type), so it is used in unit position like `(Unit.base …)`.
    let unit_field = push_atom(ast, Leaf::Name("unit".into()));
    let unit_op = ctor_record(ast, "qty-unit");
    children.push({
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, unit_field, unit_op])
    });
    push_list(ast, children)
}

/// The `Type` module record — a namespace for the type-REFLECTION operations. `of` (`(meta apply) =
/// TypeOf`) reduces `(Type.of e)` to the type-VALUE of `e`'s inferred type, so a program can name a
/// value's type and reuse it in a type position (`(: x (Type.of y))`). `eq` (`(meta apply) = TypeEq`)
/// folds `(Type.eq a b)` to the constant `Bool` of two type-values' exact structural equality, so a
/// program can BRANCH on types at compile time (`(if (Type.eq (Type.of x) Int64) …)`). Unlike `Qty`, the
/// module itself is NOT a type constructor (no top-level `(meta apply)`) — it is only a namespace; `Type`
/// in a bare type position is not a type (a value's type-of is `Ty::Type`, spelled only by reflection).
/// The type `(fn () (-> Bytes Bytes))` for `Blake3.of` — a monomorphic arrow taking a `Bytes` and
/// returning its 32-byte blake3 digest as a `Bytes`. A ZERO-PARAM `fn` wrapper (see
/// [`string_to_bytes_type`] — the wrapper makes `scheme_of` read a SCHEME, not a bare type-value). Both
/// `Bytes` positions are the `(intrinsic bytes-ty)` type-value directly (a bare `Bytes` name would
/// mis-resolve inside the module being built).
fn blake3_of_type(ast: &mut Arenas) -> StructId {
    let bytes_in = intrinsic_node(ast, "bytes-ty");
    let bytes_out = intrinsic_node(ast, "bytes-ty");
    let body = arrow_type(ast, bytes_in, bytes_out); // (-> Bytes Bytes)
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The `Blake3` module record — a record of hashing OPERATIONS reached by member access (`(. Blake3
/// of)`). It carries NO `(meta t)`: `Blake3` is NOT a type, only a namespace (unlike `Bytes`/`String`,
/// whose bare name IS a type). One field `of : Bytes → Bytes` — the blake3 content hash. NAMES THE
/// ALGORITHM (design-compiler-primitives.md D5): a future digest is a DIFFERENT named module/function,
/// never a silent change to a generic `Hash`. The operation is ENTIRELY GENERIC (raw bytes → digest, no
/// tag/prefix — all domain separation is userspace, D7). Same monomorphic-op shape as the `Bytes`/`String`
/// operations (`list_op_record` with a `(fn () (-> …))` scheme + `(meta apply) = (intrinsic blake3-of)`).
fn blake3_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let of_type = blake3_of_type(ast);
    let of_op = list_op_record(ast, "blake3-of", of_type);
    let of_field = push_atom(ast, Leaf::Name("of".into()));
    let of = {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, of_field, of_op])
    };
    push_list(ast, vec![head, of])
}

fn type_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let of_field = push_atom(ast, Leaf::Name("of".into()));
    let of_op = ctor_record(ast, "type-of");
    let of = {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, of_field, of_op])
    };
    let eq_field = push_atom(ast, Leaf::Name("eq".into()));
    let eq_op = ctor_record(ast, "type-eq");
    let eq = {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, eq_field, eq_op])
    };
    // `ast` / `ast-generic` (`(meta apply) = TypeAst`) reflect a type-VALUE to the `Ast` of its DEFINITION
    // (the verbatim `(type Name …)` decl form) — `ast` the INSTANTIATED decl (concrete args substituted),
    // `ast-generic` the GENERIC decl (type params intact). Both `Type -> Ast`, pure, folded at lower time.
    // Same builder-record shape as `of`/`eq` (`ctor_record`), reached by member access `(. Type ast)`.
    let ast_field = push_atom(ast, Leaf::Name("ast".into()));
    let ast_op = ctor_record(ast, "type-ast");
    let ast_gen = {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, ast_field, ast_op])
    };
    let ast_generic_field = push_atom(ast, Leaf::Name("ast-generic".into()));
    let ast_generic_op = ctor_record(ast, "type-ast-generic");
    let ast_generic = {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, ast_generic_field, ast_generic_op])
    };
    push_list(ast, vec![head, of, eq, ast_gen, ast_generic])
}

/// The type `(fn () (-> Char Int64))` for `Char.to-int` — the total scalar-value read. A ZERO-PARAM `fn`
/// wrapper (monomorphic, but needed so `scheme_of` reads a SCHEME not a bare type-value — see
/// [`string_to_int64_type`]). The `Char` param is `(intrinsic "Char")` (→ `Ty::Char`).
fn char_to_int_type(ast: &mut Arenas) -> StructId {
    let char_ty = intrinsic_node(ast, "Char");
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let body = arrow_type(ast, char_ty, int64); // (-> Char Int64)
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> Int64 (Option Char)))` for `Char.from-int` — the fallible integer→char
/// conversion. A ZERO-PARAM `fn` wrapper. The result `(Option Char)` — `Option` an ordinary prelude
/// name applied to the `(intrinsic "Char")` type node, reducing to `Ty::Sum{Option, [Char]}`.
fn char_from_int_type(ast: &mut Arenas) -> StructId {
    let option_char = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let char_ty = intrinsic_node(ast, "Char");
        push_list(ast, vec![option, char_ty])
    };
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let body = arrow_type(ast, int64, option_char); // (-> Int64 (Option Char))
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> String Bytes))` for `String.to-bytes` — the UTF-8 encoding. A zero-param `fn`
/// wrapper (monomorphic; the wrapper makes `scheme_of` read a SCHEME not a bare type-value, as the other
/// String ops do). The `String` param is `(intrinsic "String")` (→ `Ty::String`), the result `(intrinsic
/// bytes-ty)` (→ `Ty::Bytes`) — both the ground type-values directly, no scope lookup.
fn string_to_bytes_type(ast: &mut Arenas) -> StructId {
    let string = intrinsic_node(ast, "String");
    let bytes = intrinsic_node(ast, "bytes-ty");
    let body = arrow_type(ast, string, bytes); // (-> String Bytes)
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> Bytes (Option String)))` for `String.from-bytes` — the fallible UTF-8 DECODE.
/// A zero-param `fn` wrapper (see `string_to_bytes_type`). The `Bytes` param is `(intrinsic bytes-ty)`
/// (→ `Ty::Bytes`); the result `(Option String)` — `Option` an ordinary prelude name applied to the
/// `(intrinsic "String")` type node, reducing to `Ty::Sum{Option, [String]}` when the scheme is read.
fn string_from_bytes_type(ast: &mut Arenas) -> StructId {
    let option_string = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let string = intrinsic_node(ast, "String");
        push_list(ast, vec![option, string])
    };
    let bytes = intrinsic_node(ast, "bytes-ty");
    let body = arrow_type(ast, bytes, option_string); // (-> Bytes (Option String))
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(-> Bytes (-> Int64 (Option Int64)))` for `Bytes.at` — the FALLIBLE indexed read: take a
/// byte sequence and an `Int64` index, return `(Option Int64)` (`Some` of the byte in range, `None`
/// out of range). Monomorphic (a byte is always an `Int64`), unlike `List.at`'s element-generic scheme.
/// The `Bytes` parameter is `(intrinsic bytes-ty)` directly (a bare name would mis-resolve inside the
/// module being built); `Option`/`Int64` are ordinary prelude names resolved when the scheme reduces.
fn bytes_at_type(ast: &mut Arenas) -> StructId {
    let option_int64 = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let int64 = push_atom(ast, Leaf::Name("Int64".into()));
        push_list(ast, vec![option, int64])
    };
    let int64_idx = push_atom(ast, Leaf::Name("Int64".into()));
    let index_arrow = arrow_type(ast, int64_idx, option_int64); // (-> Int64 (Option Int64))
    let bytes = intrinsic_node(ast, "bytes-ty");
    arrow_type(ast, bytes, index_arrow) // (-> Bytes (-> Int64 (Option Int64)))
}

/// The type `(-> Bytes (-> Bytes Bytes))` for `Bytes.concat` — append two byte sequences. Both `Bytes`
/// positions are `(intrinsic bytes-ty)` (a bare name would mis-resolve inside the module). The byte
/// companion of `List.concat`.
fn bytes_concat_type(ast: &mut Arenas) -> StructId {
    let b_out = intrinsic_node(ast, "bytes-ty");
    let b_rhs = intrinsic_node(ast, "bytes-ty");
    let inner = arrow_type(ast, b_rhs, b_out); // (-> Bytes Bytes)
    let b_lhs = intrinsic_node(ast, "bytes-ty");
    arrow_type(ast, b_lhs, inner) // (-> Bytes (-> Bytes Bytes))
}

/// The type `(-> Bytes (-> Int64 (-> Int64 (Option Bytes))))` for `Bytes.slice` — the FALLIBLE
/// sub-range read: take a byte sequence, a `start` and a `len` (both `Int64`), return `(Option Bytes)`
/// (`Some` of the slice when `start`/`len` are in range and non-negative, else `None`). Monomorphic; the
/// bytes companion of the fallible `at`, returning `Option Bytes` rather than `Option Int64`.
fn bytes_slice_type(ast: &mut Arenas) -> StructId {
    let option_bytes = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let bytes = intrinsic_node(ast, "bytes-ty");
        push_list(ast, vec![option, bytes])
    };
    let len_i = push_atom(ast, Leaf::Name("Int64".into()));
    let len_arrow = arrow_type(ast, len_i, option_bytes); // (-> Int64 (Option Bytes))
    let start_i = push_atom(ast, Leaf::Name("Int64".into()));
    let start_arrow = arrow_type(ast, start_i, len_arrow); // (-> Int64 (-> Int64 (Option Bytes)))
    let bytes = intrinsic_node(ast, "bytes-ty");
    arrow_type(ast, bytes, start_arrow) // (-> Bytes (-> Int64 (-> Int64 (Option Bytes))))
}

/// The type `(-> Bytes Bytes)` for `Bytes.compact` — return a content-equal byte sequence with
/// independent (rope-collapsed) storage. Monomorphic; a total (never-fallible) unary op.
fn bytes_compact_type(ast: &mut Arenas) -> StructId {
    let b_out = intrinsic_node(ast, "bytes-ty");
    let b_in = intrinsic_node(ast, "bytes-ty");
    arrow_type(ast, b_in, b_out) // (-> Bytes Bytes)
}

/// The type `(-> (List UInt8) Bytes)` for `Bytes.of` — a monomorphic arrow taking a list of `UInt8`
/// and returning `Bytes`, TOTAL (never traps, no Option): a `UInt8` element is in `0..=255` by its TYPE,
/// so a byte sequence is well-formed by construction (`collections-and-text.md` §A Byte Is A UInt8). To
/// build a byte from a wider integer, TRUNCATE with `(UInt8.wrap n)` (total) at the call site — the LEB128
/// encoder's `(UInt8.wrap (| (& n 127) 128))` — rather than validating inside `Bytes.of`. So an
/// out-of-range LITERAL `(Bytes.of (list 256))` is a compile-time WIDTH reject (256 is not a UInt8), not a
/// runtime trap. The element type is `(UInt N)` where `N=8` — built via the same `(UInt 8)` type
/// constructor a `UInt8` annotation reduces to; `Bytes`/result is `(intrinsic bytes-ty)` directly (a bare
/// name would mis-resolve inside the module being built). Reduced to `(List UInt8) → Bytes` by `infer`.
fn bytes_of_type(ast: &mut Arenas) -> StructId {
    let list_u8 = {
        let list = push_atom(ast, Leaf::Name("List".into()));
        // `(UInt 8)` — the UInt8 type, applied via the `UInt` type constructor (the same reduction a
        // `UInt8` annotation takes). A `List` element of this type makes each byte a UInt8.
        let uint = push_atom(ast, Leaf::Name("UInt".into()));
        let eight = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(8),
                radix: Radix::Dec,
            },
        );
        let u8_ty = push_list(ast, vec![uint, eight]);
        push_list(ast, vec![list, u8_ty])
    };
    let bytes = intrinsic_node(ast, "bytes-ty");
    arrow_type(ast, list_u8, bytes)
}

/// The type `(-> Bytes Int64)` for `Bytes.len` — a monomorphic arrow taking a `Bytes` and returning its
/// length as an `Int64`. The `Bytes` parameter is the `(intrinsic bytes-ty)` type-value directly (see
/// [`bytes_of_type`] — a bare `Bytes` name would mis-resolve inside the module being built).
fn bytes_len_type(ast: &mut Arenas) -> StructId {
    let bytes = intrinsic_node(ast, "bytes-ty");
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    arrow_type(ast, bytes, int64)
}

/// The type `(fn () (-> String Int64))` for a string length query — a ZERO-PARAMETER type-lambda
/// wrapping the monomorphic arrow. The `fn` wrapper is REQUIRED even with no quantified variables: it
/// makes `scheme_of` read the op record as a polymorphic SCHEME (`type_in_env` on the body), NOT as a
/// bare type-VALUE — a plain `(-> String Int64)` `(meta t)` would make `typeval_of` reduce the whole op
/// record to a `Ty::Type`, so projecting `(. String scalar-len)` would yield a type-value (unapplyable)
/// rather than the length operation. The param is `(intrinsic "String")` (→ `Ty::String`), not the NAME
/// `String` (which is the module record, a value).
fn string_to_int64_type(ast: &mut Arenas) -> StructId {
    let string = intrinsic_node(ast, "String");
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let body = arrow_type(ast, string, int64);
    // `(fn () body)` — an empty parameter list (no quantified type variables), the monomorphic wrapper.
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> String (-> Int64 (Option String))))` for `String.at` — the fallible scalar read
/// `String → Int64 → (Option String)`. A ZERO-PARAM `fn` wrapper (monomorphic, but the wrapper is
/// needed so `scheme_of` reads a SCHEME not a bare type-value — see [`string_to_int64_type`]). The
/// `String` param + `Option`'s `String` arg are the `(intrinsic "String")` type node (→ `Ty::String`),
/// not the NAME `String` (the module record); `(Option String)` reduces via the built-in Option ctor.
fn str_at_type(ast: &mut Arenas) -> StructId {
    let option_string = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let string = intrinsic_node(ast, "String");
        push_list(ast, vec![option, string])
    };
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let index_arrow = arrow_type(ast, int64, option_string); // (-> Int64 (Option String))
    let string = intrinsic_node(ast, "String");
    let body = arrow_type(ast, string, index_arrow); // (-> String (-> Int64 (Option String)))
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> String (-> Int64 (Option Char))))` for `String.scalar-at` — the fallible read of
/// the CHAR at a scalar position (`String → Int64 → (Option Char)`, the char-typed companion of
/// `String.at`). A ZERO-PARAM `fn` wrapper (see [`str_at_type`]). The `String` param is `(intrinsic
/// "String")` (→ `Ty::String`); the result `(Option Char)` — `Option` an ordinary prelude name applied to
/// the `(intrinsic "Char")` type node (→ `Ty::Char`), reducing to `Ty::Sum{Option, [Char]}`.
fn str_scalar_at_type(ast: &mut Arenas) -> StructId {
    let option_char = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let char_ty = intrinsic_node(ast, "Char");
        push_list(ast, vec![option, char_ty])
    };
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let index_arrow = arrow_type(ast, int64, option_char); // (-> Int64 (Option Char))
    let string = intrinsic_node(ast, "String");
    let body = arrow_type(ast, string, index_arrow); // (-> String (-> Int64 (Option Char)))
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> String (-> String String)))` for `String.concat` — the total binary join. A
/// ZERO-PARAM `fn` wrapper (monomorphic, but the wrapper is needed so `scheme_of` reads a SCHEME not a
/// bare type-value — see [`string_to_int64_type`]). Both operands and the result are the `(intrinsic
/// "String")` type node (→ `Ty::String`), not the NAME `String` (the module record).
fn string_concat_type(ast: &mut Arenas) -> StructId {
    let out = intrinsic_node(ast, "String");
    let rhs = intrinsic_node(ast, "String");
    let inner = arrow_type(ast, rhs, out); // (-> String String)
    let lhs = intrinsic_node(ast, "String");
    let body = arrow_type(ast, lhs, inner); // (-> String (-> String String))
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> String (-> Int64 (-> Int64 (Option String)))))` for `String.slice` — the
/// FALLIBLE sub-range read by SCALAR offsets: take a string, a `start` and an `end` (both `Int64`,
/// half-open `[start, end)`), return `(Option String)` (`Some` of the substring when `0 <= start <= end
/// <= scalar-len`, else `None`). A ZERO-PARAM `fn` wrapper (see [`string_to_int64_type`] for why). The
/// `String` param + `Option`'s `String` arg are `(intrinsic "String")` (→ `Ty::String`); the string
/// companion of `Bytes.slice`, returning `Option String` rather than `Option Bytes` (and cutting by
/// SCALAR offset, not byte).
fn string_slice_type(ast: &mut Arenas) -> StructId {
    let option_string = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let string = intrinsic_node(ast, "String");
        push_list(ast, vec![option, string])
    };
    let end_i = push_atom(ast, Leaf::Name("Int64".into()));
    let end_arrow = arrow_type(ast, end_i, option_string); // (-> Int64 (Option String))
    let start_i = push_atom(ast, Leaf::Name("Int64".into()));
    let start_arrow = arrow_type(ast, start_i, end_arrow); // (-> Int64 (-> Int64 (Option String)))
    let string = intrinsic_node(ast, "String");
    let body = arrow_type(ast, string, start_arrow); // (-> String (-> Int64 (-> Int64 (Option String))))
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// An operation record for a `List` module field: `(record ((meta t) TYPE-LAMBDA) ((meta apply)
/// (intrinsic PRIM)))` — the same shape as `operator_record`, but the type-lambda is supplied (a list
/// operation's signature varies per op, unlike the shared arithmetic/comparison shapes).
fn list_op_record(ast: &mut Arenas, prim: &str, type_lambda: StructId) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let t_field = meta_field(ast, "t", type_lambda);
    let apply = intrinsic_node(ast, prim);
    let apply_field = meta_field(ast, "apply", apply);
    push_list(ast, vec![head, t_field, apply_field])
}

/// The type-lambda `(fn (a) (-> (List a) Int64))` for `List.len` — generic over the element type `a`,
/// taking a list of it and returning `Int64`. Written as ordinary AST so `infer` reduces it to the
/// scheme `∀a. (List a) → Int64` through the one evaluator (`(List a)` reduces via `Prim::ListCtor`).
fn list_len_type_lambda(ast: &mut Arenas) -> StructId {
    let list_a = list_a_type(ast);
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let body = arrow_type(ast, list_a, int64);
    list_type_lambda(ast, body)
}

/// The type-lambda `(fn (a) (-> (List a) (-> a (List a))))` for `List.push` — `∀a. (List a) → a →
/// (List a)`: take a list and an element of its type, return the new list.
fn list_push_type_lambda(ast: &mut Arenas) -> StructId {
    let list_r = list_a_type(ast);
    let elem = push_atom(ast, Leaf::Name("a".into()));
    let inner = arrow_type(ast, elem, list_r); // (-> a (List a))
    let list_l = list_a_type(ast);
    let body = arrow_type(ast, list_l, inner); // (-> (List a) (-> a (List a)))
    list_type_lambda(ast, body)
}

/// The type-lambda `(fn (a) (-> (List a) (-> (List a) (List a))))` for `List.concat` — `∀a. (List a) →
/// (List a) → (List a)`: concatenate two lists of the same element type.
///
/// The single quantified `a` in BOTH operand positions is what makes concatenation defined only when the
/// two operands share one element type — HM unification rejects `(List Int64) ++ (List Bool)` — and the
/// result `(List a)` is a list of that same type. The empty-list identity (`[] ++ xs = xs = xs ++ []`) is
/// the runtime `vec-concat` rope's own law (concatenating an empty rope returns the other operand).
//= spec/capabilities/collections-and-text.md#a-list-is-grown-by-functional-construction
//# Concatenation MUST be defined only when both operands share one element type — the result is a list of that type — consistent with *A List Is An Ordered Homogeneous Sequence*; concatenating with the empty list on either side MUST yield a list equal to the other operand.
fn list_concat_type_lambda(ast: &mut Arenas) -> StructId {
    let list_r = list_a_type(ast);
    let list_2 = list_a_type(ast);
    let inner = arrow_type(ast, list_2, list_r); // (-> (List a) (List a))
    let list_1 = list_a_type(ast);
    let body = arrow_type(ast, list_1, inner); // (-> (List a) (-> (List a) (List a)))
    list_type_lambda(ast, body)
}

/// The type-lambda `(fn (a) (-> (List a) (-> Int64 (-> a (List a)))))` for `List.update` — `∀a. (List a)
/// → Int64 → a → (List a)`: take a list, an Int64 index, and a replacement element of the list's type,
/// return the new list. The functional-construction companion of `List.push`.
fn list_update_type_lambda(ast: &mut Arenas) -> StructId {
    let list_r = list_a_type(ast);
    let elem = push_atom(ast, Leaf::Name("a".into()));
    let elem_arrow = arrow_type(ast, elem, list_r); // (-> a (List a))
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let index_arrow = arrow_type(ast, int64, elem_arrow); // (-> Int64 (-> a (List a)))
    let list_l = list_a_type(ast);
    let body = arrow_type(ast, list_l, index_arrow); // (-> (List a) (-> Int64 (-> a (List a))))
    list_type_lambda(ast, body)
}

/// The type-lambda `(fn (a) (-> (List a) (-> Int64 (Option a))))` for `List.at` — `∀a. (List a) → Int64
/// → (Option a)`: take a list and an Int64 index, return the element wrapped in `Option` (`Some` in
/// bounds, `None` out — collections-and-text.md #Indexing And Lookup Are Fallible). `(Option a)` reduces
/// via the built-in `Option` sum ctor exactly as `(List a)` reduces via `List`, so the fallible-access
/// result type is expressed in the ordinary generic-application evaluator, no privileged `Option` path.
fn list_at_type_lambda(ast: &mut Arenas) -> StructId {
    let option_a = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let a = push_atom(ast, Leaf::Name("a".into()));
        push_list(ast, vec![option, a])
    };
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let index_arrow = arrow_type(ast, int64, option_a); // (-> Int64 (Option a))
    let list_l = list_a_type(ast);
    let body = arrow_type(ast, list_l, index_arrow); // (-> (List a) (-> Int64 (Option a)))
    list_type_lambda(ast, body)
}

/// The type-lambda `(fn (a) (-> String a))` for `trap` — `∀a. String → a`. The RESULT is the quantified
/// parameter `a` (a BARE parameter used as a type, like the comparison operators' operand), so the scheme
/// reads as `String → <fresh var>`: `scheme_of` binds `a` to a fresh unification variable and the result
/// IS that variable, giving `(trap "x")` a fresh type at each use that unifies with any expected type —
/// the surface form of "a diverging expression has type Never, which unifies with any type" without a
/// dedicated `Ty::Never`. Reuses `list_type_lambda` (a one-parameter `(fn (a) …)`), the same wrapper the
/// element-generic list ops use.
fn trap_type_lambda(ast: &mut Arenas) -> StructId {
    let string_ty = push_atom(ast, Leaf::Name("String".into()));
    let a = push_atom(ast, Leaf::Name("a".into()));
    let body = arrow_type(ast, string_ty, a); // (-> String a)
    list_type_lambda(ast, body)
}

/// The `(module <op-record>)` field for the built-in `Ast` record — the `Ast.module` self-reflection member
/// (self-hosting-surface.md §A Program's Syntax Tree Is An Ordinary Value). A bare-value magic-constant,
/// NAMESPACED on the `Ast` record (operator directive: no bare globals). `Ast.module`
/// resolves through ordinary member access on the built-in `Ast` and types as the built-in `Ast` sum; a
/// user `type Ast` has no `module` field, so it shadows the reflection (the field lives ONLY on the built-in
/// Ast record — see `sums`/`Db::load_linked`, which augments only the built-in Ast decl). `(meta t)` is
/// `(fn () Ast)` (a bare Ast value); `(meta apply) = (intrinsic reflect-module)` (`Prim::ReflectModule`),
/// filled at lowering from the ENCLOSING MODULE's canonical source (never applied). Named `module` (operator:
/// clearer about the capture's SCOPE — the enclosing module — leaving room for future item/caller captures).
/// Replaces the retired blind `(. Ast self)` syntax-rewrite with a resolved, shadow-respecting, type-directed form.
pub(crate) fn ast_module_field(ast: &mut Arenas) -> StructId {
    // Type `(fn () Ast)` — a bare value of the built-in Ast sum (the zero-param `fn` wrapper makes
    // `scheme_of` read a monomorphic SCHEME rather than collapsing `Ast` to a type-value).
    let module_type = {
        let ast_ty = push_atom(ast, Leaf::Name("Ast".into()));
        let fn_head = push_atom(ast, Leaf::Name("fn".into()));
        let params = push_list(ast, vec![]);
        push_list(ast, vec![fn_head, params, ast_ty]) // (fn () Ast)
    };
    let op = list_op_record(ast, "reflect-module", module_type);
    let module_name = push_atom(ast, Leaf::Name("module".into()));
    {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, module_name, op])
    } // (module <op-record>)
}

/// The `(print <op-record>)` field for the built-in `Ast` record — `Ast.print v : String`, the compiler-
/// exposed PRINTER that renders an AST value as its canonical re-readable text (`self-hosting-surface.md`
/// §Text Is A Projection Reached By A Reader And A Printer). The NAMESPACED home of the former top-level
/// `print` (operator directive: prelude records with associated functions, no bare globals). The field
/// value is the EXACT op-record `print` was — a monomorphic `list_op_record` with `(meta t) = (fn () (-> Ast
/// String))` and `(meta apply) = (intrinsic print)` (`Prim::Print`), folded on a compile-time-visible operand
/// in `lower` — so `Ast.print` reduces identically to the old `print`, no new prim. Carried on the `Ast`
/// `TypeDecl.associated` (set in `sums::prelude_decls`, appended by `sum_record`), the SAME pattern as
/// `Ast.module`; a user `type Ast` carries no associated, so it shadows it.
pub(crate) fn ast_print_field(ast: &mut Arenas) -> StructId {
    //= spec/capabilities/self-hosting-surface.md#a-printer-renders-the-canonical-representation-as-re-readable-text
    //# Reading the text a printer produced for a value MUST yield a value equal to the original under structural equality, so that the reader and printer round-trip.
    let print_lambda = mono_op_type_lambda(ast, "Ast", "String");
    let op = list_op_record(ast, "print", print_lambda);
    let print_name = push_atom(ast, Leaf::Name("print".into()));
    {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, print_name, op])
    } // (print <op-record>)
}

/// The `(read <op-record>)` field for the built-in `Ast` record — `Ast.read s : Ast`, the compiler-exposed
/// READER that parses canonical text back into an AST value (`self-hosting-surface.md` §Text Is A Projection
/// Reached By A Reader And A Printer). The NAMESPACED home of the former top-level `read`; the EXACT op-record
/// `read` was — monomorphic `(meta t) = (fn () (-> String Ast))`, `(meta apply) = (intrinsic read)`
/// (`Prim::Read`) — so `Ast.read` reduces identically, no new prim. Carried on the `Ast` `TypeDecl.associated`
/// (the SAME pattern as `Ast.module`); a user `type Ast` shadows it. `Ast.read (Ast.print v) == v` round-trips.
pub(crate) fn ast_read_field(ast: &mut Arenas) -> StructId {
    let read_lambda = mono_op_type_lambda(ast, "String", "Ast");
    let op = list_op_record(ast, "read", read_lambda);
    let read_name = push_atom(ast, Leaf::Name("read".into()));
    {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, read_name, op])
    } // (read <op-record>)
}

/// The built-in `Ast` record's ASSOCIATED FUNCTIONS — the prelude-defined non-ctor member fields
/// (`(name <op-record>)` nodes) that `sums::sum_record` appends to the synthesized `Ast` record, so they
/// are reached as `(. Ast member)`. Defined HERE in the prelude (attached to the Ast `TypeDecl` in
/// `sums::prelude_decls`), like `bigint_module`'s fields live in the prelude — NOT a `Db::load`
/// post-synthesis special-case. `Ast.module` (self-reflection), `Ast.print` (printer), `Ast.read` (reader) —
/// all the former top-level self-hosting names, NAMESPACED onto the `Ast` record (a user `type Ast` carries
/// none, so it shadows them).
pub(crate) fn ast_associated_fields(ast: &mut Arenas) -> Vec<StructId> {
    vec![
        ast_module_field(ast),
        ast_print_field(ast),
        ast_read_field(ast),
        ast_gensym_field(ast),
    ]
}

/// The `(gensym <op-record>)` field for the built-in `Ast` record — `Ast.gensym base : Ast`, the
/// FRESH-NAME mint for MANUAL macro hygiene (macros are non-hygienic by default;
/// `DESIGN-macro-system.md`). Takes a base-name `String` and folds to a fresh `Ast.Name` unique to the
/// call site + deterministic across compiles (`Prim::Gensym`, `lower_gensym`) — the SAME monomorphic
/// op-record shape as `Ast.read` (`(meta t) = (fn () (-> String Ast))`, `(meta apply) = (intrinsic
/// gensym)`). Carried on the `Ast` `TypeDecl.associated` like `Ast.module`/`print`/`read`; a user `type
/// Ast` carries no associated, so it shadows it.
pub(crate) fn ast_gensym_field(ast: &mut Arenas) -> StructId {
    let gensym_lambda = mono_op_type_lambda(ast, "String", "Ast");
    let op = list_op_record(ast, "gensym", gensym_lambda);
    let gensym_name = push_atom(ast, Leaf::Name("gensym".into()));
    {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, gensym_name, op])
    } // (gensym <op-record>)
}

/// The `(of <op-record>)` field for the built-in `Ordering` record — `Ordering.of a b : Ordering`, the
/// three-way comparison (`core-semantics.md` §A Total Order Is Observed Through A Three-Way Comparison).
/// The NAMESPACED home of the former top-level `compare` (operator directive: prelude records with
/// associated functions, no bare globals). The field value is the EXACT op-record `compare` was — an
/// `operator_record` with `(meta t) = compare_type_lambda` (`∀a. a → a → Ordering`) and `(meta apply) =
/// (intrinsic compare)` (`Prim::Compare`) — so `Ordering.of` reduces identically to the old `compare`, no
/// new prim. Carried on the `Ordering` `TypeDecl.associated` (set in `sums::prelude_decls`, appended by
/// `sum_record`), the SAME pattern as `Ast.module` — so it lives in the prelude, and a user `type Ordering`
/// (a separate decl carrying no associated) shadows it.
pub(crate) fn ordering_of_field(ast: &mut Arenas) -> StructId {
    let op = operator_record(ast, "compare", OpShape::Compare);
    let of_name = push_atom(ast, Leaf::Name("of".into()));
    {
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, of_name, op])
    } // (of <op-record>)
}

/// Build the MONOMORPHIC operator scheme `(fn () (-> FROM TO))` — a zero-parameter type-lambda over the
/// concrete named types `FROM`/`TO`. The `(fn () …)` wrapper is REQUIRED even with no quantified
/// variables so `scheme_of` reads a monomorphic SCHEME rather than collapsing the bare arrow to
/// `Ty::Type` (the same reason the fixed-width `checked_field`/float `of-int` schemes wrap a `(fn () …)`).
/// Used for `print : Ast → String` and `read : String → Ast`.
fn mono_op_type_lambda(ast: &mut Arenas, from: &str, to: &str) -> StructId {
    let from_ty = push_atom(ast, Leaf::Name(from.into()));
    let to_ty = push_atom(ast, Leaf::Name(to.into()));
    let body = arrow_type(ast, from_ty, to_ty); // (-> FROM TO)
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]); // zero parameters — monomorphic
    push_list(ast, vec![fn_head, params, body])
}

/// Build `(List a)` — the list type applied to the element parameter `a`, a shared shape in the `List`
/// operation type-lambdas (each occurrence is a fresh `(List a)` referencing the same parameter name).
fn list_a_type(ast: &mut Arenas) -> StructId {
    let list = push_atom(ast, Leaf::Name("List".into()));
    let a = push_atom(ast, Leaf::Name("a".into()));
    push_list(ast, vec![list, a])
}

/// Build `(-> l r)` — a function type from `l` to `r`.
fn arrow_type(ast: &mut Arenas, l: StructId, r: StructId) -> StructId {
    let arrow = push_atom(ast, Leaf::Name("->".into()));
    push_list(ast, vec![arrow, l, r])
}

/// Wrap `body` in `(fn (a) body)` — the one-parameter type-lambda over the element type `a`, shared by
/// the `List` operation schemes.
fn list_type_lambda(ast: &mut Arenas, body: StructId) -> StructId {
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type shape of a binary operator — which `(meta t)` type-lambda it carries. Both are ordinary
/// AST written entirely in terms of the grammar (`fn`/`->`/`Int`), reduced to a `Scheme` by the one
/// evaluator; the shape only selects which lambda body is built.
#[derive(Clone, Copy)]
enum OpShape {
    /// `∀a. (Int a) → (Int a) → (Int a)` — the width-generic integer binary operators.
    IntBinary,
    /// `∀a. a → a → Bool` — the relational comparisons (bare operand var, `Bool` result).
    Comparison,
    /// `∀a. a → a → Ordering` — the three-way `compare` (bare operand var, `Ordering` sum result).
    Compare,
}

/// An operator record `(record ((meta t) TYPE-LAMBDA) ((meta apply) (intrinsic PRIM)))`. `(meta t)`
/// is the operator's type — a compile-time type-lambda read generically by `infer`; `(meta apply)` is
/// the reduction, read by `lower`. `shape` selects the type-lambda (integer-binary vs comparison).
/// A BUILT-IN OPERATION VALUE — the record a built-in module's operation field holds. It carries a
/// `(meta t)` scheme (the operation's type) and a `(meta apply)` intrinsic (its implementation). It is an
/// ordinary VALUE: projecting the field (member access on the module) yields THIS record without applying
/// it — the first-class treatment `A Function Is A First-Class Value` gives any value. APPLYING it
/// `(Mod.op args)` rides the ordinary `(meta apply)` dispatch and produces the operation's defined
/// result; applying at an argument count the operation's arrow does not accept is an ordinary type error
/// (the application unifies against the `(meta t)` arrow), never an unspecified result.
//= spec/capabilities/core-semantics.md#a-built-in-module-is-a-record-of-its-operations
//# A field of a built-in module whose implementation the language provides MUST hold a **built-in operation value** — a first-class value denoting that operation. A built-in operation value MUST be a value in the sense of *A Function Is A First-Class Value*: projecting it MUST yield the value itself, so that member access on a built-in module evaluates to the operation it names without applying it.
//= spec/capabilities/core-semantics.md#a-built-in-module-is-a-record-of-its-operations
//# Applying a built-in operation value to arguments MUST produce the same result the operation defines for those arguments, so that `(Mod.op args)` — the application of the projected field — is equivalent to invoking the built-in operation directly. Applying it at an argument count the operation does not accept MUST be a compile-time error under *Applying A Function Binds Its Parameter To Its Argument* and the arity rules, exactly as for any other function value, rather than an unspecified result.
//= spec/capabilities/core-semantics.md#a-built-in-module-is-a-record-of-its-operations
//# A built-in operation value used other than by application — bound to a name, stored in a data structure, compared, or partially applied — has no outcome fixed by this document beyond that it MUST NOT produce a wrong result: a compiler that does not realize such a use MUST decline to compile the program rather than emit code that computes an incorrect value. This preserves *reject-don't-miscompile* while leaving the first-class treatment of a built-in operation value (storage, partial application) to be specified as it is realized.
fn operator_record(ast: &mut Arenas, op: &str, shape: OpShape) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let lambda = match shape {
        OpShape::IntBinary => binop_type_lambda(ast),
        OpShape::Comparison => comparison_type_lambda(ast),
        OpShape::Compare => compare_type_lambda(ast),
    };
    let t_field = meta_field(ast, "t", lambda);
    let prim = intrinsic_node(ast, op);
    let apply_field = meta_field(ast, "apply", prim);
    push_list(ast, vec![head, t_field, apply_field])
}

/// The type-lambda `(fn (a) (-> (Int a) (-> (Int a) (Int a))))` shared by the binary arithmetic
/// operators — generic over the integer type: a lambda over the width `a`, whose body is the curried
/// function type built from the `Int` constructor applied to that same `a` in each position. Written
/// entirely as ordinary AST (lambda + applications) so `infer` reduces it through the one evaluator to
/// a `Scheme`, with `a` an ordinary lambda parameter.
fn binop_type_lambda(ast: &mut Arenas) -> StructId {
    // `(Int a)` — reused shape; each occurrence references the same parameter name `a`.
    let int_a = |ast: &mut Arenas| -> StructId {
        let int = push_atom(ast, Leaf::Name("Int".into()));
        let a = push_atom(ast, Leaf::Name("a".into()));
        push_list(ast, vec![int, a])
    };
    // `(-> (Int a) (-> (Int a) (Int a)))` — curried binary.
    let arrow = |ast: &mut Arenas, l: StructId, r: StructId| -> StructId {
        let arr = push_atom(ast, Leaf::Name("->".into()));
        push_list(ast, vec![arr, l, r])
    };
    let ia1 = int_a(ast);
    let ia2 = int_a(ast);
    let ia3 = int_a(ast);
    let inner = arrow(ast, ia2, ia3);
    let body = arrow(ast, ia1, inner);
    // `(fn (a) BODY)`.
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type-lambda `(fn (a) (-> a (-> a Bool)))` shared by the relational comparisons — generic over
/// the operand type `a` (a BARE parameter, so it unifies with `Bool` or an integer or, structurally,
/// any value), with a `Bool` result. `Bool` here is the ground-type prelude name the evaluator reduces
/// to `Ty::Bool`. Written as ordinary AST so `infer` reduces it to a `Scheme` `∀a. a → a → Bool`, with
/// `a` an ordinary lambda parameter — the same generic mechanism as the arithmetic lambda, differing
/// only in that the operand is the bare variable rather than `(Int a)`.
fn comparison_type_lambda(ast: &mut Arenas) -> StructId {
    // A bare reference to the parameter `a`.
    let a_ref = |ast: &mut Arenas| -> StructId { push_atom(ast, Leaf::Name("a".into())) };
    let arrow = |ast: &mut Arenas, l: StructId, r: StructId| -> StructId {
        let arr = push_atom(ast, Leaf::Name("->".into()));
        push_list(ast, vec![arr, l, r])
    };
    let a1 = a_ref(ast);
    let a2 = a_ref(ast);
    let bool_res = push_atom(ast, Leaf::Name("Bool".into()));
    let inner = arrow(ast, a2, bool_res); // (-> a Bool)
    let body = arrow(ast, a1, inner); // (-> a (-> a Bool))
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type-lambda `(fn (a) (-> a (-> a Ordering)))` for the three-way `compare` — the comparison shape
/// but yielding the `Ordering` sum instead of `Bool`. The result `Ordering` is a bare NAME resolving to
/// the built-in prelude sum's type-value (like `Bool`), so the scheme reduces to `∀a. a → a → Ordering`.
fn compare_type_lambda(ast: &mut Arenas) -> StructId {
    let a_ref = |ast: &mut Arenas| -> StructId { push_atom(ast, Leaf::Name("a".into())) };
    let arrow = |ast: &mut Arenas, l: StructId, r: StructId| -> StructId {
        let arr = push_atom(ast, Leaf::Name("->".into()));
        push_list(ast, vec![arr, l, r])
    };
    let a1 = a_ref(ast);
    let a2 = a_ref(ast);
    let ordering_res = push_atom(ast, Leaf::Name("Ordering".into()));
    let inner = arrow(ast, a2, ordering_res); // (-> a Ordering)
    let body = arrow(ast, a1, inner); // (-> a (-> a Ordering))
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// Append a fixed-width integer MODULE record for `(signed, width)` and return its occurrence — the
/// value a named width (`Int64`, `UInt8`, …) binds to. Built the same way the `(Int N)`/`(UInt N)`
/// constructor builds its module (see `eval::build_int_module`), so a named width and the constructor
/// application denote the same thing. It carries a `(meta t)` — its TYPE-VALUE — so the name works IN
/// TYPE POSITION (`(: e UInt8)` reduces to `Ty::Int` via the ordinary `(meta t)` projection); its
/// `max`/`min` are that width's bounds (from the shared `eval::int_bounds`, arbitrary precision so
/// `UInt64.max = 2^64-1` is exact); its arithmetic/conversion ops are `unrealized` (decline when
/// projected). Nothing is special-cased per name — only the `(signed, width)` differs.
/// Append a fixed-width FLOAT MODULE record for `width` and return its occurrence — the value a named
/// float width (`Float32`/`Float64`) binds to. The float analogue of [`int_module_record`], built the
/// SAME way the `(Float N)` constructor builds its module (`eval::build_float_module`), so a named width
/// and the constructor application denote one thing. It carries `(meta t)` = the type expression
/// `(Float width)` (so `(: e Float64)` reduces to `Ty::Float` via the ordinary `(meta t)` projection).
/// `of-int` (integer→float, the float analogue of `T.of`) and the width conversions are `unrealized`
/// fields until F5 — declining cleanly when projected, the closed-module rule every prelude module follows.
fn float_module_record(ast: &mut Arenas, width: u32) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta t)` = `(Float width)`, reduced to the concrete float type-value by `typeval_of`.
    let ty_expr = {
        let ctor = push_atom(ast, Leaf::Name("Float".into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    // `of-int` — the TOTAL integer→float conversion `Int64 → (Float width)`, an operator record whose
    // `(meta t)` is the scheme and `(meta apply)` the `float-of-int` intrinsic. The target is THIS
    // module's own width (concrete), so the scheme is monomorphic — but wrapped in a ZERO-PARAM `(fn ()
    // …)` so `scheme_of` reads it as a SCHEME (not a bare type-value, which would make `typeval_of`
    // reduce the whole op record to a `Ty::Type`), the same wrapper `String.at`/`scalar-len` use.
    let of_int_ty = float_of_int_type(ast, width);
    let of_int = list_op_record(ast, "float-of-int", of_int_ty);
    // `of` — the TOTAL float-WIDTH conversion `∀a. (Float a) → (Float width)` (promote/demote/identity),
    // width-generic in its SOURCE, this module's width as TARGET. The `(meta apply)` is `float-of`.
    let of_ty = float_of_type(ast, width);
    let of_op = list_op_record(ast, "float-of", of_ty);
    // `nan` — the canonical NOT-A-NUMBER value of THIS float width, a CONSTANT field (like `Int64.max`),
    // reached by member access `(. Float64 nan)`. Its value is the `float-nan` intrinsic directly
    // (`Prim::FloatNan` → `Core::ConstFloatNan`); NOT a literal, since `Decimal` holds only finite values.
    // Every NaN shares one canonical byte form, so `(= Float64.nan Float64.nan)` is true (core-semantics.md
    // #Floating-Point Equality Follows The Canonical Byte Form). The intrinsic is width-agnostic on its own
    // (`Prim::FloatNan` types as a DEFERRED-width float), so — exactly like `Int64.max` is `(: <lit> (Int
    // 64))` — the field ANNOTATES it with THIS module's width `(: <nan> (Float width))`. Without the
    // annotation `Float64.nan` typed as an unfixed float and unified with EITHER width, so a cross-width
    // comparison `(= Float32.nan Float64.nan)` slipped past the CDZ0301 the identical FINITE comparison
    // gets. Annotated, `Float64.nan` is `Ty::Float(Fixed(64))` and does not unify with a `Float32`.
    let nan_ty_expr = {
        let ctor = push_atom(ast, Leaf::Name("Float".into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let nan_val = {
        let intrinsic = intrinsic_node(ast, "float-nan");
        let colon = push_atom(ast, Leaf::Name(":".into()));
        push_list(ast, vec![colon, intrinsic, nan_ty_expr])
    };
    // `Infinity` — the canonical POSITIVE-INFINITY value of THIS float width, a CONSTANT field reached by
    // member access `(. Float64 Infinity)`. Built exactly like `nan`: the width-agnostic `float-inf`
    // intrinsic (`Prim::FloatInf` → `Core::ConstFloatInf`) ANNOTATED with this module's width `(: <inf>
    // (Float width))`, so `Float64.Infinity` is `Ty::Float(Fixed(64))` and does not unify with a `Float32`
    // (the same cross-width guard the `nan` annotation provides). NOT a literal (`Decimal` holds only
    // finite values). Unlike NaN it is fully ordered, so `(< 1.0 Float64.Infinity)` folds true. Negative
    // infinity is `(- Float64.Infinity)`.
    let inf_ty_expr = {
        let ctor = push_atom(ast, Leaf::Name("Float".into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let inf_val = {
        let intrinsic = intrinsic_node(ast, "float-inf");
        let colon = push_atom(ast, Leaf::Name(":".into()));
        push_list(ast, vec![colon, intrinsic, inf_ty_expr])
    };
    // `neg : (Float width) → (Float width)` — unary negation, TOTAL (`-1.0 * e`, sign-correct for
    // ±0.0/±inf, never traps). The named first-class form of prefix `(- e)`, lowered via `lower_negate`.
    let neg_op = {
        let neg_ty = float_neg_type(ast, width);
        list_op_record(ast, "neg", neg_ty)
    };
    let fields = vec![
        meta_field(ast, "t", ty_expr),
        {
            let k = push_atom(ast, Leaf::Name("neg".into()));
            {
                let eq = push_atom(ast, Leaf::Name("=".into()));
                push_list(ast, vec![eq, k, neg_op])
            }
        },
        {
            let k = push_atom(ast, Leaf::Name("of-int".into()));
            {
                let eq = push_atom(ast, Leaf::Name("=".into()));
                push_list(ast, vec![eq, k, of_int])
            }
        },
        {
            let k = push_atom(ast, Leaf::Name("of".into()));
            {
                let eq = push_atom(ast, Leaf::Name("=".into()));
                push_list(ast, vec![eq, k, of_op])
            }
        },
        {
            let k = push_atom(ast, Leaf::Name("nan".into()));
            {
                let eq = push_atom(ast, Leaf::Name("=".into()));
                push_list(ast, vec![eq, k, nan_val])
            }
        },
        {
            let k = push_atom(ast, Leaf::Name("Infinity".into()));
            {
                let eq = push_atom(ast, Leaf::Name("=".into()));
                push_list(ast, vec![eq, k, inf_val])
            }
        },
    ];
    let mut children = vec![head];
    for f in fields {
        children.push(f);
    }
    push_list(ast, children)
}

/// The type-lambda `(fn (a) (-> (Float a) (Float width)))` for a float module's `of` — the float-WIDTH
/// conversion, GENERIC over the source float width `a`, this module's `width` as the target. A real
/// type-lambda (`(fn (a) …)`, one quantified variable), unlike `of-int`'s zero-param wrapper: the source
/// `(Float a)` reduces via the `Float` constructor to a fresh float-width variable, so `infer` reads the
/// scheme `∀a. (Float a) → (Float width)` — a Float32 OR a Float64 (or a deferred literal) unifies as the
/// source, the result is always this module's concrete width.
fn float_of_type(ast: &mut Arenas, width: u32) -> StructId {
    let float_a = {
        let ctor = push_atom(ast, Leaf::Name("Float".into()));
        let a = push_atom(ast, Leaf::Name("a".into()));
        push_list(ast, vec![ctor, a])
    };
    let float_target = {
        let ctor = push_atom(ast, Leaf::Name("Float".into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let body = arrow_type(ast, float_a, float_target);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> (Float width) (Float width)))` for a float module's `neg` — unary negation,
/// TOTAL (`-1.0 * e` is sign-correct for ±0.0/±inf and never traps). The zero-param `fn` wrapper makes
/// `scheme_of` read a monomorphic SCHEME (see [`float_of_int_type`]); `(meta apply)` = the `neg` intrinsic,
/// lowered through `lower_negate`.
fn float_neg_type(ast: &mut Arenas, width: u32) -> StructId {
    let float_target = |ast: &mut Arenas| {
        let ctor = push_atom(ast, Leaf::Name("Float".into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let a = float_target(ast);
    let b = float_target(ast);
    let body = arrow_type(ast, a, b);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type `(fn () (-> Int64 (Float width)))` for a float module's `of-int` — a ZERO-PARAMETER
/// type-lambda wrapping the monomorphic arrow `Int64 → (Float width)`. The `fn` wrapper is REQUIRED
/// (even with no quantified variables) so `scheme_of` reads it as a polymorphic SCHEME rather than a
/// bare type-VALUE — see [`string_to_int64_type`]. The result `(Float width)` reduces via the `Float`
/// constructor to this module's own concrete float type.
fn float_of_int_type(ast: &mut Arenas, width: u32) -> StructId {
    let int64 = push_atom(ast, Leaf::Name("Int64".into()));
    let float_target = {
        let ctor = push_atom(ast, Leaf::Name("Float".into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let body = arrow_type(ast, int64, float_target);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    push_list(ast, vec![fn_head, params, body])
}

fn int_module_record(ast: &mut Arenas, signed: bool, width: u32) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    // `(meta t)` = the type expression `(Int width)` / `(UInt width)`, reduced to the concrete
    // type-value by `typeval_of`. This is what makes the name usable as a TYPE.
    let ty_expr = {
        let ctor = push_atom(ast, Leaf::Name(if signed { "Int" } else { "UInt" }.into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let mut fields = vec![meta_field(ast, "t", ty_expr)];
    // `max`/`min` — that width's bounds, at arbitrary precision (shared with the constructor's builder),
    // each ANNOTATED with the module's own width so projecting the field carries its type.
    match crate::eval::int_bounds(signed, width) {
        Some((max, min)) => {
            fields.push(int_field(ast, "max", max, signed, width));
            fields.push(int_field(ast, "min", min, signed, width));
        }
        None => {
            fields.push(unrealized_field(ast, "max"));
            fields.push(unrealized_field(ast, "min"));
        }
    }
    // `wrap` — the TRUNCATING conversion INTO this width: `∀(w,s). Int^s_w → THIS`. An operator record
    // whose `(meta t)` is `(fn (a) (-> (Int a) TARGET))` — the source `(Int a)` fully polymorphic in
    // width AND sign (the paired sign-variable), the target `TARGET` this module's own concrete width —
    // and whose `(meta apply)` is the `wrap` intrinsic. ONE such field per module (no per-source-type
    // explosion): the target is fixed by the module, the source by unification at the call site.
    fields.push(wrap_field(ast, signed, width));
    // `checked-add`/`checked-sub`/`checked-mul` — the FALLIBLE arithmetic companions of the trapping
    // `+`/`-`/`*`: `T → T → (Option T)`, the exact result wrapped in `Some` when it fits, `None` on
    // overflow (numeric-model.md §Overflow Is Defined — the defined value outcome alongside the trap).
    // Real operator records (the `(meta apply)` = the checked intrinsic); a constant folds, a runtime
    // operand is a later increment. A NAMED overflow-fallible form is offered for EACH of addition,
    // subtraction, and multiplication (the full set the numeric model requires):
    //= spec/capabilities/numeric-model.md#an-overflow-fallible-operation-reports-overflow-rather-than-trapping
    //# An integer type MUST offer, alongside its trapping arithmetic, a named overflow-fallible form of each of addition, subtraction, and multiplication whose result is the exact value wrapped in the present case when it is in range and the absent case when the operation overflows, so that a program can branch on overflow without trapping.
    //
    // These fallible forms are opted into BY NAME (`checked-add`/`checked-sub`/`checked-mul`) — the bare
    // `+`/`-`/`*` keeps the trapping default, so overflow is never SILENTLY reported as absent:
    //= spec/capabilities/numeric-model.md#an-overflow-fallible-operation-reports-overflow-rather-than-trapping
    //# The overflow-fallible form MUST be opted into by name at the operation, so that an author who writes the ordinary operator still gets the trapping outcome and overflow is never silently reported.
    fields.push(checked_field(
        ast,
        "checked-add",
        "checked-add",
        signed,
        width,
    ));
    fields.push(checked_field(
        ast,
        "checked-sub",
        "checked-sub",
        signed,
        width,
    ));
    fields.push(checked_field(
        ast,
        "checked-mul",
        "checked-mul",
        signed,
        width,
    ));
    // `wrapping-add`/`wrapping-sub`/`wrapping-mul` — two's-complement wraparound modulo 2^width:
    // `T → T → T`, NEVER trapping (numeric-model.md §Overflow Is Defined — the modular value outcome, what
    // a hash / fixed-width round-trip wants). Real operator records (`(meta apply)` = the wrapping
    // intrinsic); a constant folds via `wrapping_*`, a runtime operand emits the RAW machine
    // `i64.add`/`i64.sub`/`i64.mul` (no overflow guard — wasm's ops already wrap). A NAMED wrapping form is
    // offered for EACH of addition, subtraction, and multiplication (the full set the numeric model requires):
    //= spec/capabilities/numeric-model.md#a-wrapping-operation-has-a-defined-modular-outcome
    //# An integer type MUST offer a named wrapping form of each of addition, subtraction, and multiplication whose result on overflow is the two's-complement value reduced modulo the type's range, so that modular arithmetic has a defined non-trapping outcome distinct from the trapping default.
    //
    // The wrapping form is opted into BY NAME (`wrapping-add`/`wrapping-sub`/`wrapping-mul`), so it never
    // displaces the trapping default an unqualified `+`/`-`/`*` selects:
    //= spec/capabilities/numeric-model.md#a-wrapping-operation-has-a-defined-modular-outcome
    //# The wrapping form MUST be opted into by name at the operation, so that it never displaces the trapping default an unqualified operator selects.
    fields.push(wrapping_field(
        ast,
        "wrapping-add",
        "wrapping-add",
        signed,
        width,
    ));
    fields.push(wrapping_field(
        ast,
        "wrapping-sub",
        "wrapping-sub",
        signed,
        width,
    ));
    fields.push(wrapping_field(
        ast,
        "wrapping-mul",
        "wrapping-mul",
        signed,
        width,
    ));
    // `of` — the CHECKED (trapping) conversion into this width: in range → the value at the target type,
    // out of range → a TRAP. The range-checked companion of `wrap`; NOT `Option`-returning (the
    // overflow-fallible forms are the `checked-add`/`checked-mul` fields above). Together with `wrap` (the
    // truncating conversion, keeping the target's low bits) these are the TWO explicit inter-width
    // conversions — a named `.of`/`.wrap` at the site, never an implicit widen/narrow:
    //= spec/capabilities/numeric-model.md#a-conversion-between-integer-types-is-explicit
    //# A conversion between two integer types MUST be written explicitly, as either a range-checked conversion that traps on a value outside the target type's range or a truncating conversion that keeps the target type's low bits, never an implicit widening or narrowing.
    fields.push(of_field(ast, signed, width));
    // `neg` — UNARY negation `T → T` (`(Int64.neg x)` = `0 - x`), the first-class NAMED form of prefix
    // `(- e)`. Offered only on SIGNED widths: negating an unsigned value underflow-traps on every nonzero
    // input, so an unsigned `neg` would be a near-useless always-trapping op. A constant folds (and
    // `0 - min` traps CDZ0304) via the shared `lower_negate` path.
    if signed {
        fields.push(neg_field(ast, signed, width));
    }
    let mut children = vec![head];
    children.append(&mut fields);
    push_list(ast, children)
}

/// A `(wrap (record ((meta t) TYPE-LAMBDA) ((meta apply) (intrinsic wrap))))` field — the module's
/// truncating conversion. `TYPE-LAMBDA` is `(fn (a) (-> (Int a) TARGET))`: the source is the generic
/// integer `(Int a)` (width `a` + its paired sign variable, so it accepts ANY integer), the result is
/// `TARGET` = `(Int width)` / `(UInt width)`, this module's own concrete type. `(meta apply)` is the
/// shared `wrap` intrinsic — one prim, the target read off the application's solved type at lowering.
fn wrap_field(ast: &mut Arenas, signed: bool, width: u32) -> StructId {
    // `(fn (a) (-> (Int a) TARGET))`.
    let int_a = {
        let int = push_atom(ast, Leaf::Name("Int".into()));
        let a = push_atom(ast, Leaf::Name("a".into()));
        push_list(ast, vec![int, a])
    };
    let target = {
        let ctor = push_atom(ast, Leaf::Name(if signed { "Int" } else { "UInt" }.into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let arr = push_atom(ast, Leaf::Name("->".into()));
    let body = push_list(ast, vec![arr, int_a, target]);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let params = push_list(ast, vec![a_param]);
    let lambda = push_list(ast, vec![fn_head, params, body]);
    // `(record ((meta t) lambda) ((meta apply) (intrinsic wrap)))`.
    let rec_head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let t_field = meta_field(ast, "t", lambda);
    let prim = intrinsic_node(ast, "wrap");
    let apply_field = meta_field(ast, "apply", prim);
    let record = push_list(ast, vec![rec_head, t_field, apply_field]);
    // `(wrap record)`.
    let k = push_atom(ast, Leaf::Name("wrap".into()));
    {
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, k, record])
    }
}

/// A `(of (record ((meta t) TYPE-LAMBDA) ((meta apply) (intrinsic checked-of))))` field — the module's
/// CHECKED conversion. Identical SHAPE to `wrap_field` (`TYPE-LAMBDA` = `(fn (a) (-> (Int a) TARGET))`:
/// a fully-polymorphic source integer `(Int a)` → this module's own concrete `TARGET`), so the target
/// width is read off the application's solved type at lowering exactly as `wrap`'s is. The ONLY
/// difference is the intrinsic — `checked-of`, which TRAPS on an out-of-range value where `wrap`
/// truncates. One prim serves every target width (no pair-explosion), like `wrap`.
fn of_field(ast: &mut Arenas, signed: bool, width: u32) -> StructId {
    // `(fn (a) (-> (Int a) TARGET))`.
    let int_a = {
        let int = push_atom(ast, Leaf::Name("Int".into()));
        let a = push_atom(ast, Leaf::Name("a".into()));
        push_list(ast, vec![int, a])
    };
    let target = {
        let ctor = push_atom(ast, Leaf::Name(if signed { "Int" } else { "UInt" }.into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let arr = push_atom(ast, Leaf::Name("->".into()));
    let body = push_list(ast, vec![arr, int_a, target]);
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let a_param = push_atom(ast, Leaf::Name("a".into()));
    let params = push_list(ast, vec![a_param]);
    let lambda = push_list(ast, vec![fn_head, params, body]);
    // `(record ((meta t) lambda) ((meta apply) (intrinsic checked-of)))`.
    let rec_head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let t_field = meta_field(ast, "t", lambda);
    let prim = intrinsic_node(ast, "checked-of");
    let apply_field = meta_field(ast, "apply", prim);
    let record = push_list(ast, vec![rec_head, t_field, apply_field]);
    // `(of record)`.
    let k = push_atom(ast, Leaf::Name("of".into()));
    {
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, k, record])
    }
}

/// A `(name (record ((meta t) TYPE) ((meta apply) (intrinsic PRIM))))` field — a CHECKED arithmetic op
/// on this module's width. `TYPE` is `(fn () (-> TARGET (-> TARGET (Option TARGET))))`: both operands and
/// the `Some` payload are `TARGET` = `(Int width)`/`(UInt width)`, this module's own concrete type; the
/// result is `(Option TARGET)`, `Some result` when it fits / `None` on overflow. The zero-param `fn`
/// wrapper makes `scheme_of` read a monomorphic SCHEME, not a bare type-value (see `string_to_int64_type`
/// for why a bare arrow would collapse the op record to `Ty::Type`). `(meta apply)` = the `PRIM`
/// intrinsic (`checked-add`/`checked-mul`), whose target width is read off the solved type at lowering.
fn checked_field(ast: &mut Arenas, name: &str, prim: &str, signed: bool, width: u32) -> StructId {
    let target = |ast: &mut Arenas| {
        let ctor = push_atom(ast, Leaf::Name(if signed { "Int" } else { "UInt" }.into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    // `(Option TARGET)`.
    let option_target = {
        let option = push_atom(ast, Leaf::Name("Option".into()));
        let t = target(ast);
        push_list(ast, vec![option, t])
    };
    // `(-> TARGET (Option TARGET))`.
    let rhs = target(ast);
    let inner = arrow_type(ast, rhs, option_target);
    // `(-> TARGET (-> TARGET (Option TARGET)))`.
    let lhs = target(ast);
    let body = arrow_type(ast, lhs, inner);
    // `(fn () body)`.
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    let lambda = push_list(ast, vec![fn_head, params, body]);
    // `(record ((meta t) lambda) ((meta apply) (intrinsic prim)))`.
    let rec_head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let t_field = meta_field(ast, "t", lambda);
    let prim_node = intrinsic_node(ast, prim);
    let apply_field = meta_field(ast, "apply", prim_node);
    let record = push_list(ast, vec![rec_head, t_field, apply_field]);
    let k = push_atom(ast, Leaf::Name(name.into()));
    {
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, k, record])
    }
}

/// A `(name (record ((meta t) TYPE) ((meta apply) (intrinsic PRIM))))` field — a WRAPPING arithmetic op
/// on this module's width. `TYPE` is `(fn () (-> TARGET (-> TARGET TARGET)))`: both operands and the
/// result are `TARGET` = `(Int width)`/`(UInt width)`, this module's own concrete type — no `Option`,
/// wrapping never fails (two's-complement wraparound modulo 2^width). The zero-param `fn` wrapper makes
/// `scheme_of` read a monomorphic SCHEME (see `checked_field`/`string_to_int64_type`). `(meta apply)` =
/// the `PRIM` intrinsic (`wrapping-add`/`wrapping-mul`), whose target width is read off the solved type.
fn wrapping_field(ast: &mut Arenas, name: &str, prim: &str, signed: bool, width: u32) -> StructId {
    let target = |ast: &mut Arenas| {
        let ctor = push_atom(ast, Leaf::Name(if signed { "Int" } else { "UInt" }.into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    // `(-> TARGET TARGET)`.
    let rhs = target(ast);
    let out = target(ast);
    let inner = arrow_type(ast, rhs, out);
    // `(-> TARGET (-> TARGET TARGET))`.
    let lhs = target(ast);
    let body = arrow_type(ast, lhs, inner);
    // `(fn () body)`.
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    let lambda = push_list(ast, vec![fn_head, params, body]);
    // `(record ((meta t) lambda) ((meta apply) (intrinsic prim)))`.
    let rec_head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let t_field = meta_field(ast, "t", lambda);
    let prim_node = intrinsic_node(ast, prim);
    let apply_field = meta_field(ast, "apply", prim_node);
    let record = push_list(ast, vec![rec_head, t_field, apply_field]);
    let k = push_atom(ast, Leaf::Name(name.into()));
    {
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, k, record])
    }
}

/// A `(neg (record ((meta t) TYPE) ((meta apply) (intrinsic neg))))` field — UNARY negation on this
/// module's width. `TYPE` is `(fn () (-> TARGET TARGET))`: operand and result are `TARGET` = `(Int width)`,
/// this module's own concrete SIGNED type (offered only on signed widths). The zero-param `fn` wrapper
/// makes `scheme_of` read a monomorphic SCHEME (see `checked_field`/`wrapping_field`). `(meta apply)` = the
/// `neg` intrinsic; it lowers through the same `lower_negate` (`0 - e`) prefix `(- e)` uses, so a constant
/// folds (and `0 - min` traps CDZ0304). The named first-class form of prefix negation.
fn neg_field(ast: &mut Arenas, signed: bool, width: u32) -> StructId {
    let target = |ast: &mut Arenas| {
        let ctor = push_atom(ast, Leaf::Name(if signed { "Int" } else { "UInt" }.into()));
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    // `(-> TARGET TARGET)`.
    let rhs = target(ast);
    let out = target(ast);
    let body = arrow_type(ast, rhs, out);
    // `(fn () body)`.
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let params = push_list(ast, vec![]);
    let lambda = push_list(ast, vec![fn_head, params, body]);
    // `(record ((meta t) lambda) ((meta apply) (intrinsic neg)))`.
    let rec_head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let t_field = meta_field(ast, "t", lambda);
    let prim_node = intrinsic_node(ast, "neg");
    let apply_field = meta_field(ast, "apply", prim_node);
    let record = push_list(ast, vec![rec_head, t_field, apply_field]);
    let k = push_atom(ast, Leaf::Name("neg".into()));
    {
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, k, record])
    }
}

/// A `(name (: value (Int/UInt width)))` record field — an arbitrary-precision integer constant
/// ANNOTATED with the module's own width, so projecting the field yields a value typed at that width
/// (mirrors `eval::named_int_field`; the two builders must annotate identically so a named width and
/// `(Int N)` project the same typed bound).
fn int_field(ast: &mut Arenas, name: &str, value: IntValue, signed: bool, width: u32) -> StructId {
    let k = push_atom(ast, Leaf::Name(name.into()));
    let lit = push_atom(
        ast,
        Leaf::Int {
            value,
            radix: Radix::Dec,
        },
    );
    let ctor = push_atom(ast, Leaf::Name(if signed { "Int" } else { "UInt" }.into()));
    let w = push_atom(
        ast,
        Leaf::Int {
            value: IntValue::from_i64(width as i64),
            radix: Radix::Dec,
        },
    );
    let ty_expr = push_list(ast, vec![ctor, w]);
    let colon = push_atom(ast, Leaf::Name(":".into()));
    let annot = push_list(ast, vec![colon, lit, ty_expr]);
    {
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, k, annot])
    }
}

/// A `(name (unrealized name))` record field: the field exists, but its value is an `unrealized`
/// form that `resolve` turns into a decline — so projecting it declines by the ordinary path, no
/// open-module special case. The op name rides along so the decline can say which operation it is.
fn unrealized_field(ast: &mut Arenas, name: &str) -> StructId {
    let k = push_atom(ast, Leaf::Name(name.into()));
    let head = push_atom(ast, Leaf::Name("unrealized".into()));
    let who = push_atom(ast, Leaf::Name(name.into()));
    let v = push_list(ast, vec![head, who]);
    {
        let eq = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eq, k, v])
    }
}

/// Append a leaf and an `Atom` occurrence of it, returning the occurrence's id. (No dedup — the
/// prelude is small and its leaves need not be interned against the program's.) `pub(crate)` so the
/// program-driven sum-record synthesis appends its atoms through the same helper.
pub(crate) fn push_atom(ast: &mut Arenas, leaf: Leaf) -> StructId {
    let lid = LeafId(ast.leaves.len() as u32);
    ast.leaves.push(leaf);
    let sid = StructId(ast.structure.len() as u32);
    ast.structure.push(Struct::Atom(lid));
    sid
}

/// Append a `List` occurrence, returning its id. `pub(crate)` so the program-driven sum-record
/// synthesis builds its lists through the same helper.
pub(crate) fn push_list(ast: &mut Arenas, children: Vec<StructId>) -> StructId {
    let sid = StructId(ast.structure.len() as u32);
    ast.structure.push(Struct::List(children));
    sid
}
