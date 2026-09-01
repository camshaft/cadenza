//! `lower::ast_reflect` — lowering of the AST-reflection intrinsics: `ast-lift`/`ast-splice-lift`
//! (constant + active-unquote lifts), `ast-encode`/`ast-decode` (the binary-AST value codec bridge),
//! `print`/`read` (the compile-time s-expression printer + reader, incl. [`SexprReader`]/[`SNode`]),
//! `blake3-of`, and the `Ast` sum's variant-discriminant table ([`AstDiscs`]). Split out of `lower.rs`
//! to keep that module scoped to core lowering; every item here is a leaf called from `lower`'s
//! dispatcher (`compute`) and reads parent-private helpers via `use super::*`.

use super::*;

/// Fold `ast-splice-lift` over a CONSTANT list's element cores. Each element is lifted to its `Ast`
/// reflection node by [`lift_value_to_ast`] (identity for an already-`Ast` element, the matching scalar
/// leaf otherwise, or a recursive `Ast.ListCtor` for a nested list value), and the lifted nodes are
/// returned as a `Core::ListNew` (the spliced `(List Ast)`). Returns `None` if the `Ast` sum is absent OR
/// any element is not liftable this increment (a char, a NaN/Inf float, a runtime element) — decline
/// rather than build a wrong-typed node. `id` seeds nothing structural; the synthesized nodes carry their
/// own `Ty::Sum{Ast}`. This is the constant splice companion of the active-unquote `ast-lift` wrap (which
/// dispatches by inferred TYPE for the runtime case); the two agree on the leaf set + the Ast identity.
pub(super) fn lower_ast_splice_lift(db: &mut Db, id: StructId, elems: &[StructId]) -> Option<Core> {
    // The `Ast` sum type + its variant discriminants (read by name so a decl reordering never mis-tags).
    let disc = ast_variant_discs(db)?;
    let _ = id;
    let mut wrapped = Vec::with_capacity(elems.len());
    for &e in elems {
        wrapped.push(lift_value_to_ast(db, e, &disc)?);
    }
    Some(Core::ListNew {
        elems: wrapped.into(),
    })
}

/// Lift ONE constant value core to its `Ast` reflection node, RECURSIVELY. A value already of type `Ast`
/// is the IDENTITY (a pre-built AST fragment splices as-is); a scalar wraps in its leaf — `ConstInt`→
/// `Ast.Int` (WIDENED to the `BigInt` payload), `ConstFloat`→`Ast.Float`, `ConstBool`→`Ast.Bool`,
/// `ConstStr`→`Ast.Str`; and a CONSTANT list value `#list(…)` reflects to `Ast.ListCtor` of its elements
/// lifted through this same helper (the recursive companion of quote-of-collections, so `,@` of a list of
/// nested lists builds `Ast.ListCtor` children instead of declining). Returns `None` for a value with no
/// reflection this increment (a char, a NaN/Inf float — matched as `ConstFloatNan`/`ConstFloatInf`, not
/// `ConstFloat` — or a runtime element) — decline, never mis-lift.
fn lift_value_to_ast(db: &mut Db, e: StructId, disc: &AstDiscs) -> Option<StructId> {
    // The `Ast` sum's own declaration id — recognizes an element ALREADY of type `Ast` (identity splice).
    let ast_decl = match &disc.ty {
        crate::ty::Ty::Sum { decl, .. } => Some(*decl),
        _ => None,
    };
    if let crate::ty::Ty::Sum { decl, .. } = crate::infer::type_of(db, e).strip_nominal()
        && Some(*decl) == ast_decl
    {
        return Some(e);
    }
    // Otherwise dispatch by CONSTANT core kind to the matching `Ast` node. A value with no reflection this
    // increment (a char, a NaN/Inf float, a runtime element) → `None`, declining the whole splice.
    let (leaf_disc, payload) = match core_of(db, e) {
        Core::ConstInt(v) => (
            disc.int,
            synth_core(db, Core::ConstInt(v), crate::ty::Ty::BigInt),
        ),
        Core::ConstFloat(_) => (disc.float, e),
        Core::ConstBool(_) => (disc.bool, e),
        Core::ConstStr(_) => (disc.str, e),
        // A CONSTANT list value reflects to `Ast.ListCtor` of its recursively-lifted elements — the
        // dedicated collection ctor (metaprogramming.md §Quote Produces An AST Value), the same node
        // `(quote #list(…))` reifies to. A non-liftable element declines the whole (the `?`).
        Core::ListNew { elems } => {
            let mut inner = Vec::with_capacity(elems.len());
            for &el in elems.iter() {
                inner.push(lift_value_to_ast(db, el, disc)?);
            }
            let list_ast = synth_core(
                db,
                Core::ListNew {
                    elems: inner.into(),
                },
                crate::ty::Ty::List(Box::new(disc.ty.clone())),
            );
            (disc.list_ctor, list_ast)
        }
        _ => return None,
    };
    Some(synth_core(
        db,
        Core::SumNew {
            disc: leaf_disc,
            payloads: vec![payload].into(),
        },
        disc.ty.clone(),
    ))
}

/// Lower `ast-lift` (`∀a. a → Ast`) — the RUNTIME active-unquote lift. Wrap the operand's value in the
/// `Ast` leaf its INFERRED type denotes: IDENTITY when the operand is ALREADY `Ast` (the compiler's own
/// `(quasiquote (+ (unquote sub) 1))` with `sub : Ast` — no wrap, the sub-AST is spliced as-is), else a
/// `Core::SumNew` at the matching leaf disc with the operand as payload (`Int64`→`Ast.Int`, `Bool`→
/// `Ast.Bool`, `String`→`Ast.Str`). Works at RUNTIME (the operand core need not be constant — the payload
/// is the operand's own core), which is the whole point over the literal-only reify dispatch. A type with
/// no `Ast` value leaf (a Float, a compound, a function) DECLINES — never a wrong wrap or a miscompile.
pub(super) fn lower_ast_lift(db: &mut Db, operand: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, operand) {
        return Core::Poison(r);
    }
    let Some(disc) = ast_variant_discs(db) else {
        return Core::Poison(Reject::decline(
            "ast-lift: the built-in Ast sum is unavailable",
        ));
    };
    // The `Ast` sum's own declaration id — used to recognize an operand that is ALREADY an `Ast` (then
    // the lift is the identity: a sub-AST spliced into a larger AST needs no wrapping).
    let ast_decl = match &disc.ty {
        crate::ty::Ty::Sum { decl, .. } => Some(*decl),
        _ => None,
    };
    let operand_ty = crate::infer::type_of(db, operand);
    match operand_ty.strip_nominal() {
        // Already an `Ast` — identity. Return the operand's core unchanged (it is the sub-tree to splice).
        crate::ty::Ty::Sum { decl, .. } if Some(*decl) == ast_decl => core_of(db, operand),
        // A scalar the `Ast` sum has a value leaf for — wrap the operand (runtime or constant) as payload.
        // `Ast.Int`'s payload is `BigInt` (a quoted AST stores integers non-lossily), so an operand whose
        // GROUNDED integer type is signed-64 lifts to `Ast.Int` by WIDENING it to `BigInt` first (a bare
        // `,n` let-bound to `42` stays sign/width-Deferred at reify time but grounds to the default
        // signed-64, so it must lift, matching the old Int-only wrap). A CONSTANT source folds to a
        // `Core::ConstInt` retyped `BigInt` (its `IntValue` is already unbounded — value unchanged, static
        // type widens); a RUNTIME source widens via `bigint-of-i64` (B3b). A narrower/unsigned FIXED width
        // mismatches → declines rather than mis-wrap. Without this widen the lifted payload would be a raw
        // i64 where the `BigInt` leaf is required (a rust-backend E0308 / a wasm slot-shape mismatch).
        crate::ty::Ty::Int(it) if it.ground_signed() && it.ground_width() == 64 => {
            let widened = match core_of(db, operand) {
                Core::ConstInt(v) => synth_core(db, Core::ConstInt(v), crate::ty::Ty::BigInt),
                Core::Poison(r) => return Core::Poison(r),
                _ => synth_core(
                    db,
                    Core::BigIntOfI64 { value: operand },
                    crate::ty::Ty::BigInt,
                ),
            };
            Core::SumNew {
                disc: disc.int,
                payloads: vec![widened].into(),
            }
        }
        // An operand ALREADY typed `BigInt` — `Ast.Int`'s payload IS `BigInt`, so wrap it DIRECTLY (no
        // widen). Distinct from the `Ty::Int(64)` arm above (that widens a fixed-width Int64 operand): a
        // `,x` where `x : BigInt` (a let-bound BigInt, a BigInt-returning call spliced into a quasiquote)
        // is already the payload type. Without this arm it would fall to `other => decline` even though
        // `Ast.Int` is exactly the right leaf — the splice-surface regression the payload flip introduced.
        crate::ty::Ty::BigInt => Core::SumNew {
            disc: disc.int,
            payloads: vec![operand].into(),
        },
        // A CONSTANT non-canonical float operand (a NaN) has no canonical value form — lifting it into an
        // `Ast.Float` would reproduce the wasm-traps/rust-accepts split the direct-ctor guard (lower_sum_new)
        // and the splice-lift already close. Decline it here too, so the active-unquote lift `,nan` is
        // consistent with `(Ast.Float nan)` and `,@(list nan)`. A RUNTIME float operand still lifts (a finite
        // runtime float is the common case; a runtime NaN traps uniformly at the escape boundary — the
        // runtime residual owned by the rust-encode side, not compile-declinable). Checked before the wrap.
        crate::ty::Ty::Float(_)
            if matches!(
                core_of(db, operand),
                Core::ConstFloatNan | Core::ConstFloatInf
            ) =>
        {
            // Same PERMANENT correct-reject as the direct-ctor guard (lower.rs) — a coded CDZ0201, the
            // non-finite-float-has-no-value-form family (resolve.rs literal / arith_fold const fold #6893).
            Core::Poison(Reject::coded(
                crate::diag::Code::Malformed,
                "an active unquote of a non-canonical float (a NaN or infinity has no canonical value form) \
                 cannot lift into an `Ast.Float`; a finite float lifts, matching `(Ast.Float nan)`, \
                 `(Ast.Float Infinity)`, and `,@` of a non-canonical-float list",
            ))
        }
        // `Ast.Float`'s payload is Float64, so a width-64-grounded float operand lifts to `Ast.Float`.
        crate::ty::Ty::Float(ft) if ft.ground_width() == 64 => Core::SumNew {
            disc: disc.float,
            payloads: vec![operand].into(),
        },
        crate::ty::Ty::Bool => Core::SumNew {
            disc: disc.bool,
            payloads: vec![operand].into(),
        },
        crate::ty::Ty::String => Core::SumNew {
            disc: disc.str,
            payloads: vec![operand].into(),
        },
        // No `Ast` value leaf for this type (a narrow-width float, a compound, a function, an unresolved
        // var) — decline honestly rather than building a wrong-typed leaf.
        other => Core::Poison(Reject::decline(format!(
            "an active unquote of a runtime {} value has no Ast leaf to lift into \
             (only Int64/Float64/Bool/String and an existing Ast value lift)",
            other.render_name(&db.name_ctx())
        ))),
    }
}

