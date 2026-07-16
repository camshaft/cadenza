//! `Ty` → native Rust type — the Rust backend's value strategy for scalars.
//!
//! A backend that uses its target's native aggregates maps each Cadenza type to a target type
//! (`backends-and-targets.md` §A Compound Value's Representation Is The Backend's Choice). For the
//! scalar value language that is a total, obvious map: a Cadenza integer of an ALIASED width is the
//! Rust integer of that exact width and signedness (`Int8` → `i8`, `UInt32` → `u32`, `Int64` → `i64`,
//! `UInt64` → `u64`), so Cadenza's checked-overflow arithmetic becomes Rust's `checked_*` on the same
//! type — no representation gap. Bool → `bool`, Unit → `()`.
//!
//! A NON-ALIASED width (`UInt7`, `UInt24`, `UInt48`, …) has no native Rust type, so it maps to `None`
//! and the caller declines — the SAME boundary the wasm backend draws (`comp_valtype_of` returns
//! `None` for a non-standard width, so it cannot cross the component boundary either): a narrow
//! non-standard width is a fine INTERNAL type but has no wire/native form, so a value of it must be
//! converted (`.wrap`) to an aliased width before it crosses out. A compound, function, or type value
//! also maps to `None` in this scalar slice (compounds arrive with the native-aggregate strategy in a
//! later increment).

use crate::backend::Db;
use crate::ty::{IntTy, Sign, Ty, Width};