// `Ast.encode`/`Ast.decode` serialize through the SINGLE canonical `cdzast` codec (`crate::codec` —
// header + LEB128 leaf/structure arena), the SAME form the kernel `decode_shell_pipeline`/`codec::decode`
// read (operator ruling 2026-08-15, OPTION A: no bespoke formats — one source of truth). `encode_ast_value`
// builds a cadenza-ast `Arenas` for the `Ast` value + `codec::encode`s it; `arenas_to_ast_value` rebuilds
// the `Ast` value from a `codec::decode`d arena. The `Ast` variants map 1:1 to cadenza-ast leaves/structs.

/// Lower `(Ast.encode t)` — FOLD a compile-time-visible `Ast` value to a `Core::BytesOf` of its
/// canonical bytes; a runtime `Ast` (no visible `Core::SumNew`) declines. A poison operand propagates.
pub(super) fn lower_ast_encode(db: &mut Db, id: StructId, ast_val: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, ast_val) {
        return Core::Poison(r);
    }
    let Some(disc) = ast_variant_discs(db) else {
        return Core::Poison(Reject::decline(
            "Ast.encode: the built-in Ast sum is unavailable",
        ));
    };
    // Build the Ast value into a cadenza-ast `Arenas` and delegate to `codec::encode` — the SINGLE
    // canonical `cdzast` serializer the kernel `decode_shell_pipeline`/`codec::decode` read (operator
    // ruling 2026-08-15, OPTION A: no bespoke formats — one source of truth). So guest `Ast.encode` emits
    // exactly the bytes the kernel decodes, unblocking reducer_publish/git-publish end-to-end.
    let mut b = crate::ast::Builder::new();
    let Some(root) = encode_ast_value(db, ast_val, &disc, &mut b) else {
        // The operand did not fold to an encodable constant AST via `core_of`. Before the generic decline,
        // const-EVALUATE it — `Ast.encode` DEMANDS a compile-time constant, so the general evaluator is the
        // demand signal, and it reaches folds `core_of` does not (a Map/Set query, a recursion, or a
        // composition inside a `const`-param fn whose result is an Ast value):
        //   - a TAKEN `trap` surfaces its MESSAGE as the compile error CDZ0304 (a fail-loud authoring error,
        //     not the generic decline — catches the const-fn trap the inliner lowers to a textless `Trap`);
        //   - a folded Ast VALUE is MATERIALIZED (`cval_to_ast` synthesizes a node whose memoized core IS the
        //     value) and encoded through the SAME canonical `codec::encode` path — so `Ast.encode` folds ANY
        //     operand the evaluator can reduce, byte-identical to a direct fold.
        // A genuinely-runtime AST (the evaluator also declines) falls through to the honest decline below.
        let mut budget: u64 = 1_000_000;
        match const_eval(db, ast_val, &CEnv::default(), &mut budget) {
            Some(CVal::Trap(msg)) => {
                return Core::Poison(Reject::coded(Code::ConstTrap, msg.to_string()));
            }
            Some(cv) => {
                if let Some(synth) = cval_to_ast(db, &cv) {
                    let mut b2 = crate::ast::Builder::new();
                    if let Some(root2) = encode_ast_value(db, synth, &disc, &mut b2) {
                        let bytes = crate::codec::encode(&b2.finish(root2));
                        trace!(target: "rcdzc::fold", node = id.0, len = bytes.len(), "Ast.encode folds a const_eval-reduced AST value to its canonical cdzast bytes");
                        return Core::ConstBytes(bytes.into());
                    }
                }
            }
            None => {}
        }
        // A genuinely-RUNTIME Ast (neither `core_of` nor `const_eval` reduced it to a constant): serialize it
        // at run time via the value-heap `ast-encode` op (heap index 93) over the heap Ast handle, guided by
        // the baked 9-disc descriptor. Byte-identical to the compile-time `codec::encode` fold (the op runs
        // the SAME shared codec). (Was a decline; v-runtime #3634.)
        return Core::AstEncode {
            operand: ast_val,
            discs: bake_ast_discs_9(&disc),
        };
    };
    let bytes = crate::codec::encode(&b.finish(root));
    trace!(target: "rcdzc::fold", node = id.0, len = bytes.len(), "Ast.encode folds a constant AST to its canonical cdzast bytes");
    // Bake the whole canonical document as ONE leaf constant (`Core::ConstBytes`) rather than a
    // `BytesOf` of N per-byte `ConstInt` nodes — a compile-time bytes constant, the substrate a
    // compile-time `Blake3.of`/const-executed transform folds over. Consumers of a compile-time-visible
    // Bytes read it via `const_byte_slice` (which also handles a `BytesOf` of constants).
    Core::ConstBytes(bytes.into())
}

/// Lower `(Blake3.of b)` — the blake3 content hash `Bytes → Bytes`. FOLD a compile-time-visible `Bytes`
/// (a `Core::ConstBytes`, or a `Core::BytesOf` of constants) to the `Core::ConstBytes` of its 32-byte
/// `blake3::hash`; a runtime `Bytes` declines (the runtime lowering to heap op 91 `hash-blake3` is a later
/// increment). A poison operand propagates. The digest is plain UNKEYED BLAKE3-256 over the raw bytes —
/// no key, no domain tag (all domain separation is userspace, D7) — so it is BYTE-IDENTICAL to the runtime
/// op, which calls the same `blake3` crate over the same bytes (design-compiler-primitives.md §9).
pub(super) fn lower_blake3_of(db: &mut Db, id: StructId, bytes: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, bytes) {
        return Core::Poison(r);
    }
    let Some(raw) = const_byte_slice(db, bytes) else {
        // A RUNTIME `Bytes` — lower to the value-heap `hash-blake3` op (heap index 91) via
        // `Core::Blake3Of`. Byte-identical to the compile-time fold below (both call the one `blake3`
        // crate over the same bytes, design-compiler-primitives §9). (P3b runtime half.)
        trace!(target: "rcdzc::lower", node = id.0, "Blake3.of of a runtime Bytes → Core::Blake3Of (runtime hash-blake3 op 91)");
        return Core::Blake3Of { operand: bytes };
    };
    let digest = blake3::hash(&raw);
    trace!(target: "rcdzc::fold", node = id.0, len = raw.len(), "Blake3.of folds a constant Bytes to its 32-byte blake3 digest");
    Core::ConstBytes(digest.as_bytes().to_vec().into())
}

/// Read the raw bytes of a COMPILE-TIME-VISIBLE `Bytes` value, or `None` if it is not fully constant.
/// Handles BOTH representations of a constant byte sequence: a baked `Core::ConstBytes` leaf (the
/// `Ast.encode` fold produces this), and a `Core::BytesOf` whose every element folded to a `ConstInt`
/// in `0..=255` (a `b"…"` literal / `Bytes.of` of constants). A runtime element / a runtime Bytes → `None`.
/// This is the single entry every constant-Bytes fold reads through, so the two representations are
/// interchangeable at every fold site.
pub(super) fn const_byte_slice(db: &mut Db, id: StructId) -> Option<Vec<u8>> {
    match core_of(db, id) {
        Core::ConstBytes(bytes) => Some(bytes.to_vec()),
        Core::BytesOf { elems } => {
            let mut raw = Vec::with_capacity(elems.len());
            for e in elems.iter().copied() {
                match core_of(db, e) {
                    Core::ConstInt(v) => match v.to_i64() {
                        Some(n) if (0..=255).contains(&n) => raw.push(n as u8),
                        _ => return None,
                    },
                    _ => return None,
                }
            }
            Some(raw)
        }
        _ => None,
    }
}

/// Lower `(print t)` — FOLD a compile-time-visible `Ast` value to the `Core::ConstStr` of its canonical
/// re-readable s-expression text (`(Ast.List (list (Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))` → `"(+ 1
/// 2)"`). The text analogue of `Ast.encode`. A runtime `Ast` (no visible `Core::SumNew`) declines; a
/// poison operand propagates. Paired with `lower_read` so `read(print(v)) == v`.
pub(super) fn lower_print(db: &mut Db, ast_val: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, ast_val) {
        return Core::Poison(r);
    }
    let Some(disc) = ast_variant_discs(db) else {
        return Core::Poison(Reject::decline(
            "print: the built-in Ast sum is unavailable",
        ));
    };
    let mut text = String::new();
    if print_ast_value(db, ast_val, &disc, &mut text).is_none() {
        // A RUNTIME `Ast` (no compile-time-visible `Core::SumNew`): render at run time via the value-heap
        // `ast-print` op (heap index 92) over the heap `Ast` handle, guided by a baked disc descriptor. The
        // op mirrors `print_ast_value` byte-for-byte, so runtime print == compile-time print. (Was a decline.)
        return Core::AstPrint {
            operand: ast_val,
            discs: bake_ast_discs(&disc),
        };
    }
    Core::ConstStr(text.into())
}

/// Bake the `Ast` variant discriminants into a `Core::ConstBytes` OPERAND for the runtime `ast-print` (and,
/// later, `ast-encode`/`ast-decode`) heap ops — 7 unsigned-LEB `u32` discs in the fixed slot order
/// `[int, float, bool, str, name, bytes, list]` the runtime reads (`crate::leb128`, the same LEB the runtime's
/// `doc_read_leb` decodes). The runtime classifies heap-`Ast` variants by these discs (looked up BY NAME here,
/// never hardcoded runtime-side), so a reordered `Ast` decl stays correct. A small, constant descriptor.
fn bake_ast_discs(disc: &AstDiscs) -> std::rc::Rc<[u8]> {
    let mut bytes = Vec::new();
    for d in [
        disc.int, disc.float, disc.bool, disc.str, disc.name, disc.bytes, disc.list,
    ] {
        crate::leb128::write_u64(&mut bytes, d as u64);
    }
    bytes.into()
}

/// Bake the descriptor for the runtime `ast-encode`/`ast-decode` ops — SEVENTEEN ULEB `u32` discs in the fixed
/// slot order `[int, float, bool, str, name, list, bytes, char, symbol, list_ctor, tuple_ctor, record_ctor,
/// map_ctor, set_ctor, field_pair, member, rational]` (the `AstDiscs` struct field order, matching the runtime's
/// `read_ast_enc_discs` for ops 93/94). The scalar/name/list/bytes/char/symbol NINE, the SEVEN M2
/// native-collection reflected ctors (Option B), then the native RATIONAL literal (`3/2`): a compound decoded
/// from a ctor-leaf head reflects to the DISTINCT `Ast` ctor variant and a rational literal to `Ast.Rational`,
/// so encode/decode MUST round-trip all 17. (`ast-print` bakes a separate 7-disc descriptor.) The runtime
/// reader is TOTAL over exactly 17 discs and returns `None` on a shorter descriptor — so baking fewer than 17
/// truncates the read and yields an EMPTY runtime `Ast.encode`. By-name lookup, same LEB layout.
fn bake_ast_discs_9(disc: &AstDiscs) -> std::rc::Rc<[u8]> {
    let mut bytes = Vec::new();
    for d in [
        disc.int,
        disc.float,
        disc.bool,
        disc.str,
        disc.name,
        disc.list,
        disc.bytes,
        disc.char,
        disc.symbol,
        disc.list_ctor,
        disc.tuple_ctor,
        disc.record_ctor,
        disc.map_ctor,
        disc.set_ctor,
        disc.field_pair,
        disc.member,
        disc.rational,
    ] {
        crate::leb128::write_u64(&mut bytes, d as u64);
    }
    bytes.into()
}

/// Render a compile-time-visible `Ast` value (a `Core::SumNew` at an Int/Name/List disc) as canonical
/// s-expression text into `out`. Returns `None` if the value is not a fully-constant AST. The canonical
/// spelling is the ordinary s-expression form: `Ast.Int` → the decimal, `Ast.Name` → the bare identifier,
/// `Ast.List` → `(elem elem …)` space-separated. This is the inverse `SexprReader` (in `lower_read`) parses
/// back, so `read(print(v)) == v` for any WELL-FORMED AST — i.e. an `Ast.Name` whose payload is a valid
/// identifier. A `Name` is rendered as its bare word, and the reader classifies a bare token by the
/// language's number/identifier boundary (a DIGIT-LED token is a number). So an `Ast.Name` with a
/// digit-led spelling (`"1.5"`, `"123"` — names that cannot arise from parsing real source, since no valid
/// identifier is digit-led) prints as that numeric text and reads back as `Ast.Float`/`Ast.Int`, NOT the
/// original `Name`: the text round-trip is scoped to grammatically-valid names, matching the surface
/// grammar. The BYTE codec (`Ast.encode`/`Ast.decode`) is total over ANY `Name` string (its tag delimits
/// the payload), so a digit-led name still round-trips there.
fn print_ast_value(db: &mut Db, node: StructId, disc: &AstDiscs, out: &mut String) -> Option<()> {
    let Core::SumNew { disc: d, payloads } = core_of(db, node) else {
        return None;
    };
    if d == disc.int && payloads.len() == 1 {
        let Core::ConstInt(v) = core_of(db, payloads[0]) else {
            return None;
        };
        // `Ast.Int`'s payload is arbitrary-precision `BigInt` (the non-lossy quote/read storage), so render
        // it with `to_decimal_string` (total over any magnitude) rather than `to_i64` — a beyond-i64 literal
        // would make `to_i64()` return None and DECLINE the print of an otherwise fully-constant Ast.
        out.push_str(&v.to_decimal_string());
        Some(())
    } else if d == disc.float && payloads.len() == 1 {
        // A float LITERAL renders as the shortest round-tripping decimal (`float_text`) — always carrying
        // a `.` or `e` so it re-reads as a float, not an int. `read` parses it back to the same f64 bits.
        let Core::ConstFloat(dec) = core_of(db, payloads[0]) else {
            return None;
        };
        out.push_str(&float_text(&dec));
        Some(())
    } else if d == disc.bool && payloads.len() == 1 {
        let Core::ConstBool(b) = core_of(db, payloads[0]) else {
            return None;
        };
        out.push_str(if b { "true" } else { "false" });
        Some(())
    } else if d == disc.str && payloads.len() == 1 {
        // A string LITERAL renders `"…"` with the closed escape set (`\n \t \r \\ \"`) — the same
        // canonical spelling the reader parses back, so `read(print v) == v`. DISTINCT from `Ast.Name`,
        // which renders the bare identifier (a Name is not a quoted string).
        let Core::ConstStr(s) = core_of(db, payloads[0]) else {
            return None;
        };
        out.push('"');
        push_escaped_str(out, &s);
        out.push('"');
        Some(())
    } else if d == disc.name && payloads.len() == 1 {
        let Core::ConstStr(s) = core_of(db, payloads[0]) else {
            return None;
        };
        out.push_str(&s);
        Some(())
    } else if d == disc.bytes && payloads.len() == 1 {
        // A byte-sequence LITERAL renders `b"…"` — printable ASCII verbatim, `\n \t \r \\ \"` named, else
        // `\xNN` (two lowercase hex). A constant `Ast.Bytes` payload is a `Core::BytesOf` of `ConstInt`
        // elements each range-checked to `0..=255` at `lower_bytes_of` (a non-constant element declines
        // there, so no `BytesOf` reaches here). Mirrors cadenza-syntax `literal::escape_bytes` (COPIED, not
        // depended — the rcdzc lib is dependency-free).
        let Core::BytesOf { elems } = core_of(db, payloads[0]) else {
            return None;
        };
        out.push_str("b\"");
        for e in elems.iter() {
            let Core::ConstInt(v) = core_of(db, *e) else {
                return None;
            };
            let b = u8::try_from(v.to_i64().filter(|n| (0..=255).contains(n))?).ok()?;
            match b {
                b'\n' => out.push_str("\\n"),
                b'\t' => out.push_str("\\t"),
                b'\r' => out.push_str("\\r"),
                b'\\' => out.push_str("\\\\"),
                b'"' => out.push_str("\\\""),
                0x20..=0x7e => out.push(b as char),
                // `\xNN` (two lowercase hex) — push the nibbles directly rather than `format!`, which would
                // alloc a temp String per non-printable byte (matters for a large byte-literal).
                _ => {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    out.push('\\');
                    out.push('x');
                    out.push(HEX[(b >> 4) as usize] as char);
                    out.push(HEX[(b & 0xf) as usize] as char);
                }
            }
        }
        out.push('"');
        Some(())
    } else if d == disc.list && payloads.len() == 1 {
        let Core::ListNew { elems } = core_of(db, payloads[0]) else {
            return None;
        };
        out.push('(');
        for (i, e) in elems.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            print_ast_value(db, *e, disc, out)?;
        }
        out.push(')');
        Some(())
    } else {
        None
    }
}

/// Escape a string's contents for a `"…"` literal into `out` — the closed escape set (`\n \t \r \\ \"`),
/// matching `cadenza_syntax::literal::escape_string` (kept in sync — the rcdzc lib is dependency-free, so
/// it cannot call that crate). The exact inverse of `SexprReader::read_string`'s unescape, so a printed
/// `Ast.Str` reads back to the same string.
fn push_escaped_str(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
}

/// The canonical re-readable text of a float `Decimal` — the double it denotes (`to_f64_bits`) rendered
/// as Rust's SHORTEST round-tripping decimal (`{}`), forced to carry a `.` (or `e`) so the reader lexes it
/// as a float, not an integer (a bare `3` would re-read as `Ast.Int`). `SexprReader::read_node` parses
/// this back via `f64` + `Decimal::from_f64`, so `read(print (Ast.Float d)) == (Ast.Float d)` bit-for-bit
/// (the shortest form re-parses to the same double). A non-finite double cannot arise — a `Decimal` is
/// always finite. `-0.0` keeps its sign (`{}` prints `-0`); infinities have no `Decimal`, so never occur.
fn float_text(dec: &crate::ast::Decimal) -> String {
    let f = f64::from_bits(dec.to_f64_bits());
    let s = format!("{f}");
    // `{}` prints an integer-valued double without a fraction (`3` for `3.0`); append `.0` so it re-lexes
    // as a float. A form already carrying `.`/`e`/`inf`/`nan` (the last two cannot occur) is left as-is.
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

/// Lower `(read s)` — the inverse of `print`: FOLD a compile-time-visible `Core::ConstStr` by parsing it
/// as ONE s-expression and reifying it into the `Ast` value it denotes (`"(+ 1 2)"` → `(Ast.List (list
/// (Ast.Name "+") (Ast.Int 1) (Ast.Int 2)))`). A runtime `String` (no visible `Core::ConstStr`) declines;
/// a poison operand propagates. Text that does not parse, or that mentions a leaf the `Ast` sum cannot
/// carry (only Int/Name/List — an integer, a bare atom, or a parenthesized list), declines — never a
/// miscompile. Parses with a SELF-CONTAINED reader for exactly the Int/Name/List subset (the rcdzc lib is
/// dependency-free — it carries no general s-expression reader), the minimal inverse of `print_ast_value`.
pub(super) fn lower_read(db: &mut Db, str_val: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, str_val) {
        return Core::Poison(r);
    }
    let Core::ConstStr(text) = core_of(db, str_val) else {
        return Core::Poison(Reject::unsupported(
            "read of a runtime string is not supported (constant strings only)",
        ));
    };
    let Some(disc) = ast_variant_discs(db) else {
        return Core::Poison(Reject::decline("read: the built-in Ast sum is unavailable"));
    };
    let mut r = SexprReader::new(&text);
    let parsed = r.read_node();
    // Exactly one node must consume the whole input (trailing content is ill-formed).
    match parsed {
        Some(node) if r.at_end() => reify_read_ast(db, &node, &disc),
        // Malformedness is a PERMANENT fact (not reducible-later), so a bad read is a CODED REJECT, not a
        // decline/todo: trailing content after the first s-expression, or text that is not a well-formed
        // s-expression over the Ast subset (unbalanced parens, a stray `)`, an empty/EOF input).
        Some(_) => Core::Poison(
            Reject::coded(
                Code::Malformed,
                "read of text with trailing content after the first s-expression",
            )
            .at(str_val),
        ),
        None => Core::Poison(
            Reject::coded(
                Code::Malformed,
                "read of text that is not a well-formed s-expression over the Ast subset",
            )
            .at(str_val),
        ),
    }
}

/// A parsed s-expression over the `Ast`-value subset: an integer, a bare atom (name), or a list. The
/// minimal grammar `read` accepts — exactly the shapes `print_ast_value` emits, so the two round-trip.
/// Parse an all-ASCII-digits token (with an optional leading `-`) into an arbitrary-precision [`IntValue`],
/// or `None` if it is not a well-formed decimal integer (empty, sign-only, or containing a non-digit). Used
/// by `read` at the >i64 boundary where `str::parse::<i64>` overflows but the token IS an integer literal —
/// accumulates digit-by-digit (`acc*10 + d`) via `IntValue`'s bignum arithmetic, mirroring the
/// `rational_from_literal` idiom, so a beyond-i64 literal reads back as an `Ast.Int` not a misclassified
/// `Ast.Name`. A single `-0`/`0…` reads as zero (canonicalized by `IntValue`).
pub(super) fn parse_bigint_decimal(tok: &str) -> Option<IntValue> {
    let (negative, digits) = match tok.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, tok),
    };
    // Validate first (cheap, allocation-free): this fn is the `Err(_)` arm of `str::parse::<i64>()`, so
    // it's reached for ANY non-i64 token — floats, names, `+`-led — NOT only >i64 integers. A non-digit
    // (or empty) token must fast-return `None` WITHOUT touching the bignum table below, so the common
    // decline path (a float/name that `read` then re-classifies) stays allocation-free.
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // `ten` and the 0..=9 digit `IntValue`s are constants; build them ONCE per process (a `OnceLock`,
    // the crate's existing cache idiom) rather than per call — the prior per-call array setup ran on
    // every non-integer token even though only genuine >i64 integer tokens reach the accumulation loop.
    // Then accumulate `acc = acc*10 + d` reusing the table (no fresh `from_i64` per digit).
    static TABLE: std::sync::OnceLock<(IntValue, [IntValue; 10])> = std::sync::OnceLock::new();
    let (ten, digit_values) = TABLE.get_or_init(|| {
        (
            IntValue::from_i64(10),
            std::array::from_fn(|i| IntValue::from_i64(i as i64)),
        )
    });
    let mut acc = IntValue::zero();
    for b in digits.bytes() {
        acc = acc.mul(ten).add(&digit_values[(b - b'0') as usize]);
    }
    Some(if negative { acc.neg() } else { acc })
}