/// The native Rust type for a solved Cadenza type, or `None` if this backend has no native
/// representation for it (a non-aliased integer width, a not-yet-supported compound, or an
/// unresolved/erased type). The caller turns a `None` into a decline attributed to this target.
///
/// Returns an owned `String` because a compound type is a COMPOSED spelling (a tuple `(T0, T1)`), not a
/// fixed primitive name. A scalar's mapping is still one of a fixed set (`int_type`/`bool`/`()`).
pub fn rust_type(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Int(it) => int_type(*it).map(String::from),
        // A float maps to Rust's native `f64`/`f32` by its width — the admitted {32,64} are exactly
        // Rust's two IEEE float types. (A non-admitted width never reaches here — it is CDZ0302.)
        Ty::Float(ft) => Some(
            if ft.ground_width() == 32 {
                "f32"
            } else {
                "f64"
            }
            .to_string(),
        ),
        Ty::Bool => Some("bool".to_string()),
        Ty::Unit => Some("()".to_string()),
        // An arbitrary-precision integer maps to `cdz_num::Big` — the SAME bignum the wasm runtime uses,
        // shared by SOURCE (`cdz-num` `#[path]`-includes the runtime's `bigint.rs`; see that crate). A
        // `Big` is a value type (`Clone + Eq`, total `cmp`), so it composes as a tuple/record element, a
        // sum payload, a `Vec`/`BTreeSet`/`BTreeMap` element/key. Non-Copy (owns a limb `Vec`) → the
        // clone-on-read discipline covers a shared BigInt (`ty_is_non_copy` includes it). The gate links
        // the `cdz-num` rlib via `--extern cdz_num`.
        Ty::BigInt => Some("cdz_num::Big".to_string()),
        // An exact rational maps to `cdz_num::Rational` (a `Big` num/den pair in the runtime's canonical
        // normalized form). Its ops mirror the wasm runtime's `rational-*` byte-for-byte, so a rust program
        // computes the same rational value. Non-Copy (owns two limb `Vec`s) → clone-on-read.
        Ty::Rational => Some("cdz_num::Rational".to_string()),
        // A QUANTITY is a COMPILE-TIME-only dimension over an inner numeric magnitude — `lower` erases the
        // `Ty::Qty` wrapper, so the emitted VALUE is just the inner magnitude. Map to `rust_type(inner)`;
        // the unit is recovered for the boundary render from the `cdz-return` note's `render_name` and
        // rendered as `((. Qty of) <magnitude> <unit-value-form>)` by the gate harness, which splices the
        // unit from the backend's `// cdz-unit[…]` note (`Unit::render_value_form`) — so ANY unit SHAPE is
        // renderable (a simple base, a power `meter²`, a product, a `Unit./` quotient for a velocity). The
        // one remaining restriction is SCALE-1: a non-scale-1 unit (`5 mile`, `5 kilometer`) DISPLAY-scales
        // its magnitude to the reference (`5 mile` → `201168/25 meter`), which needs per-inner-type magnitude
        // arithmetic in the emit — a later increment — so it still declines here. A scale-1 unit stores the
        // magnitude RAW (the displayed number IS the stored one), so the value emit is just the inner type.
        Ty::Qty { inner, unit } if unit.scale() == (1, 1) => rust_type(inner),
        // A CHAR is a single Unicode scalar value — Rust's native `char` (which IS a Unicode scalar,
        // exactly the Cadenza model). Copy, so no clone-on-read needed. Lets a `Char` cross as a sum
        // payload / tuple element (a `(Tok (Ch Char))` enum) and a `ConstChar` emit as a `'…'` literal.
        Ty::Char => Some("char".to_string()),
        // A tuple is Rust's native tuple: `(T0, T1, …)` — each element mapped recursively (so a nested
        // tuple / a tuple of scalars composes). A 1-tuple is written `(T,)` (Rust needs the trailing
        // comma to distinguish it from a parenthesized type). An element with no native mapping declines
        // the whole tuple. (The empty tuple `Ty::Tuple([])` is distinct from `Unit` upstream, but has no
        // element to map — render it as `()`, Rust's unit/empty-tuple type.)
        Ty::Tuple(elems) => tuple_type(elems.iter()),
        // A RECORD is structural (anonymous) in Cadenza and at run time IS a positional array in
        // sorted-field-name order (a record field read is a `Core::Proj` at the field's sorted index —
        // the SAME machinery a tuple uses). So it maps to the SAME Rust tuple as a tuple of its fields'
        // types IN SORTED KEY ORDER: `(record (b Bool) (a Int64))` → `(i64, bool)` (a before b). The
        // `BTreeMap` already iterates sorted, so this is just the tuple mapping over its values. Field
        // NAMES are compile-time-only (they became sorted positions) — the emitted `.rs` reads fields
        // positionally (`r.0`); the names re-appear only in the boundary render (`(record (a …) …)`).
        // (When Cadenza gains NOMINAL records, THAT is when a named Rust struct becomes the right
        // emission — the name will come from the language, not be synthesized.)
        Ty::Record(fields) => tuple_type(fields.values()),
        // A SUM is a NOMINAL type — unlike a record it HAS a name (the declared sum name), so it maps to
        // a Rust ENUM of that name (the backend emits the `enum <Name> { … }` declaration separately).
        // A generic sum instantiation carries its type ARGS (`Option Int64` → args `[Int64]`), which
        // become the Rust type parameters: `Option<i64>`. A monomorphic sum (`Sign`, no args) is the
        // bare name. The enum name is sanitized (a `-` in a sum name → `_`), matching the declaration.
        Ty::Sum { name, args, .. } => {
            let ident = sum_ident(name);
            if args.is_empty() {
                Some(ident)
            } else {
                let mut params = Vec::with_capacity(args.len());
                for a in args.iter() {
                    params.push(rust_type(a)?);
                }
                Some(format!("{ident}<{}>", params.join(", ")))
            }
        }
        // A NOMINAL newtype erases at run time to its underlying structural value (the tag "adds nothing
        // to the value's runtime representation", `type-system.md §156`), so it maps to the SAME Rust type
        // as its underlying type — a transparent alias. `(type UserId (Mk Int64))` → `i64`, `(type Point
        // (Mk Int64 Int64))` → `(i64, i64)`. (A named Rust newtype struct is a possible future
        // refinement; the erased mapping is correct and matches the wasm backend's read-through.)
        Ty::Nominal { inner, .. } => rust_type(inner),
        // A FUNCTION value is a first-class closure — `Rc<dyn Fn(A, …) -> R>`. `Rc` (not `Box`) so it is
        // CLONE-able: a closure bound/used in more than one position is cloned on read (the tick-5
        // clone-on-read discipline), and `Box<dyn Fn>` is not Clone.
        //
        // The arrow SPINE is FLATTENED: `(-> A (-> B C))` → `Rc<dyn Fn(A, B) -> C>`, NOT a nested
        // `Rc<dyn Fn(A) -> Rc<dyn Fn(B) -> C>>`. A runtime closure is lambda-LIFTED to a FLAT function of
        // all its parameters and applied at FULL arity by a `Core::CallClosure` carrying every arg (a
        // PARTIAL application of a runtime multi-param closure declines at the application site — the wasm
        // backend draws the same line), so the runtime representation is flat and the type must match it.
        // Each parameter and the final (non-arrow) result maps recursively; any with no native mapping
        // declines the whole function type.
        Ty::Fn(_, _) => {
            let mut params = Vec::new();
            let mut cur = ty;
            while let Ty::Fn(p, r) = cur {
                params.push(rust_type(p)?);
                cur = r;
            }
            let ret = rust_type(cur)?;
            Some(format!(
                "std::rc::Rc<dyn Fn({}) -> {ret}>",
                params.join(", ")
            ))
        }
        // A LIST is a homogeneous sequence — Rust's native growable sequence `Vec<T>` (the wasm backend
        // builds it on the persistent `vec-*` heap; here it is an owned `Vec`). The element type maps
        // recursively, so a `List Int64` → `Vec<i64>` and a nested `List (List Int64)` → `Vec<Vec<i64>>`
        // compose; an element with no native mapping declines the whole list. (Cadenza lists are
        // PERSISTENT/immutable; a `Vec` is owned+`Clone`, and every list op emitted for this backend
        // produces a NEW `Vec` — no in-place mutation — so the value semantics agree.)
        Ty::List(elem) => Some(format!("Vec<{}>", rust_type(elem)?)),
        // A MAP is a persistent key→value association — Rust's ordered `BTreeMap<K, V>`. `BTree` (not
        // `HashMap`) because it ITERATES IN SORTED KEY ORDER, which is exactly the CANONICAL order Cadenza's
        // `Map.to-list` yields — so enumeration matches the runtime for free (a `HashMap` would need an
        // explicit sort). Keys compare BY VALUE (`BTreeMap` uses `Ord`), matching the language's by-value
        // key semantics. `K` must be `Ord` — but a FLOAT (or a float-containing tuple/record/SUM) is only
        // `PartialOrd`, so a float-keyed `BTreeMap` fails to compile (E0277). The Ord CHECK is NOT here:
        // `rust_type` is pure (no `Db`), so it cannot resolve whether a `Ty::Sum` KEY's enum derives `Ord`
        // (that needs the sum's payloads). The decline is enforced by the `Db`-aware `enums::ty_ord_key_ok`
        // gate at the emit boundary (`sum_representable` for a param/result Map/Set TYPE; the construction
        // ops `MapNew`/`MapInsert`/`SetOf`/`SetInsert` in `expr.rs` for a VALUE). So this arm just maps the
        // shape; a non-Ord key/element is caught before it can emit an uncompilable `BTreeMap`/`BTreeSet`.
        Ty::Map(k, v) => Some(format!(
            "std::collections::BTreeMap<{}, {}>",
            ord_key_type(k)?,
            rust_type(v)?
        )),
        // A SET is a persistent collection of unique elements — Rust's ordered `BTreeSet<T>` (sorted
        // iteration = the canonical `Set.to-list` order; `Ord` element compares by value, dedup at insert).
        // The Ord-element decline is enforced by the `Db`-aware gates (see the `Ty::Map` note), not here.
        Ty::Set(elem) => Some(format!(
            "std::collections::BTreeSet<{}>",
            ord_key_type(elem)?
        )),
        // A BYTES value is a raw byte sequence — Rust's owned `Vec<u8>`. Non-Copy (owned heap buffer) →
        // clone-on-read covers a shared bytes value. (Cadenza's `Bytes` is a persistent rope at run time;
        // the native rep is a flat `Vec<u8>`, and every emitted bytes op produces a NEW `Vec` — the
        // rope-vs-flat distinction is invisible at the value level, so `Bytes.compact` is a no-op here.)
        Ty::Bytes => Some("Vec<u8>".to_string()),
        // A STRING is a UTF-8 text value — Rust's owned `String`. A Cadenza string counts UNICODE SCALAR
        // VALUES (not bytes), which Rust's `String`/`.chars()` model directly. Non-Copy (owned heap
        // buffer) → clone-on-read covers a shared string. (`String` is `Ord`, so it can also key a
        // `BTreeMap` — unblocking String-keyed maps that declined while `String` had no rep.)
        Ty::String => Some("String".to_string()),
        // Functions and type/erased values have no native mapping. (A `Ty::Symbol` has no rust rep yet —
        // the rust-backend Symbol representation, incl. its render/const/conversion handling, is a
        // separate v-rust-backend increment; a runtime Symbol op declines cleanly on rust until then,
        // while the wasm side emits it as a tagless byte-leaf retag.)
        _ => None,
    }
}