pub(super) enum SNode {
    Int(IntValue),
    Float(f64),
    Bool(bool),
    Str(String),
    Name(String),
    Bytes(Vec<u8>),
    List(Vec<SNode>),
}

/// Reify a parsed [`SNode`] into the `Ast` value (`Core::SumNew` tree) it denotes and return its Core —
/// the inverse of `print_ast_value`. `Int` → `Ast.Int`, `Name` → `Ast.Name` (identifier as a String
/// payload), `List` → `Ast.List` of the reified children. The `Core`-building twin of `quote::reify`.
fn reify_read_ast(db: &mut Db, node: &SNode, disc: &AstDiscs) -> Core {
    match node {
        SNode::Int(n) => {
            // `Ast.Int`'s payload is `BigInt` (a quoted/read AST stores integers non-lossily), so a
            // read-produced integer node is typed `BigInt` — NOT `Int64`. Typing it `Int64` here made a
            // `read` result carry a raw-i64 payload rep while the sum's declared payload is a boxed BigInt;
            // the mismatch only surfaces when the read Ast meets another Ast in `=`/reify (a rebuilt list),
            // where the equality lowering commits to two different reps → wasm invalid-module / rust E0308.
            // (The `IntValue` is already arbitrary-precision; only the static type widens — matching the
            // `decode_ast_value` retype and the reify grounding.)
            let payload = synth_core(db, Core::ConstInt(n.clone()), crate::ty::Ty::BigInt);
            Core::SumNew {
                disc: disc.int,
                payloads: vec![payload].into(),
            }
        }
        SNode::Float(f) => {
            // Rebuild the `Decimal` from the parsed f64 (exact — `from_f64` round-trips the bits). A
            // non-finite f64 can't arise: `print` never emits `inf`/`nan` and `read_node` only makes a
            // Float from a finite-parsing token. Fall back to a name-free decline path via ConstFloat of 0
            // only if `from_f64` somehow returns None (a finite f64 always yields Some).
            match crate::ast::Decimal::from_f64(*f) {
                Some(dec) => {
                    let payload = synth_core(db, Core::ConstFloat(dec), crate::ty::Ty::float64());
                    Core::SumNew {
                        disc: disc.float,
                        payloads: vec![payload].into(),
                    }
                }
                // Same non-finite-float-has-no-value-form family → coded CDZ0201 (uniform with #6893/#7031).
                // A defensive guard: `print` never emits inf/nan, so `read` cannot construct one — but code
                // it for family uniformity rather than leave a codeless refusal.
                None => Core::Poison(Reject::coded(
                    crate::diag::Code::Malformed,
                    "read: a non-finite float has no Ast.Float value form",
                )),
            }
        }
        SNode::Bool(b) => {
            let payload = synth_core(db, Core::ConstBool(*b), crate::ty::Ty::Bool);
            Core::SumNew {
                disc: disc.bool,
                payloads: vec![payload].into(),
            }
        }
        SNode::Str(s) => {
            let payload = synth_core(db, Core::ConstStr(s.clone().into()), crate::ty::Ty::String);
            Core::SumNew {
                disc: disc.str,
                payloads: vec![payload].into(),
            }
        }
        SNode::Name(name) => {
            let payload = synth_core(
                db,
                Core::ConstStr(name.clone().into()),
                crate::ty::Ty::String,
            );
            Core::SumNew {
                disc: disc.name,
                payloads: vec![payload].into(),
            }
        }
        SNode::Bytes(raw) => {
            // A `b"…"` byte-string reads back to an `Ast.Bytes` whose payload is a `Core::BytesOf` of the
            // raw bytes (the same shape `b"…"`/`Bytes.of` build, and the shape `decode_ast_value`'s bytes
            // arm produces), so `read(print v) == v` for a bytes node. Typed `Ty::Bytes`.
            let elems = bytes_to_elems(db, raw);
            let payload = synth_core(
                db,
                Core::BytesOf {
                    elems: elems.into(),
                },
                crate::ty::Ty::Bytes,
            );
            Core::SumNew {
                disc: disc.bytes,
                payloads: vec![payload].into(),
            }
        }
        SNode::List(items) => {
            let elems: Vec<StructId> = items
                .iter()
                .map(|e| {
                    let core = reify_read_ast(db, e, disc);
                    synth_core(db, core, disc.ty.clone())
                })
                .collect();
            let payload = synth_core(
                db,
                Core::ListNew {
                    elems: elems.into(),
                },
                crate::ty::Ty::List(Box::new(disc.ty.clone())),
            );
            Core::SumNew {
                disc: disc.list,
                payloads: vec![payload].into(),
            }
        }
    }
}

/// A minimal recursive s-expression reader for the `Ast`-value subset — integers, booleans, string
/// literals, bare atoms (names), and parenthesized lists. Self-contained (the rcdzc lib carries no
/// general reader): whitespace-separated tokens, `(`/`)` nesting, a `"…"` token (closed escape set
/// `\n \t \r \\ \"`) is a `Str`, `true`/`false` are `Bool`, a leading-`-`/digit token that fully parses
/// as `i64` is an `Int`, any other bare token is a `Name`. A token the subset cannot represent (a float
/// literal) surfaces as a `Name` of its raw spelling — harmless, since only `print`'s own output is ever
/// round-tripped here.
pub(super) struct SexprReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> SexprReader<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        SexprReader {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }
    /// Skip inter-token whitespace AND `;` line-comments (run to end-of-line). The front-end reader skips
    /// `;`-to-EOL in ALL corpus program text, so `read` — parsing the text of a PROGRAM
    /// (`self-hosting-surface.md`) — must too; otherwise `(read "(+ 1 ; c\n 2)")` tokenizes `;` as a bare
    /// `Name` and yields a 5-element list, silently the WRONG value. A `;` inside a `"…"` string is never
    /// seen here (the string body is consumed by `read_string`), so only genuine inter-token comments skip.
    fn skip_ws(&mut self) {
        loop {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.pos < self.bytes.len() && self.bytes[self.pos] == b';' {
                // A line-comment: consume to the next newline (or end of input).
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue; // loop to skip the newline + any following ws/comment
            }
            break;
        }
    }
    fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.pos >= self.bytes.len()
    }
    /// Parse ONE node from the current position, or `None` on a malformed input (unbalanced parens, an
    /// empty stray `)`, or an empty token).
    pub(super) fn read_node(&mut self) -> Option<SNode> {
        self.skip_ws();
        if self.pos >= self.bytes.len() {
            return None;
        }
        if self.bytes[self.pos] == b'(' {
            self.pos += 1; // consume '('
            let mut items = Vec::new();
            loop {
                self.skip_ws();
                if self.pos >= self.bytes.len() {
                    return None; // unterminated list
                }
                if self.bytes[self.pos] == b')' {
                    self.pos += 1; // consume ')'
                    return Some(SNode::List(items));
                }
                items.push(self.read_node()?);
            }
        }
        if self.bytes[self.pos] == b')' {
            return None; // a stray close-paren
        }
        // A `"…"` string literal (the `print_ast_value` spelling of an `Ast.Str`) — unescape the closed
        // set. `None` on an unterminated string or a bad escape (the read declines, never mis-parses).
        if self.bytes[self.pos] == b'"' {
            return self.read_string().map(SNode::Str);
        }
        // A `b"…"` byte-string literal (the `print_ast_value` spelling of an `Ast.Bytes`) — a `b`
        // IMMEDIATELY followed by `"`. Must precede the bare-token scan (a lone `b` is a valid name-start),
        // mirroring the front-end lexer's `b"…"` rule. Reads the RAW bytes with the byte-literal escape set
        // (`\n \t \r \\ \"` + `\xNN`). `None` on an unterminated literal or a bad escape.
        if self.bytes[self.pos] == b'b' && self.bytes.get(self.pos + 1) == Some(&b'"') {
            self.pos += 1; // consume the `b`; `read_byte_string` consumes the `"…"`
            return self.read_byte_string().map(SNode::Bytes);
        }
        // A bare token: run to the next whitespace, paren, or `;` (comment start — the front-end reader
        // treats `;` as a comment delimiter everywhere, so a token never contains one).
        let start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b.is_ascii_whitespace() || b == b'(' || b == b')' || b == b';' {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return None; // empty token
        }
        let tok = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
        // `true`/`false` are BOOLEAN literals, not names — `print_ast_value` emits an `Ast.Bool` as the
        // bare word, and the lexer never yields a `Name` spelled `true`/`false` FROM SOURCE, so a
        // source-derived round-trip is unambiguous. (A HAND-CONSTRUCTED `Ast.Name "true"` — or a digit-led
        // `Ast.Name "1.5"` — prints its bare word and reads back HERE as the keyword/number it looks like,
        // not the Name: the text round-trip is scoped to grammatically-valid identifiers, the encode/decode
        // byte path is total over any name. Pinned in 12-metaprogramming.) Then a decimal `i64`; then a
        // FLOAT token (one carrying a `.`/`e`/`E` that parses as finite f64 — `print` always renders an
        // `Ast.Float` with a `.`/`e`, so an int-shaped token stays `Ast.Int`); else a bare name.
        match tok {
            "true" => Some(SNode::Bool(true)),
            "false" => Some(SNode::Bool(false)),
            // A leading `+` is NEVER part of a numeric literal: the front-end lexer (cadenza-syntax
            // lexer.rs) makes `+` an operator (`Kind::Plus`) always, begins a number ONLY on an ASCII
            // digit, and folds `-` into a number only when a digit follows. `read` mirrors that reader, so
            // a `+`-prefixed token (`+5`, `+<beyond-i64>`, `+1.5`, or bare `+`) is a NAME, not an Int/Float.
            // Without this guard `str::parse::<i64>`/`parse::<f64>` would silently ACCEPT the `+` (so `+5`
            // read as an `Ast.Int` while `+<beyond-i64>` fell through the bignum path — which strips only
            // `-` — to a `Name`: an i64-boundary inconsistency). `-` is still accepted below (print emits a
            // leading `-` for a negative Int/Float, so it must round-trip). (v-syntax ruling A.)
            _ if tok.starts_with('+') => Some(SNode::Name(tok.to_string())),
            _ => match tok.parse::<i64>() {
                Ok(n) => Some(SNode::Int(IntValue::from_i64(n))),
                Err(_) => {
                    // An i64 parse fails at the 64-bit boundary too, not only on non-numeric tokens. An
                    // all-digits (optionally sign-led) token IS an integer literal at any magnitude, so try
                    // an arbitrary-precision decimal parse BEFORE the float/Name fallback — else a >i64
                    // literal would silently misclassify as an `Ast.Name`, breaking `read(print v) == v`
                    // for the bignum `Ast.Int` the non-lossy feature stores. (`Ast.Int`'s payload is BigInt.)
                    if let Some(big) = parse_bigint_decimal(tok) {
                        return Some(SNode::Int(big));
                    }
                    let looks_float = tok.contains('.') || tok.contains('e') || tok.contains('E');
                    match tok.parse::<f64>() {
                        Ok(f) if looks_float && f.is_finite() => Some(SNode::Float(f)),
                        _ => Some(SNode::Name(tok.to_string())),
                    }
                }
            },
        }
    }

    /// Parse a `"…"` string literal from the current position (which must be the opening `"`), unescaping
    /// the closed set (`\n \t \r \\ \"`) — the exact inverse of `push_escaped_str`. Returns the decoded
    /// string, or `None` on an unterminated literal or an unrecognized escape (the read declines). Operates
    /// over the raw bytes; a decoded byte run is validated as UTF-8 at the end. `print` never emits a
    /// non-UTF-8 `Ast.Str` (its payload is a Rust `String`), so this only rejects genuinely malformed input.
    fn read_string(&mut self) -> Option<String> {
        debug_assert_eq!(self.bytes.get(self.pos), Some(&b'"'));
        self.pos += 1; // consume opening '"'
        let mut out: Vec<u8> = Vec::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            match b {
                b'"' => {
                    self.pos += 1; // consume closing '"'
                    return String::from_utf8(out).ok();
                }
                b'\\' => {
                    // An escape: the next byte selects the char. An unknown escape / a trailing `\` fails.
                    self.pos += 1;
                    match self.bytes.get(self.pos)? {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'\\' => out.push(b'\\'),
                        b'"' => out.push(b'"'),
                        _ => return None, // unrecognized escape — decline
                    }
                    self.pos += 1;
                }
                _ => {
                    out.push(b);
                    self.pos += 1;
                }
            }
        }
        None // unterminated string
    }
    /// Read a `b"…"` byte-string literal's payload as RAW bytes — the inverse of `print_ast_value`'s
    /// `Ast.Bytes` rendering. Called with `self.pos` at the opening `"` (the `b` already consumed). The
    /// escape set matches the printer: `\n \t \r \\ \"` named, `\xNN` (two lowercase-hex) for any other
    /// byte, else the byte verbatim. `None` on an unterminated literal or a malformed escape (declines,
    /// never mis-parses). Distinct from `read_string`: bytes are RAW (not UTF-8-validated) and `\xNN` is
    /// accepted (a string literal has no `\x`).
    fn read_byte_string(&mut self) -> Option<Vec<u8>> {
        debug_assert_eq!(self.bytes.get(self.pos), Some(&b'"'));
        self.pos += 1; // consume opening '"'
        let mut out: Vec<u8> = Vec::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            match b {
                b'"' => {
                    self.pos += 1; // consume closing '"'
                    return Some(out);
                }
                b'\\' => {
                    self.pos += 1;
                    match self.bytes.get(self.pos)? {
                        b'n' => {
                            out.push(b'\n');
                            self.pos += 1;
                        }
                        b't' => {
                            out.push(b'\t');
                            self.pos += 1;
                        }
                        b'r' => {
                            out.push(b'\r');
                            self.pos += 1;
                        }
                        b'\\' => {
                            out.push(b'\\');
                            self.pos += 1;
                        }
                        b'"' => {
                            out.push(b'"');
                            self.pos += 1;
                        }
                        b'x' => {
                            // `\xNN` — exactly two hex digits (the printer emits lowercase; accept any case).
                            self.pos += 1; // consume 'x'
                            let hi = self.bytes.get(self.pos)?;
                            let lo = self.bytes.get(self.pos + 1)?;
                            let byte =
                                u8::from_str_radix(std::str::from_utf8(&[*hi, *lo]).ok()?, 16)
                                    .ok()?;
                            out.push(byte);
                            self.pos += 2;
                        }
                        _ => return None, // unrecognized escape — decline
                    }
                }
                _ => {
                    out.push(b);
                    self.pos += 1;
                }
            }
        }
        None // unterminated byte-string
    }
}

/// The Int/Float/Bool/Str/Name/List discriminants of the built-in `Ast` sum (read by name so a
/// reordering does not silently mis-tag). `None` if the sum or a variant is missing.
pub(super) struct AstDiscs {
    pub(super) int: u32,
    pub(super) float: u32,
    pub(super) bool: u32,
    pub(super) str: u32,
    pub(super) name: u32,
    pub(super) list: u32,
    pub(super) bytes: u32,
    pub(super) char: u32,
    pub(super) symbol: u32,
    // The native-compound-data (Option B) reflected ctors — mirror the codec ctor-head leaf kinds.
    pub(super) list_ctor: u32,
    pub(super) tuple_ctor: u32,
    pub(super) record_ctor: u32,
    pub(super) map_ctor: u32,
    pub(super) set_ctor: u32,
    pub(super) field_pair: u32,
    pub(super) member: u32,
    // The native RATIONAL literal (`3/2`) reflected variant — payload is a `(Tuple Ast Ast)` of num/den.
    pub(super) rational: u32,
    pub(super) ty: crate::ty::Ty,
}
/// Whether a `Core::SumNew { disc }` at result type `ty` constructs the reify `Ast` sum's `Float` variant
/// — the ONE variant whose payload is a float that must be CANONICAL to cross the value-encode boundary (a
/// non-canonical NaN/±inf has no canonical value form). The rust backend uses this to guard a RUNTIME
/// non-canonical float at construction (a compile-time-constant NaN is already declined at `lower_ctor`),
/// matching wasm's runtime value-encode trap. Only the `Ast` sum's Float variant — an ordinary float value
/// (or any other sum's float payload) crosses fine, so this is narrowly the reify escape's obligation.
pub(crate) fn is_ast_float_variant(db: &mut Db, ty: &crate::ty::Ty, disc: u32) -> bool {
    let crate::ty::Ty::Sum { decl, .. } = ty else {
        return false;
    };
    match ast_variant_discs(db) {
        Some(a) => {
            disc == a.float
                && matches!(&a.ty, crate::ty::Ty::Sum { decl: ast_decl, .. } if ast_decl == decl)
        }
        None => false,
    }
}

pub(super) fn ast_variant_discs(db: &mut Db) -> Option<AstDiscs> {
    let ty = {
        let occ = db.type_decls.iter().find(|t| t.name == "Ast")?.occ;
        db.normalize_sum(occ, Vec::new())
    };
    Some(AstDiscs {
        int: variant_disc_by_name(db, &ty, "Int")?,
        float: variant_disc_by_name(db, &ty, "Float")?,
        bool: variant_disc_by_name(db, &ty, "Bool")?,
        str: variant_disc_by_name(db, &ty, "Str")?,
        name: variant_disc_by_name(db, &ty, "Name")?,
        list: variant_disc_by_name(db, &ty, "List")?,
        bytes: variant_disc_by_name(db, &ty, "Bytes")?,
        char: variant_disc_by_name(db, &ty, "Char")?,
        symbol: variant_disc_by_name(db, &ty, "Symbol")?,
        list_ctor: variant_disc_by_name(db, &ty, "ListCtor")?,
        tuple_ctor: variant_disc_by_name(db, &ty, "TupleCtor")?,
        record_ctor: variant_disc_by_name(db, &ty, "RecordCtor")?,
        map_ctor: variant_disc_by_name(db, &ty, "MapCtor")?,
        set_ctor: variant_disc_by_name(db, &ty, "SetCtor")?,
        field_pair: variant_disc_by_name(db, &ty, "FieldPair")?,
        member: variant_disc_by_name(db, &ty, "Member")?,
        rational: variant_disc_by_name(db, &ty, "Rational")?,
        ty,
    })
}