/// The Rust type for a Set ELEMENT / Map KEY position — like [`rust_type`], except a bare `Float` maps to
/// the total-order wrapper `CdzF64` (a `BTreeSet`/`BTreeMap` needs `Ord`, which `f64` lacks). This is the
/// ONLY place a float becomes `CdzF64`: a float in a NON-key position (a value, a tuple element, a Map
/// VALUE) stays a bare `f64`. A float NESTED inside a compound KEY (a `(Tuple Float Int64)` key) is NOT
/// handled here — that still declines via `ty_is_ord` (the wrapper would have to be threaded through the
/// tuple, a later increment); this covers the bare-`Float` key/element the corpus exercises. Any other
/// type falls through to `rust_type` unchanged.
pub(super) fn ord_key_type(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Float(_) => Some("CdzF64".to_string()),
        _ => rust_type(ty),
    }
}

/// Whether `ty` maps to a Rust type that implements `Ord` — the bound `BTreeSet<T>`/`BTreeMap<K,_>`
/// requires of its element/key. Every scalar/text/compound the backend represents IS `Ord` EXCEPT a
/// FLOAT (and anything CONTAINING one): Rust's `f64`/`f32` are `PartialOrd` but NOT `Ord` (NaN breaks
/// totality), so a float in an ordered position (the Set element or Map key, or nested inside a
/// tuple/record/list/set/map/SUM used there) makes the `BTree*` uninstantiable (E0277). The runtime
/// orders a float by its canonical bytes (so wasm supports a float key/element), but the Rust backend
/// has no total float order, so it DECLINES rather than emit an uncompilable `BTreeSet<f64>`.
///
/// A `Ty::Sum`/`Ty::Nominal` is orderable iff its emitted enum derives `Ord` — which is iff it derives
/// `Eq` (see `enums::emit_one_enum`: both gate on `sum_derives_eq`, since Rust's `Ord` derive composes
/// over the SAME payload fields `Eq` does). So the SUM case delegates to the `Db`-aware
/// `enums::ty_derives_eq`. This CLOSES the float-carrying-sum hole (Copilot PR#455): the old version
/// returned `true` for EVERY `Ty::Sum`, so a `Set`/`Map` keyed by a sum whose enum canNOT derive `Ord`
/// (a float / non-`Eq` payload) slipped past and the backend emitted an uncompilable
/// `BTreeSet<Enum>`/`BTreeMap<Enum,_>` — the exact failure the float-key decline prevents, one shape over.
///
/// NOT a blanket delegation to `ty_derives_eq`, because Ord ≠ current-Eq for the NUMERIC types: a
/// `BigInt`/`Rational` has a total order (usable as a `BTree*` key — the CHAMP orders it by canonical
/// bytes on wasm; a rust `cdz_num::Big` has a total `cmp`) even though it is not yet native-`Eq`-derivable
/// on the rust backend. Treating those as non-Ord would REGRESS the passing BigInt-keyed set/map cases.
/// So keep the structural walk (with the sum case now Db-aware) rather than reusing the Eq predicate.
pub(super) fn ty_is_ord(db: &mut Db, ty: &Ty) -> bool {
    match ty {
        // A float is `PartialOrd` but NOT `Ord` — the one scalar that cannot key a `BTree*`.
        Ty::Float(_) => false,
        // Compounds are `Ord` iff every ordered component is. Recurse over BORROWS directly — `db: &mut Db`
        // and the element borrows (which come from `ty`, not `db`) don't conflict, so no clone is needed
        // (the earlier `.clone()`s were over-defensive; this mirrors `enums::ty_derives_eq`'s
        // allocation-free walk over the same shapes). PR#460 (Copilot) cleanup.
        Ty::Tuple(elems) => elems.iter().all(|e| ty_is_ord(db, e)),
        Ty::Record(fields) => fields.values().all(|t| ty_is_ord(db, t)),
        Ty::Nominal { inner, .. } => ty_is_ord(db, inner),
        Ty::List(e) | Ty::Set(e) => ty_is_ord(db, e),
        Ty::Map(k, v) => ty_is_ord(db, k) && ty_is_ord(db, v),
        // A SUM/NOMINAL is orderable iff its enum derives `Ord` = iff it derives `Eq` — the Db-aware check
        // that closes the float-carrying-sum hole (the whole point of this fix).
        Ty::Sum { .. } => crate::backend::rust::enums::ty_derives_eq(
            db,
            ty,
            &mut std::collections::HashSet::new(),
        ),
        // Every other representable type (Int/Bool/Unit/Char/String/Bytes and the NUMERIC BigInt/Rational,
        // which have a total order) maps to an `Ord` Rust type. A non-representable type declines earlier.
        _ => true,
    }
}