/// Build a compile-time-visible `Ast` value (a `Core::SumNew` at an Int/Float/Bool/Str/Name/List/Bytes
/// disc) into the cadenza-ast `Arenas` builder `b`, returning the `StructId` of the built node. The caller
/// (`lower_ast_encode`) `codec::encode`s the finished arena — the SINGLE canonical `cdzast` form the kernel
/// `decode_shell_pipeline`/`codec::decode` read (operator ruling 2026-08-15, OPTION A: no bespoke formats).
/// The `Ast` variants map 1:1 to cadenza-ast leaves/structs. `None` if the value is not a fully-constant
/// AST (a runtime node, or a payload not the expected constant shape) — the caller then declines.
fn encode_ast_value(
    db: &mut Db,
    node: StructId,
    disc: &AstDiscs,
    b: &mut crate::ast::Builder,
) -> Option<StructId> {
    let Core::SumNew { disc: d, payloads } = core_of(db, node) else {
        return None;
    };
    if d == disc.int && payloads.len() == 1 {
        // `IntValue` is natively (sign, big-endian minimal magnitude) — exactly what `Leaf::Int` carries;
        // an `Ast` value has no radix, so it is decimal. NON-LOSSY (arbitrary precision, no i64 round-trip).
        let Core::ConstInt(v) = core_of(db, payloads[0]) else {
            return None;
        };
        Some(b.atom_leaf(crate::ast::Leaf::Int {
            value: v,
            radix: crate::ast::Radix::Dec,
        }))
    } else if d == disc.float && payloads.len() == 1 {
        // `Leaf::Float` carries the full `Decimal` (negative/exponent/significand) — NOT a lossy f64 bit
        // pattern (the bespoke form's bug); the codec round-trips the Decimal exactly.
        let Core::ConstFloat(dec) = core_of(db, payloads[0]) else {
            return None;
        };
        Some(b.atom_leaf(crate::ast::Leaf::Float(dec)))
    } else if d == disc.bool && payloads.len() == 1 {
        let Core::ConstBool(x) = core_of(db, payloads[0]) else {
            return None;
        };
        Some(b.atom_leaf(crate::ast::Leaf::Bool(x)))
    } else if d == disc.str && payloads.len() == 1 {
        let Core::ConstStr(s) = core_of(db, payloads[0]) else {
            return None;
        };
        Some(b.atom_leaf(crate::ast::Leaf::Str(s)))
    } else if d == disc.name && payloads.len() == 1 {
        let Core::ConstStr(s) = core_of(db, payloads[0]) else {
            return None;
        };
        Some(b.atom_leaf(crate::ast::Leaf::Name(s)))
    } else if d == disc.list && payloads.len() == 1 {
        // A list node is a `Struct::List` of the encoded children (each recursively built into `b`).
        let Core::ListNew { elems } = core_of(db, payloads[0]) else {
            return None;
        };
        let mut children = Vec::with_capacity(elems.len());
        for e in elems.iter().copied() {
            children.push(encode_ast_value(db, e, disc, b)?);
        }
        Some(b.list(children))
    } else if d == disc.bytes && payloads.len() == 1 {
        // A byte-sequence node → `Leaf::Bytes` of the raw bytes. The payload is a `Core::BytesOf` of
        // `ConstInt` elements each range-checked to `0..=255` at `lower_bytes_of` (a non-constant element
        // declines there, so no `BytesOf` reaches here).
        let Core::BytesOf { elems } = core_of(db, payloads[0]) else {
            return None;
        };
        let mut raw = Vec::with_capacity(elems.len());
        for e in elems.iter().copied() {
            let Core::ConstInt(v) = core_of(db, e) else {
                return None;
            };
            raw.push(u8::try_from(v.to_i64().filter(|n| (0..=255).contains(n))?).ok()?);
        }
        Some(b.atom_leaf(crate::ast::Leaf::Bytes(raw.into())))
    } else if d == disc.char && payloads.len() == 1 {
        // A char node → `Leaf::Char` of the scalar (`Core::ConstChar`). The codec stores it as `KIND_CHAR`.
        let Core::ConstChar(c) = core_of(db, payloads[0]) else {
            return None;
        };
        Some(b.atom_leaf(crate::ast::Leaf::Char(c)))
    } else if d == disc.symbol && payloads.len() == 1 {
        // A symbol node → `Leaf::Sym` of the interned text. A constant symbol shares the `Core::ConstStr`
        // rep (at `Ty::Symbol`), so its payload is a `ConstStr` whose text is the symbol name (`KIND_SYM`).
        let Core::ConstStr(s) = core_of(db, payloads[0]) else {
            return None;
        };
        Some(b.atom_leaf(crate::ast::Leaf::Sym(s)))
    } else if let Some(ctor) = [
        (disc.list_ctor, crate::ast::CompoundCtor::List),
        (disc.tuple_ctor, crate::ast::CompoundCtor::Tuple),
        (disc.record_ctor, crate::ast::CompoundCtor::Record),
        (disc.map_ctor, crate::ast::CompoundCtor::Map),
        (disc.set_ctor, crate::ast::CompoundCtor::Set),
    ]
    .iter()
    .find(|(dd, _)| *dd == d)
    .map(|(_, c)| *c)
    {
        // A native compound-ctor Ast value (Option B) → `(<ctor-leaf> child…)`. The payload is a `(List Ast)`
        // of the children (record/map children are FieldPair Ast values), each recursively encoded.
        if payloads.len() != 1 {
            return None;
        }
        let Core::ListNew { elems } = core_of(db, payloads[0]) else {
            return None;
        };
        let mut children = Vec::with_capacity(elems.len());
        for e in elems.iter().copied() {
            children.push(encode_ast_value(db, e, disc, b)?);
        }
        Some(b.compound(ctor, &children))
    } else if (d == disc.field_pair || d == disc.member) && payloads.len() == 1 {
        // Ast.FieldPair `(= k v)` / Ast.Member `(. obj key)` — payload is a `(Tuple Ast Ast)` of the two
        // children; emit the payloadless FieldPair / Member head via the Builder primitives.
        let Core::Tuple { elems } = core_of(db, payloads[0]) else {
            return None;
        };
        if elems.len() != 2 {
            return None;
        }
        let x = encode_ast_value(db, elems[0], disc, b)?;
        let y = encode_ast_value(db, elems[1], disc, b)?;
        Some(if d == disc.field_pair {
            // Emit the native payloadless FIELD_PAIR leaf head directly (rcdzc's Builder::field_pair still
            // emits the legacy Name("=") head; its flip is the const_value_ast/field_pair slice).
            let head = b.atom_leaf(crate::ast::Leaf::FieldPair);
            b.list(vec![head, x, y])
        } else {
            b.member(x, y)
        })
    } else if d == disc.rational && payloads.len() == 1 {
        // Ast.Rational `3/2` — payload is a `(Tuple Ast Ast)` of the numerator/denominator (each an
        // `Ast.Int`); emit the native `(RationalTag <num> <den>)` node via `Builder::rational` (the two
        // children encode to ordinary Int leaves, exactly the wire shape `rational_parts` reads back).
        let Core::Tuple { elems } = core_of(db, payloads[0]) else {
            return None;
        };
        if elems.len() != 2 {
            return None;
        }
        let num = encode_ast_value(db, elems[0], disc, b)?;
        let den = encode_ast_value(db, elems[1], disc, b)?;
        Some(b.rational(num, den))
    } else {
        None
    }
}

/// Build the `Core::BytesOf` element occurrences for a raw byte slice — each a fresh `UInt8` `Leaf::Int`
/// (the shape `String.to-bytes` / `Bytes.of` build).
fn bytes_to_elems(db: &mut Db, bytes: &[u8]) -> Vec<StructId> {
    bytes
        .iter()
        .map(|&b| {
            db.push_atom(crate::ast::Leaf::Int {
                value: IntValue::from_i64(b as i64),
                radix: crate::ast::Radix::Dec,
            })
        })
        .collect()
}

/// Lower `(Ast.decode b)` — the TOTAL inverse of encode. FOLD a compile-time-visible `Core::BytesOf`:
/// parse the WHOLE input as one canonical AST node → `(Ok <SumNew tree>)`; ill-formed / truncated /
/// trailing bytes → `(Err unit)`. NEVER a trap (a bad byte sequence is DATA). A runtime `Bytes` declines;
/// a poison operand propagates.
pub(super) fn lower_ast_decode(db: &mut Db, id: StructId, bytes: StructId) -> Core {
    if let Core::Poison(r) = core_of(db, bytes) {
        return Core::Poison(r);
    }
    // `decode` returns `(Result Ast e)`; read the Ok/Err discriminants off the result Option/Result sum.
    let Some((disc_ok, disc_err)) = result_discs(db, id) else {
        return Core::Poison(Reject::decline(
            "Ast.decode result is not the built-in Result sum",
        ));
    };
    let Some(disc) = ast_variant_discs(db) else {
        return Core::Poison(Reject::decline(
            "Ast.decode: the built-in Ast sum is unavailable",
        ));
    };
    // A poison operand propagates as the decline.
    if let Core::Poison(r) = core_of(db, bytes) {
        return Core::Poison(r);
    }
    // Collect the raw bytes of a compile-time-visible Bytes — a baked `Core::ConstBytes` (what the
    // `Ast.encode` fold now produces) OR a `Core::BytesOf` of constant elements (a `b"…"` literal /
    // `Bytes.of`); a runtime Bytes takes the RUNTIME path below.
    let Some(raw) = const_byte_slice(db, bytes) else {
        // RUNTIME bytes: parse at run time via the value-heap `ast-decode` op (heap index 94) over the Bytes
        // handle, guided by the SAME baked descriptor as `Ast.encode` — byte-identical to the const fold
        // below (the op runs the SAME shared codec). The emit wraps the op's handle-or-0 result as the
        // `(Result Ast e)` sum (a fresh Ast → `(Ok …)`, 0 → `(Err unit)`). The symmetric companion of the
        // runtime `Ast.encode` (`Core::AstEncode`, #3634); the runtime op 94 already existed, only this
        // compiler path was missing (unblocks a caller-boundary round-trip over runtime bytes).
        return Core::AstDecode {
            operand: bytes,
            discs: bake_ast_discs_9(&disc),
            disc_ok,
            disc_err,
        };
    };
    // `codec::decode` parses the WHOLE byte sequence into a cadenza-ast `Arenas` — the canonical `cdzast`
    // decoder (None on a bad header / malformed structure / out-of-range id / TRAILING bytes; it consumes
    // the whole input or refuses, so no separate length check). Then rebuild the `Ast` sum VALUE from the
    // arena's root. Bytes that decode to a cadenza-ast which is NOT an `Ast` value shape (e.g. a Char/Sym
    // leaf) yield `None` → the `Err` arm. Inverse of `encode_ast_value` (operator ruling OPTION A: one
    // canonical codec, no bespoke format). Total: never a trap on untrusted bytes.
    let ok = crate::codec::decode(&raw)
        .and_then(|arenas| arenas_to_ast_value(db, &arenas, arenas.root, &disc));
    match ok {
        Some(node) => {
            trace!(target: "rcdzc::fold", node = id.0, "Ast.decode folds canonical bytes to (Ok ast)");
            Core::SumNew {
                disc: disc_ok,
                payloads: vec![node].into(),
            }
        }
        None => {
            trace!(target: "rcdzc::fold", node = id.0, "Ast.decode folds non-canonical bytes to (Err …)");
            let unit = synth_core(db, Core::Unit, crate::ty::Ty::Unit);
            Core::SumNew {
                disc: disc_err,
                payloads: vec![unit].into(),
            }
        }
    }
}

/// Fold `Ast.module` to the `Ast` VALUE reflecting the module in `file_index` — the module's own source
/// tree (`self-hosting-surface.md` §A Program's Syntax Tree Is An Ordinary Value). `file_index` is the
/// DEFINING module of the `Ast.module` occurrence — the caller passes the file of the ACCESS SITE (the
/// `(. Ast module)` member-access node), NOT the shared, fileless built-in reflect op-record: the op-record
/// is the same node for every occurrence, so keying on it would lose which module wrote `Ast.module` and
/// reflect file 0 (the operator-confirmed use-site late-binding bug). Reads that file's source snapshot and
/// rebuilds its AST value via `arenas_to_ast_value`. Declines cleanly (never miscompiles) when the built-in
/// `Ast` sum is unavailable, the file has no snapshot, or the module has a leaf with no `Ast` variant.
pub(super) fn reflect_module_value(db: &mut Db, file_index: usize) -> Core {
    let Some(disc) = ast_variant_discs(db) else {
        return Core::Poison(Reject::decline(
            "Ast.module: the built-in Ast sum is unavailable",
        ));
    };
    let Some(snapshot) = db.source_snapshots.get(file_index).cloned().flatten() else {
        return Core::Poison(Reject::decline(
            "Ast.module: no source snapshot for the enclosing module",
        ));
    };
    let module_root = snapshot.root;
    match arenas_to_ast_value(db, &snapshot, module_root, &disc) {
        Some(root) => core_of(db, root),
        None => Core::Poison(Reject::decline(
            "Ast.module: the enclosing module has a node with no Ast variant",
        )),
    }
}