/// Whether `ty` can occupy a Set ELEMENT / Map KEY position directly — the gate the construction ops and
/// the boundary use. Like [`ty_is_ord`], EXCEPT a BARE `Float` is now OK: it maps to the total-order
/// wrapper `CdzF64` (see [`ord_key_type`]), which IS `Ord`. A float NESTED inside a compound key (a
/// `(Tuple Float …)` / a float-payload sum) is still NOT ok — the wrapper is only substituted at the
/// top-level key/element by `ord_key_type`, not threaded through a compound — so those delegate to
/// `ty_is_ord` (which rejects the contained float). This is the ONE place the bare-float relaxation lives;
/// `ty_is_ord` stays strict so its RECURSIVE use (a float inside a compound) keeps declining.
pub(super) fn ty_is_ord_key(db: &mut Db, ty: &Ty) -> bool {
    match ty {
        // A bare float keys/elements via the `CdzF64` wrapper — representable and totally ordered.
        Ty::Float(_) => true,
        // Any other shape uses the strict predicate (a nested float still declines).
        _ => ty_is_ord(db, ty),
    }
}

/// The Rust identifier for a sum type / variant name — sanitized to a valid identifier the same way a
/// def name is (`super::sanitize_ident`: a `-` and any non-ident char → `_`, a Rust keyword → `r#kw`), so
/// the emitted `enum` declaration, every `Name::Variant(...)` construction, and every `match` pattern agree
/// on the spelling.
///
/// ADDITIONALLY escapes a name that collides with a Rust PRIMITIVE TYPE (`i64`, `bool`, `str`, `usize`, …).
/// Unlike a `fn`/binder name (a local `let i64 = …` harmlessly shadows the primitive in value position), an
/// ENUM name appears in TYPE position — `enum i64 { A(i64), … }` makes the field `A(i64)` refer to the ENUM
/// (infinitely sized, rustc E0072) instead of the primitive. A primitive name is not a Rust keyword, so
/// `sanitize_ident` passes it through unescaped; prefix it here (`cdz_ty_i64`) so a user sum named after a
/// primitive is namespace-isolated from the primitive it would otherwise shadow. The prefix is applied to
/// the WHOLE sum/variant-name space uniformly (decl, construct, match all route through here), so they agree.
pub fn sum_ident(name: &str) -> String {
    // INJECTIVITY (a sum name is an emitted TYPE name — two distinct sums that collapse to one enum ident
    // would be a duplicate `enum` / conflated construct+match, rustc E0428). `sanitize_ident` is LOSSY: it
    // maps every non-`[A-Za-z0-9_]` char to `_` and prefixes a leading digit, so `Foo-Bar` and `Foo_Bar`
    // both become `Foo_Bar`. For the sum/variant namespace we need an INJECTIVE map. A "clean" name — a
    // valid Rust identifier start + body (`[A-Za-z_][A-Za-z0-9_]*`) that does NOT begin with the mangle
    // MARKER — passes through `sanitize_ident` LOSSLESSLY (it changes nothing), so keep it (readable, and
    // the common case). Any OTHER name (a lossy char, a leading digit, or a literal MARKER prefix) is
    // HEX-mangled: `MARKER + hex(utf8)`. Two distinct originals give distinct hex; a clean name never
    // begins with MARKER (excluded); a name that literally begins with MARKER is itself mangled — so the
    // clean space and the mangled space are disjoint and each is injective. Result: no two distinct sum
    // names ever share an emitted ident.
    const MARKER: &str = "cdzsum_";
    if is_clean_ident(name) && !name.starts_with(MARKER) {
        let s = super::sanitize_ident(name);
        // A clean name may still be a Rust PRIMITIVE (`i64`, `bool`, …) — valid char-wise but ruinous as an
        // emitted enum name (`enum i64 { A(i64) }` makes the field refer to the enum, E0072). Escape it.
        // (`sanitize_ident` already handled keywords → `r#kw` / `cdz_kw_…`.)
        if is_rust_primitive_type(&s) {
            format!("cdz_ty_{s}")
        } else {
            s
        }
    } else {
        // Lossy / leading-digit / MARKER-prefixed → hex-mangle the whole ORIGINAL name (injective).
        let mut hex = String::with_capacity(name.len() * 2 + MARKER.len());
        hex.push_str(MARKER);
        for b in name.bytes() {
            hex.push_str(&format!("{b:02x}"));
        }
        hex
    }
}

/// Whether `name` is already a valid Rust identifier — starts with `[A-Za-z_]`, and every char is
/// `[A-Za-z0-9_]`. Such a name passes through `sanitize_ident` UNCHANGED (the lossy `→ _` map fires on no
/// char), so it is safe to emit verbatim; any other name must be hex-mangled to stay injective. An empty
/// name is not a valid ident. (A keyword IS a clean ident here — `sanitize_ident` escapes it separately to
/// `r#kw`, which is still injective: distinct keywords give distinct `r#kw`.)
fn is_clean_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Whether `s` names a Rust PRIMITIVE / built-in scalar type — the set an emitted `enum <name>` would
/// shadow in type position. Covers the integer/float widths, `bool`, `char`, `str`, the machine-size ints,
/// and the unit-like `()` is not spellable as an ident so is excluded. NOT the keyword set (that is
/// [`super::is_rust_keyword`], handled by `sanitize_ident`); these are contextual type names, valid as a
/// binder but ruinous as an emitted type name.
fn is_rust_primitive_type(s: &str) -> bool {
    matches!(
        s,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
    )
}

/// The Rust tuple type for a sequence of element types — `(T0, T1, …)`, each mapped recursively; a
/// 1-element tuple is `(T,)` (Rust needs the trailing comma to distinguish it from a parenthesized
/// type); an empty one is `()`. `None` if any element has no native mapping. Shared by `Ty::Tuple` and
/// `Ty::Record` (a record IS a tuple of its fields' types in sorted-key order — the `BTreeMap`'s
/// `.values()` iterate sorted, so passing them here gives the right positional order).
fn tuple_type<'a>(elems: impl Iterator<Item = &'a Ty>) -> Option<String> {
    let mut parts = Vec::new();
    for e in elems {
        parts.push(rust_type(e)?);
    }
    if parts.is_empty() {
        return Some("()".to_string());
    }
    let trailing = if parts.len() == 1 { "," } else { "" };
    Some(format!("({}{trailing})", parts.join(", ")))
}