/// Fold `Type.ast-generic e` (and the non-generic case of `Type.ast e`) to the `Ast` VALUE of `e`'s type
/// DEFINITION — the verbatim `(type Name …)` declaration reflected via the ordinary `Ast.*` ctors
/// (`DESIGN-type-to-ast-reflection.md`; increment 1 = nominal/sum only). Reduces `e` to its type-VALUE
/// (`typeval_of`), recovers the declaring `TypeDecl` from the `Ty::Sum`/`Nominal` `decl` occurrence, and
/// reflects that decl's PRE-RESOLVE SOURCE node — from the defining file's snapshot, the SAME verbatim-source
/// path `Ast.module` uses, since the live arena is post-resolve — via `arenas_to_ast_value`. `instantiated`
/// (the short `Type.ast`) is the same verbatim decl for a NON-generic type; a GENERIC type's instantiated
/// substitution is increment 3, so it declines here. Declines cleanly (never miscompiles) when the argument
/// is not a concrete nominal/sum, the built-in `Ast` sum is unavailable, or no snapshot / decl node is found.
pub(super) fn lower_type_ast(db: &mut Db, arg: StructId, instantiated: bool) -> Core {
    let Some(ty) = crate::eval::typeval_of(db, arg) else {
        return Core::Poison(Reject::decline(
            "Type.ast requires a concrete type-value (a Type.of result or a written type)",
        ));
    };
    let (decl, ty_args) = match &ty {
        crate::ty::Ty::Sum { decl, args } | crate::ty::Ty::Nominal { decl, args, .. } => {
            (*decl, args.to_vec())
        }
        // Increment 2 — a STRUCTURAL type (a bare record/tuple/`List`/`Map`/`Set`/primitive/`Bytes`/
        // `Float`/`Qty`/`Type`) has no `TypeDecl` to reify, so reflect its CANONICAL type-surface AST via
        // `type_ast` (`lower.rs:1511`) — the same renderer the value-form + `encode_ty` use, so the
        // reflected surface agrees byte-for-byte. There are no type params to keep generic, so
        // `Type.ast-generic` and `Type.ast` COINCIDE for a structural type (both reach here). A
        // NON-CONCRETE type (one still carrying an unresolved `Ty::Var`) DECLINES with a diagnostic naming
        // the ambiguity — reflecting an undetermined type would fabricate a shape. `type_ast` yields `None`
        // for a `Fn`/`Cont` (no value-form surface); those decline here too (arrow-surface reflection is a
        // later refinement — the corpus TODO-pins the intended `(-> …)` form).
        _ => {
            if ty.has_free_var() {
                // A type variable has NO definition to reflect, so this is a genuine SEMANTIC reject (a
                // CODED CDZ0203), not a well-formed-but-unsupported decline (v-spec-oracle review + the
                // operator corpus policy: a decline is reserved for a construct the compiler does-not-yet
                // compile; an unresolved type var is permanently ill-formed here).
                return Core::Poison(
                    Reject::coded(
                        Code::TypeMismatch,
                        "Type.ast requires a concrete type; found an unresolved type variable (annotate the type)",
                    )
                    .at(arg),
                );
            }
            let Some(disc) = ast_variant_discs(db) else {
                return Core::Poison(Reject::decline(
                    "Type.ast: the built-in Ast sum is unavailable",
                ));
            };
            let mut b = crate::ast::Builder::new();
            // Scope the `NameCtx` borrow of `db` so the mutable `arenas_to_ast_value(db, …)` below is free.
            let surface = {
                let ncx = db.name_ctx();
                type_surface_ast(&mut b, &ty, &ncx)
            };
            let Some(node) = surface else {
                return Core::Poison(Reject::decline(
                    "Type.ast: this type has no canonical surface form to reflect (a continuation type)",
                ));
            };
            let arenas = b.finish(node);
            return match arenas_to_ast_value(db, &arenas, arenas.root, &disc) {
                Some(root) => core_of(db, root),
                None => Core::Poison(Reject::decline(
                    "Type.ast: the type-surface form has a node with no Ast variant",
                )),
            };
        }
    };
    let Some((name, params)) = db
        .type_decl_by_occ(decl)
        .map(|d| (d.name.clone(), d.params.clone()))
    else {
        return Core::Poison(Reject::decline(
            "Type.ast: no declaration found for the type",
        ));
    };
    let is_generic = !params.is_empty();
    let Some(disc) = ast_variant_discs(db) else {
        return Core::Poison(Reject::decline(
            "Type.ast: the built-in Ast sum is unavailable",
        ));
    };
    // The decl SOURCE to reflect. For a USER type it is the verbatim node in the defining file's
    // PRE-RESOLVE snapshot (the live arena is post-resolve). For a BUILT-IN / snapshot-less type (`Option`,
    // `Result`, … — the prelude synthesizes them, so there is no user source file) fall back to the
    // PRELUDE-synthesized `(type …)` node at `TypeDecl.occ` (`decl`) in the live arena, copied out into an
    // OWNED arena so the later `arenas_to_ast_value(db, …)` mutable borrow does not clash with the `&db.ast`
    // read (operator directive 2026-08-31: type reflection MUST handle built-in type definitions). Both
    // paths yield an owned `(Arenas, root-node)` the generic/instantiated logic below reflects uniformly.
    let file = db.file_of(decl).unwrap_or(0);
    let (snapshot, node) = if let Some(snap) = db.source_snapshots.get(file).cloned().flatten()
        && let Some(n) = find_type_decl_node(&snap, snap.root, &name)
    {
        (snap, n)
    } else {
        let mut b = crate::ast::Builder::new();
        let empty: std::collections::HashMap<String, StructId> = std::collections::HashMap::new();
        let copied = copy_subst_node(&db.ast, decl, &empty, &mut b);
        let arenas = b.finish(copied);
        (std::rc::Rc::new(arenas), copied)
    };

    // The INSTANTIATED form (`Type.ast`) of a GENERIC type (increment 3): substitute the decl's own type
    // params with the type-value's concrete args, then reflect. A free LOWERCASE name in a variant payload
    // IS a type parameter (a type ref is Capitalized — `TypeDecl.params` doc), so substituting a
    // param-named atom by its arg's canonical type-surface is UNAMBIGUOUS (no shadowing — a type decl
    // cannot rebind its own param), which is why an AST-level substitution here is capture-safe. The head
    // param BINDERS are DROPPED (design §7.1 default — the params are gone once concrete), so
    // `type Pair a b (Pair a b)` at `Pair Int Str` reflects `(type Pair (Pair Int Str))`. FINITENESS
    // (design §3.3): a nested named ref is copied structurally with its params substituted but NEVER
    // unfolded — `type List a = Nil | Cons a (List a)` at `List Int` reflects
    // `(type List (Nil) (Cons Int64 (List Int64)))`, the self-reference staying a named application.
    if instantiated && is_generic {
        // A NON-CONCRETE instantiation (an arg still carrying an unresolved `Ty::Var` — a polymorphic
        // value reflected before it is pinned) has no definite shape to substitute; decline naming the
        // ambiguity rather than fabricating one (design §3.4). (A CONCRETE generic — `Opt Int64`,
        // `Lst Int64` — proceeds; a concrete arg that is itself a `Fn` still declines below, as arrow
        // surfaces are a later increment.)
        if ty.has_free_var() {
            // An unresolved arg has no concrete definition to substitute — a CODED semantic reject
            // (CDZ0203), not a decline (see the structural arm above; v-spec-oracle review).
            return Core::Poison(
                Reject::coded(
                    Code::TypeMismatch,
                    "Type.ast requires a concrete type; a type argument is an unresolved type variable \
                     (annotate the type)",
                )
                .at(arg),
            );
        }
        // Render each concrete arg to its canonical type-surface node in a fresh builder; a param appearing
        // in a payload is replaced by the matching arg surface. An arg with no surface (a `Fn`) declines.
        let mut b = crate::ast::Builder::new();
        let mut param_surface: std::collections::HashMap<String, StructId> =
            std::collections::HashMap::new();
        {
            let ncx = db.name_ctx();
            for (p, arg) in params.iter().zip(ty_args.iter()) {
                match type_surface_ast(&mut b, arg, &ncx) {
                    Some(surf) => {
                        param_surface.insert(p.clone(), surf);
                    }
                    None => {
                        return Core::Poison(Reject::decline(
                            "Type.ast: a type argument has no canonical surface form to substitute (a \
                             continuation type)",
                        ));
                    }
                }
            }
        }
        // If some params had no matching arg (arity mismatch — should not happen for a solved concrete
        // type), fall back to the verbatim generic decl rather than emitting a partial substitution.
        if param_surface.len() != params.len() {
            return match arenas_to_ast_value(db, &snapshot, node, &disc) {
                Some(root) => core_of(db, root),
                None => Core::Poison(Reject::decline(
                    "Type.ast: the declaration has a node with no Ast variant",
                )),
            };
        }
        let params_set: std::collections::HashSet<&str> =
            params.iter().map(String::as_str).collect();
        let Some(instantiated_decl) =
            copy_decl_instantiated(&snapshot, node, &params_set, &param_surface, &mut b)
        else {
            return Core::Poison(Reject::decline(
                "Type.ast: the generic declaration was not a well-formed (type …) form",
            ));
        };
        let arenas = b.finish(instantiated_decl);
        return match arenas_to_ast_value(db, &arenas, arenas.root, &disc) {
            Some(root) => core_of(db, root),
            None => Core::Poison(Reject::decline(
                "Type.ast: the instantiated declaration has a node with no Ast variant",
            )),
        };
    }

    // Otherwise (`Type.ast-generic`, or `Type.ast` on a NON-generic type): reflect the verbatim decl.
    match arenas_to_ast_value(db, &snapshot, node, &disc) {
        Some(root) => core_of(db, root),
        None => Core::Poison(Reject::decline(
            "Type.ast: the declaration has a node with no Ast variant",
        )),
    }
}

/// Render a `Ty` to its canonical type-SURFACE AST node in `b` — the reflection surface for a structural
/// type / a type argument. A function type `Ty::Fn(p, r)` renders `(-> <surface p> <surface r>)` (matching
/// `Ty::render_name`'s arrow form; a curried multi-arg fn nests, `(-> p0 (-> p1 r))`); everything else
/// delegates to `super::type_ast` (the value-form surface renderer — scalars/collections/Sum/Nominal/…).
/// `None` for a type with no surface even here (a `Cont`, or a bare `Var`/`Any` — the latter are rejected
/// upstream by the concreteness check). This lets `Type.ast` reflect a function type's arrow surface, which
/// `type_ast` alone declines (a function is not a value-form).
fn type_surface_ast(
    b: &mut crate::ast::Builder,
    ty: &crate::ty::Ty,
    ncx: &crate::ty::NameCtx,
) -> Option<StructId> {
    if let crate::ty::Ty::Fn(p, r) = ty {
        let arrow = b.name("->");
        let ps = type_surface_ast(b, p, ncx)?;
        let rs = type_surface_ast(b, r, ncx)?;
        return Some(b.list(vec![arrow, ps, rs]));
    }
    super::type_ast(b, ty, ncx)
}

/// Copy a generic `(type Name param… variant…)` source declaration into `b` with its type params
/// SUBSTITUTED by concrete-arg surfaces — the instantiated form for `Type.ast` (increment 3). The head
/// param binders (bare atoms whose name is a param) are DROPPED; every other node is copied structurally,
/// and a bare atom whose name is a param is REPLACED by that param's arg-surface node (`param_surface`,
/// already built in `b`). A nested named ref (`(List a)`) is copied with its params substituted but is
/// NOT unfolded, keeping the result finite under recursive/self-referential generics. `None` if `node` is
/// not a `(type …)` form.
fn copy_decl_instantiated(
    src: &crate::ast::Arenas,
    node: StructId,
    params: &std::collections::HashSet<&str>,
    param_surface: &std::collections::HashMap<String, StructId>,
    b: &mut crate::ast::Builder,
) -> Option<StructId> {
    let crate::ast::Struct::List(children) = src.get(node) else {
        return None;
    };
    let children = children.clone();
    // Head: `type`, the Name, then param BINDERS (dropped) + variants (copied, params substituted).
    let mut out = Vec::with_capacity(children.len());
    for (i, &child) in children.iter().enumerate() {
        // A head-level bare atom whose name is a param is a BINDER → drop it (the params are now concrete).
        // Index 0/1 are the `type` head + the type name — never a binder — so only skip at index >= 2.
        if i >= 2
            && let crate::ast::Struct::Atom(l) = src.get(child)
            && let crate::ast::Leaf::Name(n) = src.leaf(*l)
            && params.contains(n.as_ref())
        {
            continue;
        }
        out.push(copy_subst_node(src, child, param_surface, b));
    }
    Some(b.list(out))
}