/// The native Rust integer type for an integer of a given signedness and (grounded) width, or `None`
/// for a width Rust has no primitive for. Rust primitives exist only for 8/16/32/64/128 bits; Cadenza
/// exposes 8/16/32/64 as its aliased boundary widths, so those four (each signed and unsigned) map,
/// and any other width (`UInt7`, `UInt24`, `UInt48`, an odd width) is `None` — no native form, decline.
/// The width is GROUNDED (a still-deferred/variable width takes the default, `Int64`), exactly as the
/// wasm backend grounds an unresolved width at selection.
fn int_type(it: IntTy) -> Option<&'static str> {
    // Only a fixed width has a native primitive; a deferred/variable width grounds to the default (64),
    // matching `IntTy::ground_width`. Read the fixed axes directly so a non-aliased FIXED width (e.g.
    // `UInt 24`) is rejected rather than silently grounded to something wider.
    let width = match it.width {
        Width::Fixed(w) => w,
        Width::Deferred | Width::Var(_) => crate::ty::DEFAULT_INT_WIDTH,
    };
    let signed = match it.sign {
        Sign::Fixed(s) => s,
        Sign::Deferred | Sign::Var(_) => true,
    };
    Some(match (signed, width) {
        (true, 8) => "i8",
        (true, 16) => "i16",
        (true, 32) => "i32",
        (true, 64) => "i64",
        (false, 8) => "u8",
        (false, 16) => "u16",
        (false, 32) => "u32",
        (false, 64) => "u64",
        // Any other width (non-aliased/odd) has no native Rust primitive — decline.
        _ => return None,
    })
}

/// The UNSIGNED Rust integer type whose bit width matches an integer type's slot — the type a constant
/// bit-pattern literal is written in before casting to the signed/target type (mirroring the wasm
/// backend's `to_i64_bits`/`to_i32_bits`, which emit the two's-complement bit pattern). Used by the
/// expression emitter to write `<bits>u64 as i64` etc., so a negative signed value and an unsigned
/// value at/above the signed max share ONE spelling. `None` for a non-aliased width (which declines
/// upstream before a constant of it is emitted).
pub fn unsigned_bits_type(it: IntTy) -> Option<&'static str> {
    let width = match it.width {
        Width::Fixed(w) => w,
        Width::Deferred | Width::Var(_) => crate::ty::DEFAULT_INT_WIDTH,
    };
    Some(match width {
        8 => "u8",
        16 => "u16",
        32 => "u32",
        64 => "u64",
        _ => return None,
    })
}