/// Recursively copy `node` from `src` into `b`, replacing any bare `Leaf::Name` atom that is a type
/// PARAM with its concrete arg-surface node (`param_surface`); every other node is copied verbatim. A
/// nested compound is copied structurally (its own param atoms substituted), so a named type reference is
/// preserved as a named application — never unfolded.
fn copy_subst_node(
    src: &crate::ast::Arenas,
    node: StructId,
    param_surface: &std::collections::HashMap<String, StructId>,
    b: &mut crate::ast::Builder,
) -> StructId {
    match src.get(node) {
        crate::ast::Struct::Atom(l) => {
            let leaf = src.leaf(*l).clone();
            if let crate::ast::Leaf::Name(n) = &leaf
                && let Some(&surf) = param_surface.get(n.as_ref())
            {
                return surf;
            }
            b.atom_leaf(leaf)
        }
        crate::ast::Struct::List(kids) => {
            let kids = kids.clone();
            let copied: Vec<StructId> = kids
                .iter()
                .map(|&c| copy_subst_node(src, c, param_surface, b))
                .collect();
            b.list(copied)
        }
    }
}

/// Recursively find the `(type NAME …)` declaration form for `name` under `sid` in a PRE-RESOLVE source
/// arena — the verbatim node `Type.ast-generic` reflects. First-match by name within the declaring file
/// (increment 1; identity-by-occurrence for same-named decls is a later increment).
fn find_type_decl_node(arenas: &crate::ast::Arenas, sid: StructId, name: &str) -> Option<StructId> {
    if let Some(elems) = arenas.as_form(sid, "type")
        && let Some(&first) = elems.first()
        && arenas.as_name(first) == Some(name)
    {
        return Some(sid);
    }
    if let crate::ast::Struct::List(children) = arenas.get(sid) {
        for c in children.clone() {
            if let Some(found) = find_type_decl_node(arenas, c, name) {
                return Some(found);
            }
        }
    }
    None
}

/// Rebuild an `Ast` sum VALUE (`Core::SumNew`) from a node of a `codec::decode`d cadenza-ast `Arenas` — the
/// inverse of `encode_ast_value`. Walks the `Struct`/`Leaf` at `sid` (an arena-local id) and maps each
/// cadenza-ast kind to the matching `Ast` variant, synthesizing the payload + `SumNew` in `db`'s arena.
/// `None` if a node is NOT a shape an `Ast` value covers (a Char/Sym/… leaf) — so `Ast.decode` yields the
/// `Err` arm (TOTAL, never a trap). Operator ruling OPTION A: one canonical codec, no bespoke format.
pub(super) fn arenas_to_ast_value(
    db: &mut Db,
    arenas: &crate::ast::Arenas,
    sid: StructId,
    disc: &AstDiscs,
) -> Option<StructId> {
    match arenas.get(sid) {
        // A `Struct::List`. A native-compound-data HEAD (Option B) reflects to its DISTINCT Ast variant;
        // any other list is the generic `Ast.List` of the rebuilt children.
        crate::ast::Struct::List(children) => {
            let children = children.clone();
            // Native ctor-head → Ast.{List,Tuple,Record,Map,Set}Ctor of the reflected TAIL (a (List Ast)).
            if let Some(ctor) = arenas.compound_ctor_leaf(sid) {
                let disc_ctor = match ctor {
                    crate::ast::CompoundCtor::List => disc.list_ctor,
                    crate::ast::CompoundCtor::Tuple => disc.tuple_ctor,
                    crate::ast::CompoundCtor::Record => disc.record_ctor,
                    crate::ast::CompoundCtor::Map => disc.map_ctor,
                    crate::ast::CompoundCtor::Set => disc.set_ctor,
                };
                let mut elems = Vec::with_capacity(children.len().saturating_sub(1));
                for &c in &children[1..] {
                    elems.push(arenas_to_ast_value(db, arenas, c, disc)?);
                }
                let payload = synth_core(
                    db,
                    Core::ListNew {
                        elems: elems.into(),
                    },
                    crate::ty::Ty::List(Box::new(disc.ty.clone())),
                );
                return Some(synth_core(
                    db,
                    Core::SumNew {
                        disc: disc_ctor,
                        payloads: vec![payload].into(),
                    },
                    disc.ty.clone(),
                ));
            }
            // Native FIELD_PAIR (`(= k v)`) / MEMBER (`(. obj key)`) head → Ast.FieldPair / Ast.Member of a
            // `(Tuple Ast Ast)` of the two reflected children.
            let pair = arenas
                .field_pair_parts(sid)
                .map(|kv| (kv, disc.field_pair))
                .or_else(|| arenas.member_parts(sid).map(|ok| (ok, disc.member)));
            if let Some(((x, y), disc_pair)) = pair {
                let ax = arenas_to_ast_value(db, arenas, x, disc)?;
                let ay = arenas_to_ast_value(db, arenas, y, disc)?;
                let payload = synth_core(
                    db,
                    Core::Tuple {
                        elems: vec![ax, ay].into(),
                    },
                    crate::ty::Ty::Tuple(vec![disc.ty.clone(), disc.ty.clone()].into()),
                );
                return Some(synth_core(
                    db,
                    Core::SumNew {
                        disc: disc_pair,
                        payloads: vec![payload].into(),
                    },
                    disc.ty.clone(),
                ));
            }
            // Native RATIONAL (`(RationalTag <num> <den>)`) head → Ast.Rational of a `(Tuple Ast Ast)` of the
            // two reflected Int children — the decode twin of the `Builder::rational` encode above.
            if let Some((num, den)) = arenas.rational_parts(sid) {
                let an = arenas_to_ast_value(db, arenas, num, disc)?;
                let ad = arenas_to_ast_value(db, arenas, den, disc)?;
                let payload = synth_core(
                    db,
                    Core::Tuple {
                        elems: vec![an, ad].into(),
                    },
                    crate::ty::Ty::Tuple(vec![disc.ty.clone(), disc.ty.clone()].into()),
                );
                return Some(synth_core(
                    db,
                    Core::SumNew {
                        disc: disc.rational,
                        payloads: vec![payload].into(),
                    },
                    disc.ty.clone(),
                ));
            }
            let mut elems = Vec::with_capacity(children.len());
            for c in children {
                elems.push(arenas_to_ast_value(db, arenas, c, disc)?);
            }
            let payload = synth_core(
                db,
                Core::ListNew {
                    elems: elems.into(),
                },
                crate::ty::Ty::List(Box::new(disc.ty.clone())),
            );
            Some(synth_core(
                db,
                Core::SumNew {
                    disc: disc.list,
                    payloads: vec![payload].into(),
                },
                disc.ty.clone(),
            ))
        }
        crate::ast::Struct::Atom(leaf_id) => {
            // Clone the leaf out so no borrow of `arenas` is held across the `&mut db` synth calls below.
            let leaf = arenas.leaves.get(leaf_id.0 as usize)?.clone();
            match leaf {
                crate::ast::Leaf::Int { value, .. } => {
                    // `Ast.Int`'s payload is `BigInt` (a quoted AST stores integers non-lossily), so the
                    // decoded integer node is typed `BigInt` — matching the sum's declared payload and the
                    // arbitrary-precision `IntValue` (which can exceed i64). Mirrors the encode `Leaf::Int`.
                    let payload = db.push_atom(crate::ast::Leaf::Int {
                        value: value.clone(),
                        radix: crate::ast::Radix::Dec,
                    });
                    db.core.fill(payload, Core::ConstInt(value));
                    db.types.fill(payload, crate::ty::Ty::BigInt);
                    Some(synth_core(
                        db,
                        Core::SumNew {
                            disc: disc.int,
                            payloads: vec![payload].into(),
                        },
                        disc.ty.clone(),
                    ))
                }
                crate::ast::Leaf::Float(dec) => {
                    let payload = synth_core(db, Core::ConstFloat(dec), crate::ty::Ty::float64());
                    Some(synth_core(
                        db,
                        Core::SumNew {
                            disc: disc.float,
                            payloads: vec![payload].into(),
                        },
                        disc.ty.clone(),
                    ))
                }
                crate::ast::Leaf::Bool(x) => {
                    let payload = synth_core(db, Core::ConstBool(x), crate::ty::Ty::Bool);
                    Some(synth_core(
                        db,
                        Core::SumNew {
                            disc: disc.bool,
                            payloads: vec![payload].into(),
                        },
                        disc.ty.clone(),
                    ))
                }
                crate::ast::Leaf::Str(s) => {
                    let payload = synth_core(db, Core::ConstStr(s), crate::ty::Ty::String);
                    Some(synth_core(
                        db,
                        Core::SumNew {
                            disc: disc.str,
                            payloads: vec![payload].into(),
                        },
                        disc.ty.clone(),
                    ))
                }
                crate::ast::Leaf::Name(n) => {
                    let payload = synth_core(db, Core::ConstStr(n), crate::ty::Ty::String);
                    Some(synth_core(
                        db,
                        Core::SumNew {
                            disc: disc.name,
                            payloads: vec![payload].into(),
                        },
                        disc.ty.clone(),
                    ))
                }
                crate::ast::Leaf::Bytes(bytes) => {
                    let elems = bytes_to_elems(db, &bytes);
                    let payload = synth_core(
                        db,
                        Core::BytesOf {
                            elems: elems.into(),
                        },
                        crate::ty::Ty::Bytes,
                    );
                    Some(synth_core(
                        db,
                        Core::SumNew {
                            disc: disc.bytes,
                            payloads: vec![payload].into(),
                        },
                        disc.ty.clone(),
                    ))
                }
                // A char leaf → `(Ast.Char #\c)`: a `Core::ConstChar` payload at `Ty::Char`.
                crate::ast::Leaf::Char(c) => {
                    let payload = synth_core(db, Core::ConstChar(c), crate::ty::Ty::Char);
                    Some(synth_core(
                        db,
                        Core::SumNew {
                            disc: disc.char,
                            payloads: vec![payload].into(),
                        },
                        disc.ty.clone(),
                    ))
                }
                // A symbol leaf → `(Ast.Symbol #"s")`: a constant symbol shares the `Core::ConstStr` rep
                // at `Ty::Symbol`.
                crate::ast::Leaf::Sym(s) => {
                    let payload = synth_core(db, Core::ConstStr(s), crate::ty::Ty::Symbol);
                    Some(synth_core(
                        db,
                        Core::SumNew {
                            disc: disc.symbol,
                            payloads: vec![payload].into(),
                        },
                        disc.ty.clone(),
                    ))
                }
                // BadChar / BadEscape (reader error markers) / a suffixed leaf — NOT `Ast` value variants
                // (they arise only from malformed source) → `Err`.
                _ => None,
            }
        }
    }
}
