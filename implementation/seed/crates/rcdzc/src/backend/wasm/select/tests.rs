use super::*;
use crate::testkit::scalar_program;

/// Compute the boundary layout for a test program (all test fixtures have an export). `select_*`
/// needs it to resolve a `Core::Call` callee's function index; these Lir-level tests exercise no
/// call, so the layout's contents beyond the exported def are irrelevant — but a real one is passed
/// so the signature is honest.
fn layout_of(db: &mut Db) -> Layout {
    crate::layout::compute(db).expect("layout")
}

// The general result-lift's canonical-ABI layout helper: pins the component-model `(size, align)` the
// guest reallocs the spilled-result return area to and reads it back at — including `list<list<u8>>`
// (graph.neighbors), whose element STRIDE is `canonical_layout(list<u8>) = 8`. A regression here would
// silently mis-size the retptr or mis-stride a list element → a garbage lift. The Sum/option case is
// proven end-to-end by the wasmtime kv.get invoke test; these are the db-free structural cases.
#[test]
fn canonical_layout_pins_the_spilled_result_shapes() {
    use crate::ty::{IntTy, Ty};
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (main) 0) (export main))",
    ));
    let l = |db: &mut Db, t: &Ty| canonical_layout(db, t);
    // Scalars at their component width.
    assert_eq!(l(&mut db, &Ty::Bool), (1, 1));
    assert_eq!(l(&mut db, &Ty::Int(IntTy::fixed(true, 32))), (4, 4));
    assert_eq!(l(&mut db, &Ty::Int(IntTy::fixed(false, 64))), (8, 8));
    // list<u8> (Bytes), string, and any list<T> cross as an 8-byte (ptr, len/count) header, align 4.
    assert_eq!(l(&mut db, &Ty::Bytes), (8, 4));
    assert_eq!(l(&mut db, &Ty::List(Box::new(Ty::Bytes))), (8, 4)); // list<list<u8>> (graph.neighbors)
    // The graph.neighbors element stride is the layout of `list<u8>` = 8.
    assert_eq!(l(&mut db, &Ty::Bytes).0, 8);
    // A tuple<list<u8>, list<u8>> (the kv.prefix-scan element) = two 8-byte headers = 16, align 4.
    assert_eq!(
        l(&mut db, &Ty::Tuple(vec![Ty::Bytes, Ty::Bytes].into())),
        (16, 4)
    );
    // Alignment padding: tuple<bool, u64> = bool@0 (1), pad to 8, u64@8 (8) → size 16, align 8.
    assert_eq!(
        l(
            &mut db,
            &Ty::Tuple(vec![Ty::Bool, Ty::Int(IntTy::fixed(false, 64))].into())
        ),
        (16, 8)
    );
    // A nested list<tuple<list<u8>,list<u8>>> (prefix-scan result) is still an 8-byte header.
    assert_eq!(
        l(
            &mut db,
            &Ty::List(Box::new(Ty::Tuple(vec![Ty::Bytes, Ty::Bytes].into())))
        ),
        (8, 4)
    );
}

// WIT-ABI completion (result side): the option result LIFT is GENERAL over its payload — `option<T>`,
// not pinned to `option<list<u8>>`. Pins the pure boundary-classification helpers (no reducer run): the
// payload is read as `T`, the type admits, and it maps to the WIT type `option<wit(T)>`. A pure-fn unit
// test (per the operator's Rust-is-unit-tests-only directive); the emit/invoke proof lives in the corpus
// WIT-integration harness (a dedicated owner is building it).
#[test]
fn option_result_lift_is_general_over_the_payload() {
    use crate::backend::wasm::host;
    use crate::wit_world::WitType;
    // option<list<list<u8>>> — an option payload BEYOND Bytes (`(List Bytes)` = list<list<u8>>).
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (f (: x (Option (List Bytes)))) x) (def (main) 0) (export main))",
    ));
    let (params, _) = function_of(&mut db, "f");
    let opt_ty = params[0].1.clone();
    assert_eq!(
        host::option_payload_ty(&mut db, &opt_ty),
        Some(Ty::List(Box::new(Ty::Bytes))),
        "the payload of option<list<list<u8>>> is list<list<u8>>"
    );
    assert!(
        host::result_is_liftable(&mut db, &opt_ty),
        "option<list<list<u8>>> is a liftable spilled result (general over the payload, not just Bytes)"
    );
    assert_eq!(
        host::spilled_result_wit_type(&mut db, &opt_ty),
        Some(WitType::Option(Box::new(WitType::List(Box::new(
            WitType::List(Box::new(WitType::U8))
        ))))),
        "option<list<list<u8>>> maps to the WIT type option<list<list<u8>>>"
    );
    // Regression: option<list<u8>> (the kv.get shape) still classifies as liftable.
    let mut db2 = Db::load(crate::testkit::parse(
        "(module m (def (g (: y (Option Bytes))) y) (def (main) 0) (export main))",
    ));
    let (p2, _) = function_of(&mut db2, "g");
    assert!(
        host::result_is_liftable(&mut db2, &p2[0].1),
        "option<list<u8>> is still liftable"
    );
}

// WIT-ABI completion (result side): a SCALAR LEAF of a spilled compound result now lifts (loaded
// width-correct + boxed), so a tuple/list/option OF scalars is a liftable result — while a bare scalar
// stays NON-spilled (it crosses by value, not via the lift). Pure-fn classification test.
#[test]
fn scalar_leaves_lift_inside_a_compound_result_but_a_bare_scalar_does_not() {
    use crate::backend::wasm::host;
    // tuple<u64, list<u8>> — a scalar leaf + a bytes leaf: a liftable spilled result.
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (f (: x (Tuple UInt64 Bytes))) x) (def (main) 0) (export main))",
    ));
    let (params, _) = function_of(&mut db, "f");
    assert!(
        host::result_is_liftable(&mut db, &params[0].1),
        "tuple<u64, list<u8>> lifts — a scalar leaf is boxed inside the aggregate"
    );
    // list<s32> — a list of a NARROW int leaf lifts.
    let mut db2 = Db::load(crate::testkit::parse(
        "(module m (def (g (: y (List Int32))) y) (def (main) 0) (export main))",
    ));
    let (p2, _) = function_of(&mut db2, "g");
    assert!(
        host::result_is_liftable(&mut db2, &p2[0].1),
        "list<s32> lifts — a narrow-int element is boxed"
    );
    // A BARE scalar result is NOT a spilled compound (crosses by value) — must NOT classify as liftable.
    let mut db3 = Db::load(crate::testkit::parse(
        "(module m (def (h (: z UInt64)) z) (def (main) 0) (export main))",
    ));
    let (p3, _) = function_of(&mut db3, "h");
    assert!(
        !host::result_is_liftable(&mut db3, &p3[0].1),
        "a bare u64 result is not spilled — it crosses by value, not via the lift"
    );
}

// WIT-ABI completion (result side): a bare `string` host-import RESULT now lifts — the result-side twin of
// the string ARG (which already crosses on every path). A `string` result crosses on the world-driven
// boundary as the SAME `(ptr,len)` spill a `list<u8>` (Bytes) result rides (a guest `String` is a byte-rope
// handle, the same value-heap representation the Bytes lift produces), so `result_is_liftable` admits it via
// its shared `Ty::Bytes | Ty::String` leaf arm and `spilled_result_wit_type` maps it to `WitType::String`.
// Pure-fn classification test (per the Rust-is-unit-tests-only directive); the emit+run proof lives in the
// corpus WIT-integration harness (28-wit-abi-boundary SHAPE 57).
#[test]
fn a_string_host_result_lifts_and_maps_to_wit_string() {
    use crate::backend::wasm::host;
    use crate::wit_world::WitType;
    // A bare `string` result — the same spilled `(ptr,len)` shape as `list<u8>`, so it lifts.
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (f (: x String)) x) (def (main) 0) (export main))",
    ));
    let (params, _) = function_of(&mut db, "f");
    assert!(
        host::result_is_liftable(&mut db, &params[0].1),
        "a bare string result lifts — the (ptr,len) spill twin of a list<u8> (Bytes) result"
    );
    assert_eq!(
        host::spilled_result_wit_type(&mut db, &params[0].1),
        Some(WitType::String),
        "a string result maps to the WIT type string"
    );
    // Regression: a `list<u8>` (Bytes) result still lifts (the arm it now shares).
    let mut db2 = Db::load(crate::testkit::parse(
        "(module m (def (g (: y Bytes)) y) (def (main) 0) (export main))",
    ));
    let (p2, _) = function_of(&mut db2, "g");
    assert!(
        host::result_is_liftable(&mut db2, &p2[0].1),
        "a list<u8> (Bytes) result still lifts"
    );
}

// WIT-ABI completion (arg side): a record host-arg FIELD is now any aliased SCALAR width, not just
// 64-bit ints — narrow ints (s8..s32/u8..u32), char, and narrow floats cross via `field_boundary_abi` →
// `abi_val_type` (general over width), read back with `get-int` + an i64→i32 narrow. Pure-fn unit test
// (per the Rust-is-unit-tests-only directive) pinning the classification; the emit proof lives in the
// corpus WIT-integration harness.
#[test]
fn a_record_host_arg_admits_narrow_scalar_fields() {
    use crate::backend::wasm::host;
    // A record with a narrow int (Int32), a char, a bool, and a narrow float — all now boundary-crossable.
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (f (: r (Record (a Int32) (b Char) (c Bool) (d Float32)))) 0) \
             (def (main) 0) (export main))",
    ));
    let (params, _) = function_of(&mut db, "f");
    assert!(
        host::is_boundary_record(&mut db, &params[0].1),
        "a record with narrow-int / char / bool / narrow-float fields crosses as a boundary record \
             (was declined when only 64-bit int/bool/float fields were admitted)"
    );
    // Regression: an all-64-bit record still crosses.
    let mut db2 = Db::load(crate::testkit::parse(
        "(module m (def (g (: r (Record (a Int64) (b Bool)))) 0) (def (main) 0) (export main))",
    ));
    let (p2, _) = function_of(&mut db2, "g");
    assert!(
        host::is_boundary_record(&mut db2, &p2[0].1),
        "64-bit-field record still crosses"
    );
}

// The deliver-message field-order fix: a record host-arg's fields must emit in the host WIT record's
// DECLARATION order, not the guest's name-lex order — else the component-linker's structural match fails
// and the guest silently fails to instantiate (deliver-message/response non-routing). Pure-fn test of the
// reorder over the message shape: name-lex `contract, payload, sender{host, reducer}, token` →
// WIT-declaration `contract, sender{reducer, host}, payload, token` (top-level sender/payload swap + the
// nested reducer/host swap, both fixed). The runtime link is re-verified by v-platform-itest's drive.
#[test]
fn record_host_arg_fields_reorder_to_wit_declaration_order() {
    use crate::backend::wasm::host::{RecordFieldAbi, reorder_record_fields_to_wit};
    use crate::wit_world::WitType;
    let lu8 = || WitType::List(Box::new(WitType::U8));
    // Guest name-lex order (BTreeMap): contract, payload, sender{host, reducer}, token.
    let name_lex = vec![
        ("contract".to_string(), RecordFieldAbi::Bytes),
        ("payload".to_string(), RecordFieldAbi::Bytes),
        (
            "sender".to_string(),
            RecordFieldAbi::Record(vec![
                ("host".to_string(), RecordFieldAbi::Bytes),
                ("reducer".to_string(), RecordFieldAbi::Bytes),
            ]),
        ),
        ("token".to_string(), RecordFieldAbi::Bytes),
    ];
    // Host WIT declaration order: contract, sender{reducer, host}, payload, token.
    let wit = WitType::Record(vec![
        ("contract".to_string(), lu8()),
        (
            "sender".to_string(),
            WitType::Record(vec![
                ("reducer".to_string(), lu8()),
                ("host".to_string(), lu8()),
            ]),
        ),
        ("payload".to_string(), lu8()),
        ("token".to_string(), lu8()),
    ]);
    let out = reorder_record_fields_to_wit(name_lex, &wit);
    assert_eq!(
        out.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["contract", "sender", "payload", "token"],
        "top-level fields reorder to the WIT declaration order (sender before payload)"
    );
    let RecordFieldAbi::Record(sub) = &out[1].1 else {
        panic!("sender is a nested record");
    };
    assert_eq!(
        sub.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["reducer", "host"],
        "the nested sender record reorders to the WIT order (reducer before host)"
    );
}

// The deliver-RESPONSE fix (fix-forward of #3223): a `result<list<u8>, err>` host-arg field must emit its
// err arm with the SAME component-type CONSTRUCTOR the host WIT declares — a payload-less `variant` when
// the WIT says `variant` (the platform `variant error`), an `enum` when it says `enum`. A `result<_,
// variant>` and a `result<_, enum>` are DISTINCT component types, so a guest whose err arm mismatched the
// host's constructor SILENTLY failed to instantiate (deliver-response's `answer: result<list<u8>, error>`).
// The reorder stamps `err_is_variant` from the WIT (the guest side, a payload-less `Sum`, cannot tell).
#[test]
fn result_host_arg_err_arm_follows_the_wit_variant_or_enum_constructor() {
    use crate::backend::wasm::host::{RecordFieldAbi, reorder_record_fields_to_wit};
    use crate::wit_world::WitType;
    let lu8 = || WitType::List(Box::new(WitType::U8));
    let cases = || {
        vec![
            "timeout".to_string(),
            "missing-handler".to_string(),
            "schema-violation".to_string(),
            "faulted".to_string(),
        ]
    };
    // Name-lex guest order {answer, contract, token}; answer is a result-field (err defaults to enum).
    let name_lex = || {
        vec![
            (
                "answer".to_string(),
                RecordFieldAbi::Result {
                    err_cases: cases(),
                    err_is_variant: false,
                },
            ),
            ("contract".to_string(), RecordFieldAbi::Bytes),
            ("token".to_string(), RecordFieldAbi::Bytes),
        ]
    };
    // Host WIT `response { contract, token, answer: result<list<u8>, VARIANT error> }` — the platform shape.
    let wit_variant = WitType::Record(vec![
        ("contract".to_string(), lu8()),
        ("token".to_string(), lu8()),
        (
            "answer".to_string(),
            WitType::Result {
                ok: Some(Box::new(lu8())),
                err: Some(Box::new(WitType::Variant(
                    cases().into_iter().map(|c| (c, None)).collect(),
                ))),
            },
        ),
    ]);
    let out = reorder_record_fields_to_wit(name_lex(), &wit_variant);
    assert_eq!(
        out.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["contract", "token", "answer"],
        "the response record reorders to the WIT declaration order (answer last)"
    );
    let RecordFieldAbi::Result {
        err_is_variant,
        err_cases,
    } = &out[2].1
    else {
        panic!("answer is a result field");
    };
    assert!(
        *err_is_variant,
        "the err arm follows the WIT `variant` constructor, so it emits a component variant not an enum"
    );
    assert_eq!(
        err_cases,
        &cases(),
        "the err case names are preserved in order"
    );
    // A host WIT that declares the err arm as `enum` keeps the enum constructor (no false promotion).
    let wit_enum = WitType::Record(vec![
        ("contract".to_string(), lu8()),
        ("token".to_string(), lu8()),
        (
            "answer".to_string(),
            WitType::Result {
                ok: Some(Box::new(lu8())),
                err: Some(Box::new(WitType::Enum(cases()))),
            },
        ),
    ]);
    let out = reorder_record_fields_to_wit(name_lex(), &wit_enum);
    let RecordFieldAbi::Result { err_is_variant, .. } = &out[2].1 else {
        panic!("answer is a result field");
    };
    assert!(
        !*err_is_variant,
        "a WIT `enum` err arm stays an enum (the constructor follows the WIT, not a default)"
    );
}

#[test]
fn selects_a_literal_to_i64_const() {
    let (ast, body) = scalar_program();
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let f = select_body(&mut db, body, &layout).expect("select");
    assert_eq!(f.code, vec![Lir::ConstI64(42)]);
    assert!(f.ret.agrees_with(&Ty::int64()));
}

#[test]
fn selects_a_runtime_if_with_leaf_branches_to_a_branchless_select() {
    // A RUNTIME condition (a bool param `p`) with two CHEAP TRAP-FREE LEAF branches (constants) —
    // the `if` selects to wasm's BRANCHLESS `select`, not a structured block: push the two branch
    // values then the condition, then `select` (which pops `[then, else, cond]` and pushes `then`
    // if `cond` is nonzero). `local.get 0` is the condition `p`. This replaces the old
    // `if (result i64) … else … end` control block — one instruction, no branch. (A CONSTANT
    // condition folds away in `lower`; a NON-leaf/heap/effecting branch keeps the structured `if`,
    // covered by `keeps_the_structured_if_when_a_branch_is_not_a_cheap_leaf`.)
    let ast = crate::testkit::parse(
        "(module m (def (f (: p Bool)) (if p 1 2)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![
            Lir::ConstI64(1),
            Lir::ConstI64(2),
            Lir::LocalGet(0),
            Lir::Select,
        ]
    );
}

#[test]
fn an_if_between_two_enum_disc_variants_selects_branchlessly() {
    // `(if c (Dir.North) (Dir.South))` — the result type `Dir` is an ENUM-DISC sum (all variants
    // nullary), so its runtime rep is a plain i32 DISCRIMINANT and each variant emits as just its
    // discriminant constant (no `sum-new`, no allocation, no drop). So this is `(if c 0 1)` on the
    // disc, and it selects BRANCHLESSLY — `i32.const 0 ; i32.const 1 ; local.get 0 ; select` — even
    // though the result is nominally a "heap type" (the `is_heap_type` gate is relaxed for enum-disc).
    let ast = crate::testkit::parse(
        "(module m (type Dir (North) (South) (East) (West)) \
               (def (f (: c Bool)) (if c (Dir.North) (Dir.South))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![
            Lir::ConstI32(0), // Dir.North's discriminant
            Lir::ConstI32(1), // Dir.South's discriminant
            Lir::LocalGet(0), // the condition c
            Lir::Select,
        ],
        "an if between two enum-disc variants is a branchless select on the discriminant; got {:?}",
        f.code
    );
}

#[test]
fn a_negated_if_condition_swaps_branches_and_drops_the_eqz() {
    // `(if (not c) a b)` ≡ `(if c b a)`: the negation is absorbed by swapping the branches — no
    // `i32.eqz`. It then selects branchlessly (leaf branches): `b ; a ; c ; select`, where the
    // branch operands are swapped (else-then `a`, then-first `b`) vs the un-negated `(if c a b)`.
    let ast = crate::testkit::parse(
        "(module m (def (f (: c Bool) (: a Int64) (: b Int64)) (if (not c) a b)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![
            Lir::LocalGet(2), // b (the else branch, now first — swapped)
            Lir::LocalGet(1), // a (the then branch)
            Lir::LocalGet(0), // c (the un-negated condition)
            Lir::Select,
        ],
        "the negation is absorbed by the branch swap — no i32.eqz"
    );
    assert!(
        !f.code.contains(&Lir::I32Eqz),
        "the `not` must be gone (swapped into the branch order), got: {:?}",
        f.code
    );
    // A double negation `(if (not (not c)) a b)` cancels back to the un-swapped order `a ; b ; c`.
    let ast2 = crate::testkit::parse(
        "(module m (def (f (: c Bool) (: a Int64) (: b Int64)) (if (not (not c)) a b)) (def (main) 0) (export main))",
    );
    let mut db2 = Db::load(ast2);
    let layout2 = layout_of(&mut db2);
    let (params2, body2) = function_of(&mut db2, "f");
    let f2 = select_function(&mut db2, body2, &params2, &layout2).expect("select");
    assert_eq!(
        f2.code,
        vec![
            Lir::LocalGet(1),
            Lir::LocalGet(2),
            Lir::LocalGet(0),
            Lir::Select
        ],
        "double negation cancels — branches back in original order, no eqz"
    );
}

#[test]
fn keeps_the_structured_if_when_a_branch_is_not_a_cheap_leaf() {
    // A branch that is NOT a cheap trap-free leaf (here `(+ a a)`, a checked add) must keep the
    // structured `if`/`else`/`end`: `select` evaluates BOTH branches unconditionally, so converting
    // a heavier/possibly-trapping branch would waste the work the `if` avoids (and could surface a
    // trap on the untaken side). So the wasm block survives with a real `if`. This pins the
    // eligibility gate `is_select_arm` alongside the positive case above.
    let ast = crate::testkit::parse(
        "(module m (def (f (: p Bool) (: a Int64)) (if p a (+ a a))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.contains(&Lir::If(BlockType::Val(ValType::I64))),
        "a non-leaf branch keeps the structured if, got: {:?}",
        f.code
    );
    assert!(
        !f.code.contains(&Lir::Select),
        "a non-leaf branch must NOT use select, got: {:?}",
        f.code
    );
}

#[test]
fn selects_a_runtime_if_with_small_trap_free_arms_to_a_branchless_select() {
    // A runtime `if` whose arms are NOT leaves but ARE small TRAP-FREE scalar ops — here `(& x 7)`
    // and `(| x 8)`, each a total bitwise op — converts to a branchless `select` (the widened
    // `is_select_arm` gate). Both arms + the condition are pushed, then `select`; no `if`/`else`/`end`
    // block. Sound because a bitwise op can neither trap nor allocate when evaluated on the untaken
    // path. Emitted arms: `(& x 7)` = get x ; const 7 ; and ; `(| x 8)` = get x ; const 8 ; or.
    let ast = crate::testkit::parse(
        "(module m (def (f (: x Int64)) (if (< x 0) (& x 7) (| x 8))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.contains(&Lir::Select),
        "small trap-free bitwise arms convert to a branchless select, got: {:?}",
        f.code
    );
    assert!(
        !f.code
            .iter()
            .any(|i| matches!(i, Lir::If(_) | Lir::Else | Lir::End)),
        "the structured if/else block is gone (branchless), got: {:?}",
        f.code
    );
    // The bitwise ops themselves are present (both arms evaluated, then select picks).
    assert!(
        f.code.contains(&Lir::I64And) && f.code.contains(&Lir::I64Or),
        "both trap-free arms are emitted before the select, got: {:?}",
        f.code
    );
}

#[test]
fn keeps_the_structured_if_when_a_trap_free_arm_exceeds_the_size_bound() {
    // A TRAP-FREE arm that is TOO BIG (`> SELECT_ARM_MAX_SIZE` nodes) keeps the structured `if`: a
    // `select` would compute the whole heavy arm on the untaken path, wasting more than the branch it
    // removes. Here the then-arm `(& (| (& (>> x 1) 3) 4) 7)` is a 4-deep bitwise nest (>5 nodes) —
    // trap-free but over the ceiling — so the branch survives. Pins the cost bound, not just the
    // trap-freedom gate.
    let ast = crate::testkit::parse(
        "(module m (def (f (: x Int64)) (if (< x 0) (& (| (& (>> x 1) 3) 4) 7) x)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.contains(&Lir::If(BlockType::Val(ValType::I64))),
        "an over-size trap-free arm keeps the structured if, got: {:?}",
        f.code
    );
    assert!(
        !f.code.contains(&Lir::Select),
        "an over-size trap-free arm must NOT use select, got: {:?}",
        f.code
    );
}

#[test]
fn a_nested_conditional_folds_to_nested_branchless_selects() {
    // The sign/clamp/3-way idiom `(if (< x 0) -1 (if (> x 0) 1 0))` — an `if` whose else arm is
    // itself a small conditional over trap-free (compare + constant) parts — folds to fully BRANCHLESS
    // code: no `if`/`else`/`end` block anywhere. The inner `(if (> x 0) 1 0)` is a bool materialization
    // (`x>0` extended) and the outer selects between `-1` and that. Sound: every condition is trap-free
    // (safe to evaluate unconditionally) and every arm is a constant, so evaluating both discards no
    // owned cell and runs no effect. Pins the nested-conditional widening of `is_select_arm`.
    let ast = crate::testkit::parse(
        "(module m (def (f (: x Int64)) (if (< x 0) -1 (if (> x 0) 1 0))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        !f.code
            .iter()
            .any(|i| matches!(i, Lir::If(_) | Lir::Else | Lir::End)),
        "a nested conditional over trap-free parts is fully branchless, got: {:?}",
        f.code
    );
    assert!(
        f.code.contains(&Lir::Select),
        "the nested conditional uses select, got: {:?}",
        f.code
    );
    // A genuine 3-way nested select `(if (= x 0) 0 (if (< x 0) -1 1))` nests TWO selects (the inner
    // picks -1/1, the outer picks 0/inner) — still no branch.
    let ast2 = crate::testkit::parse(
        "(module m (def (f (: x Int64)) (if (= x 0) 0 (if (< x 0) -1 1))) (def (main) 0) (export main))",
    );
    let mut db2 = Db::load(ast2);
    let layout2 = layout_of(&mut db2);
    let (params2, body2) = function_of(&mut db2, "f");
    let f2 = select_function(&mut db2, body2, &params2, &layout2).expect("select");
    assert_eq!(
        f2.code.iter().filter(|i| matches!(i, Lir::Select)).count(),
        2,
        "a 3-way nested conditional nests two selects, got: {:?}",
        f2.code
    );
    assert!(
        !f2.code
            .iter()
            .any(|i| matches!(i, Lir::If(_) | Lir::Else | Lir::End)),
        "the 3-way nested conditional is fully branchless, got: {:?}",
        f2.code
    );
}

#[test]
fn a_nested_conditional_with_a_trapping_inner_arm_keeps_the_branch() {
    // A nested conditional whose inner arm is NOT trap-free — here `(* x 1000000000000)`, a checked
    // multiply that overflows for a large `x` — must keep the structured `if` and NOT become a nested
    // `select` (which would evaluate the would-overflow arm unconditionally, surfacing a trap on the
    // untaken path). The branch survives; the mul keeps its overflow guard. Pins the trap-freedom gate
    // on the nested-conditional recursion (`select_arm_convertible` descends into the inner arm).
    let ast = crate::testkit::parse(
        "(module m (def (f (: x Int64)) (if (< x 0) 0 (if (> x 100) (* x 1000000000000) x))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.contains(&Lir::If(BlockType::Val(ValType::I64))),
        "a nested conditional with a trapping inner arm keeps the structured if, got: {:?}",
        f.code
    );
    assert!(
        f.code.contains(&Lir::I64Mul),
        "the checked multiply survives (guarded), got: {:?}",
        f.code
    );
}

#[test]
fn a_two_arm_match_with_leaf_bodies_selects() {
    // The match analogue of the `if`→`select` rewrite: a 2-arm scalar/bool match with a literal
    // probe + wildcard (or the two Bool literals) and cheap trap-free LEAF bodies emits a branchless
    // `select`, not an `if`/`else`. `(match n (0 a) (_ b))` → `a ; b ; (n eqz) ; select` (the 0-probe
    // uses `eqz`, cycle-43); `(match p (true a) (false b))` → `a ; b ; p ; select` (a Bool IS its own
    // condition — no `p == 1` compare). A NON-leaf body / a guard / >2 arms keeps the probe chain.
    let lir = |src: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(src);
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code
    };
    // (match n (0 a) (_ b)) → a ; b ; n ; eqz ; select.
    let zero = lir(
        "(module m (def (f (: n Int64) (: a Int64) (: b Int64)) (match n (0 a) (_ b))) (def (main) 0) (export main))",
    );
    assert_eq!(
        zero,
        vec![
            Lir::LocalGet(1), // a
            Lir::LocalGet(2), // b
            Lir::LocalGet(0), // n
            Lir::I64Eqz,      // n == 0
            Lir::Select,
        ],
        "a 2-arm 0-probe match selects with eqz"
    );
    // (match p (true a) (false b)) → a ; b ; p ; select — no `p == 1` compare (a Bool is the cond).
    let boolm = lir(
        "(module m (def (f (: p Bool) (: a Int64) (: b Int64)) (match p (true a) (false b))) (def (main) 0) (export main))",
    );
    assert_eq!(
        boolm,
        vec![
            Lir::LocalGet(1),
            Lir::LocalGet(2),
            Lir::LocalGet(0),
            Lir::Select,
        ],
        "a Bool 2-arm match selects on the bare condition"
    );
    // A body that is NOT trap-free (`(+ a 1)`, a checked add) keeps the structured if (no select) —
    // `select` would evaluate the untaken arm, possibly surfacing its overflow trap.
    let nonleaf = lir(
        "(module m (def (f (: n Int64) (: a Int64) (: b Int64)) (match n (0 (+ a 1)) (_ b))) (def (main) 0) (export main))",
    );
    assert!(
        !nonleaf.contains(&Lir::Select) && nonleaf.iter().any(|i| matches!(i, Lir::If(_))),
        "a possibly-trapping arm body keeps the if, got: {nonleaf:?}"
    );
}

#[test]
fn a_two_arm_match_with_small_trap_free_op_bodies_selects() {
    // The match analogue of cycle-161/162's widened `if`→`select`: a 2-arm scalar/bool match whose
    // bodies are small TRAP-FREE ops (not bare leaves) — here `(& x 7)` / `(| x 8)` — emits a
    // branchless `select`, not a probe chain. Sound for the same reason as the `if` case: a bitwise op
    // can neither trap nor allocate on the untaken path. This unifies the match dispatch with the `if`
    // dispatch (both use `is_select_arm`).
    let lir = |src: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(src);
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code
    };
    // (match n (0 (& x 7)) (_ (| x 8))) → branchless select over the two bitwise arms.
    let ops = lir(
        "(module m (def (f (: n Int64) (: x Int64)) (match n (0 (& x 7)) (_ (| x 8)))) (def (main) 0) (export main))",
    );
    assert!(
        ops.contains(&Lir::Select) && !ops.iter().any(|i| matches!(i, Lir::If(_) | Lir::Else)),
        "small trap-free op arms select branchlessly, got: {ops:?}"
    );
    assert!(
        ops.contains(&Lir::I64And) && ops.contains(&Lir::I64Or),
        "both trap-free arms are emitted before the select, got: {ops:?}"
    );
    // A body binding the SCRUTINEE (`(match n (0 -1) (m (& m 255)))` — `m` binds `n`) selects too: the
    // binder reads the scrutinee's spill slot, which is materialized before the arm bodies emit.
    let bind = lir(
        "(module m (def (f (: n Int64)) (match n (0 -1) (m (& m 255)))) (def (main) 0) (export main))",
    );
    assert!(
        bind.contains(&Lir::Select) && !bind.iter().any(|i| matches!(i, Lir::If(_))),
        "a scrutinee-binding trap-free arm selects, got: {bind:?}"
    );
}

#[test]
fn the_terminal_pair_of_a_sparse_match_chain_selects() {
    // A 3+-arm SPARSE scalar match (not dense enough for a br_table) emits a linear probe chain — but
    // its TERMINAL pair (the last literal-probe arm + the wildcard cover) is a 2-arm select shape, so
    // when both are trap-free `is_select_arm` bodies it emits a branchless `select` there instead of a
    // nested `if`/`else`. `(match x (0 10) (100 20) (_ 30))`: the outer `(0 10)` stays an `if` (its
    // else is the inner match sub-chain), but the `(100 20)/(_ 30)` tail → `20 ; 30 ; (x==100) ;
    // select`. So the chain has exactly ONE structured `if` (the outer 0-probe) and ONE `select`.
    let lir = |src: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(src);
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code
    };
    let sparse = lir(
        "(module m (def (f (: x Int64)) (match x (0 10) (100 20) (_ 30))) (def (main) 0) (export main))",
    );
    assert_eq!(
        sparse.iter().filter(|i| matches!(i, Lir::Select)).count(),
        1,
        "the terminal pair selects, got: {sparse:?}"
    );
    assert_eq!(
        sparse.iter().filter(|i| matches!(i, Lir::If(_))).count(),
        1,
        "only the outer 0-probe stays a structured if, got: {sparse:?}"
    );
    // A 4-arm sparse chain: only the LAST pair selects; the two leading probes stay `if`s.
    let four = lir(
        "(module m (def (f (: x Int64)) (match x (0 1) (5 2) (9 3) (_ 4))) (def (main) 0) (export main))",
    );
    assert_eq!(
        four.iter().filter(|i| matches!(i, Lir::Select)).count(),
        1,
        "the 4-arm chain's terminal pair selects once, got: {four:?}"
    );
    // A terminal pair with a POSSIBLY-TRAPPING body (`(+ y 1)`, checked add) does NOT select — the
    // chain stays a nested `if` for that pair.
    let trapping = lir(
        "(module m (def (f (: x Int64) (: y Int64)) (match x (0 y) (7 (+ y 1)) (_ y))) (def (main) 0) (export main))",
    );
    assert!(
        trapping.iter().filter(|i| matches!(i, Lir::Select)).count() == 0,
        "a possibly-trapping terminal-pair body keeps the if, got: {trapping:?}"
    );
}

// ── runtime lowering: a parameterized function body selects to local reads + machine ops ──────
//
// These select a FUNCTION body standalone (as `select_function`, the path an exported function
// takes) — the parameters are runtime values, so their references become `local.get` and the
// operation is a runtime machine op, NOT folded. Asserted at the Lir level (no export/run yet).

/// Locate def `name`'s parameter name-occurrences (seeing through `(: a T)`) and body, plus solve
/// each param's type — the inputs `select_function` takes for an exported parameterized function.
fn function_of(db: &mut Db, name: &str) -> (Vec<(StructId, Ty)>, StructId) {
    let d = db.def_by_name(name).expect("def present");
    let sig_params = db.defs[d].params.clone();
    let body = db.defs[d].body.expect("body");
    let mut params = Vec::new();
    for p in sig_params {
        // The name occurrence a reference binds to — bare `a` or the inner name of `(: a T)`.
        let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => p,
        };
        let ty = type_of(db, binder);
        params.push((binder, ty));
    }
    (params, body)
}

// ---- B2 co-verify harness (sharing-aware emit-into-Let-slot) ----------------------------------
// The durable B2 fix (v-core-opt owns the Core-IR: lift cse_body Guard A for heap handles + place a
// shared heap node in a Core::Let slot; I own the emit-side dup/drop) rests on one refcount invariant:
// when a shared heap node becomes a Let slot whose reads are `Core::LocalRef` occurrences, each read
// must be marked by EXACTLY ONE of {B1 `collect_shell_reclaim_child_dups`/`collect_row_op_field_dups`,
// `mark_binder_dups` (dup_sites)} — never BOTH (double-dup = leak) and never neither (missed retain =
// UAF). This harness pins that the three collectors are PAIRWISE DISJOINT on the B2 target shapes, built
// here from real source over RUNTIME HEAP PARAMS (a constructed literal constant-folds away; a param
// can't) — the same shapes the B2 pass will produce implicitly. Covered: `mark_binder_dups` fires (a
// real retain) at body root, WITHIN a match arm, and on a SHARED match scrutinee (the last confirms the
// Q2 ownership caveat: a shared scrutinee is BORROWED so shell_reclaim stays empty and dup covers it —
// exactly-one, never both). NOT yet exercised here: shell_reclaim firing SIMULTANEOUSLY with dup on
// distinct nodes — its narrow owned-compound-boxed-sum-with-consuming-scrutinee-child-projection pattern
// is not reachable from these source shapes (match arms bind payloads as fresh binders, not the direct
// scrutinee-child projections the collector keys on). v-core-opt hands the exact synth_core for that
// (+ the enclosing-scope binding) when their design MR frees the branch; it slots into `b2_dup_site_sets`
// unchanged. Agreed co-verify before either side lands B2 code.

/// Run the three dup-site collectors over `body`: (shell_reclaim, row_op, dup_sites).
fn b2_dup_site_sets(
    db: &mut Db,
    body: StructId,
) -> (HashSet<StructId>, HashSet<StructId>, HashSet<StructId>) {
    let mut shell = HashSet::new();
    collect_shell_reclaim_child_dups(db, body, &mut shell);
    let mut row = HashSet::new();
    collect_row_op_field_dups(db, body, &mut row);
    let mut binders = Vec::new();
    collect_retain_candidate_binders(db, body, &mut binders);
    let mut dup = HashSet::new();
    collect_dup_sites(db, body, &binders, &mut dup);
    (shell, row, dup)
}

/// The B2 invariant: no node id is marked by more than one collector (exactly-one-of per shared read).
fn assert_dup_sites_pairwise_disjoint(
    shell: &HashSet<StructId>,
    row: &HashSet<StructId>,
    dup: &HashSet<StructId>,
) {
    assert!(
        shell.is_disjoint(row),
        "B2: shell_reclaim and row_op must be disjoint ({shell:?} vs {row:?})"
    );
    assert!(
        shell.is_disjoint(dup),
        "B2: shell_reclaim and dup_sites must be disjoint ({shell:?} vs {dup:?})"
    );
    assert!(
        row.is_disjoint(dup),
        "B2: row_op and dup_sites must be disjoint ({row:?} vs {dup:?})"
    );
}

#[test]
fn b2_disjoint_body_root_heap_let_shared_k_reads() {
    // B2 shape (i): a RUNTIME heap PARAM `e` (a List — a param can't constant-fold) consumed by
    // `List.push` AND read again by a later `List.len`. `mark_binder_dups` marks the consume-with-later-
    // use occurrence a dup site; B1 marks nothing (no MatchSum scrutinee, no self-keyed materialize-
    // record). Sets pairwise disjoint.
    let ast = crate::testkit::parse(
        "(module m (def (f (: e (List Int64))) \
               (+ (List.len (List.push e 9)) (List.len e))) \
             (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let _ = select_function(&mut db, body, &params, &layout).expect("select f");
    let (shell, row, dup) = b2_dup_site_sets(&mut db, body);
    assert!(
        !dup.is_empty(),
        "the consume-then-later-use e must be a dup site"
    );
    assert!(
        shell.is_empty(),
        "no MatchSum scrutinee -> shell_reclaim empty"
    );
    assert!(
        row.is_empty(),
        "no self-keyed materialize-record -> row_op empty"
    );
    assert_dup_sites_pairwise_disjoint(&shell, &row, &dup);
}

#[test]
fn b2_disjoint_within_arm_heap_let() {
    // B2 shape (ii): a runtime heap PARAM `e` used consume-then-later INSIDE a match arm (the cmb1
    // within-arm placement). The outer match is over an enum-disc `Col` (scalar disc, NOT owned-compound-
    // boxed) so shell_reclaim stays empty; `mark_binder_dups` marks the arm-local occurrence. Disjoint.
    let ast = crate::testkit::parse(
        "(module m (type Col (Red) (Green) (Blue)) \
               (def (f (: c Col) (: e (List Int64))) \
                 (match c \
                   ((Red) (+ (List.len (List.push e 9)) (List.len e))) \
                   ((Green) 0) \
                   ((Blue) 0))) \
             (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let _ = select_function(&mut db, body, &params, &layout).expect("select f");
    let (shell, row, dup) = b2_dup_site_sets(&mut db, body);
    assert!(
        !dup.is_empty(),
        "the arm-local consume-then-later-use e must be a dup site"
    );
    assert!(
        shell.is_empty(),
        "enum-disc scrutinee is not owned-compound-boxed -> shell_reclaim empty"
    );
    assert_dup_sites_pairwise_disjoint(&shell, &row, &dup);
}

#[test]
fn b2_disjoint_shared_owned_boxed_sum_scrutinee() {
    // B2 shape (iii), the Q2 ownership caveat + the MEANINGFUL case where B1 shell_reclaim and dup_sites
    // are BOTH non-empty: a boxed sum `Box` (a compound `(List Int64)` payload) bound to `w`, matched
    // TWICE. The FIRST arm CONSUMES the extracted payload (`List.push xs 9`) -> shell_reclaim marks that
    // consuming payload site (a SumPayload/Proj node); `w` is matched a second time so it is SHARED, and
    // its first (consuming) scrutinee occurrence gets a dup site (a `LocalRef` off `w`). shell marks the
    // payload node, dup marks the scrutinee LocalRef -> DIFFERENT node ids -> disjoint. This is exactly
    // the double-dup=leak guard: the scrutinee read is covered by mark_binder_dups, the payload by B1,
    // never both.
    let ast = crate::testkit::parse(
        "(module m (type Box (Wrap (List Int64)) (Empty)) \
               (def (f (: w Box)) \
                 (+ (match w ((Wrap xs) (List.len (List.push xs 9))) ((Empty) 0)) \
                    (match w ((Wrap ys) (List.len ys)) ((Empty) 0)))) \
             (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let _ = select_function(&mut db, body, &params, &layout).expect("select f");
    let (shell, row, dup) = b2_dup_site_sets(&mut db, body);
    // A SHARED scrutinee is BORROWED (not the last use), so shell_reclaim (which reclaims an OWNED
    // scrutinee's shell) does NOT fire — mark_binder_dups covers the shared scrutinee read instead. The
    // Q2 ownership caveat thus resolves to exactly-one (dup), never both. #4857's non-tail-spine
    // relaxation of `collect_shell_reclaim_child_dups` initially over-fired here (it marked w's first
    // consuming payload extraction, a SumPayload the shared-read dup already owns, because
    // count_param_consumes==0 does not detect a scrutinee matched TWICE) → the `count_matchsum_over_binder
    // <= 1` sole-match guard now excludes the shared param, restoring exactly-one-of.
    assert!(!dup.is_empty(), "the shared scrutinee w must be a dup site");
    assert!(
        shell.is_empty(),
        "a shared (borrowed) scrutinee is not owned -> shell_reclaim empty"
    );
    assert_dup_sites_pairwise_disjoint(&shell, &row, &dup);
}

#[test]
fn a_parameterized_addition_selects_to_a_checked_sequence() {
    // (def (add (: a Int64) (: b Int64)) (+ a b)) — the body is a RUNTIME add over two params, and
    // the numeric model requires it to TRAP on overflow, so it selects to the CHECKED sequence.
    // Both operands are ALREADY in locals (params, slots 0,1), so they are read DIRECTLY — no copy
    // into `$a`/`$b` scratch (see `operand_src`). Only the result needs scratch: $r = slot 2.
    // get0 get1 add set$r; signed-overflow guard `((r^a)&(r^b))<0 → if unreachable` reading the
    // params' own slots; get$r.
    let ast = crate::testkit::parse(
        "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "add");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(f.params, vec![ValType::I64, ValType::I64]);
    assert_eq!(
        f.code,
        vec![
            // r = a + b — operands read straight from the param slots, no set$a/set$b copies.
            Lir::LocalGet(0),
            Lir::LocalGet(1),
            Lir::I64Add,
            // `local.set 2 ; local.get 2` (store $r, then the guard's first read of $r) is fused by
            // the `peephole` pass into a single `local.tee 2`.
            Lir::LocalTee(2),
            // overflow guard: ((r^a) & (r^b)) < 0 → trap, reading a=slot0, b=slot1 directly.
            Lir::LocalGet(0),
            Lir::I64Xor,
            Lir::LocalGet(2),
            Lir::LocalGet(1),
            Lir::I64Xor,
            Lir::I64And,
            Lir::ConstI64(0),
            Lir::I64LtS,
            Lir::IfIntegerOverflowEnd,
            // result
            Lir::LocalGet(2),
        ]
    );
    // One i64 scratch local declared ($r) — the operand copies ($a,$b) are eliminated.
    assert_eq!(f.declared, vec![ValType::I64; 1]);
    assert!(f.ret.agrees_with(&Ty::int64()));
}

#[test]
fn a_constant_operand_is_inlined_not_stashed_in_scratch() {
    // (def (f (: a Int64)) (+ a 1)) — the RHS is a compile-time constant. `operand_src` returns a
    // `Const` source for it, so it is pushed inline (`i64.const 1`) at the add rather than stored
    // into a `$b` scratch local. Only $r needs scratch. And because a constant `+`/`-` operand at
    // full signed width lets the overflow guard SPECIALIZE, the guard is a single `r <ₛ a` compare
    // (C=1>0 for `+` overflows only upward → wrap makes `r < a`), NOT the general two-`xor` sign
    // test. Sequence: get$a const1 add tee$r ; get$a lt_s ; if unreachable ; get$r — the `set$r ;
    // get$r` pair fuses to `local.tee` via the peephole.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64)) (+ a 1)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![
            // r = a + 1 — `a` from its param slot, `1` inline (no $b scratch).
            Lir::LocalGet(0),
            Lir::ConstI64(1),
            Lir::I64Add,
            Lir::LocalTee(1), // set $r then the guard's first read of $r, fused.
            // specialized guard: `r <ₛ a` → trap (a constant `+1` overflows only past MAX).
            Lir::LocalGet(0),
            Lir::I64LtS,
            Lir::IfIntegerOverflowEnd,
            Lir::LocalGet(1),
        ]
    );
    // Only $r (slot 1) is declared — the constant operand needs no scratch slot at all.
    assert_eq!(f.declared, vec![ValType::I64; 1]);
}

#[test]
fn a_list_at_on_a_param_reads_the_param_slot_directly_no_handle_copy() {
    // (def (at (: xs (List Int64)) (: i Int64)) (List.at xs i)) — the list is a parameter, already
    // resident in slot 0 for the whole body. `vec-len` (bounds check) and `vec-get` (element read)
    // BORROW it, so both read slot 0 DIRECTLY — no copy into a scratch slot first (the heap analogue
    // of the scalar operand-slot reuse). So the body has NO `LocalSet(0)` (a param slot is never
    // stored to here), and every `vec-len`/`vec-get` is immediately preceded by `LocalGet(0)`.
    let ast = crate::testkit::parse(
        "(module m (def (at (: xs (List Int64)) (: i Int64)) (List.at xs i)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "at");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    // The list handle is never copied into a scratch slot: no `local.set` targets the param slot 0,
    // and — since the reuse frees the would-be list scratch slot — no `local.set`/`tee` of the list
    // handle appears at all before the first `vec-len`. Assert both `vec-len` and `vec-get` read the
    // list param slot 0 directly.
    let vec_len_pos = f
        .code
        .iter()
        .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_LEN))
        .expect("a bounds-check vec-len");
    assert_eq!(
        f.code[vec_len_pos - 1],
        Lir::LocalGet(0),
        "the bounds-check vec-len reads the list param slot 0 directly; got {:?}",
        &f.code[..=vec_len_pos]
    );
    let vec_get_pos = f
        .code
        .iter()
        .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_GET))
        .expect("an element vec-get");
    // vec-get takes the wrapped index on top, so the handle is one deeper: `LocalGet(0) ; LocalGet(idx)
    // ; I32WrapI64 ; vec-get`. Confirm slot 0 is pushed for the handle (three before the call).
    assert_eq!(
        f.code[vec_get_pos - 3],
        Lir::LocalGet(0),
        "the element vec-get reads the list param slot 0 directly; got {:?}",
        &f.code[vec_get_pos - 3..=vec_get_pos]
    );
    // The list handle is never COPIED into slot 0 BEFORE its last read (vec-get): the borrowed
    // list param is read directly through both borrows, never stored into scratch first — that is
    // the no-handle-copy intent this test guards. (Relaxed deliberately: AFTER vec-get the borrowed
    // list is fully consumed, so local-slot COALESCING may soundly re-home a later local into the
    // now-dead param slot 0 — `reuse_dead_param_slots`, the same dead-slot reuse `wasm-opt` does.
    // That post-consumption `LocalTee(0)` is a legitimate slot reuse, NOT a handle copy, so the
    // guard covers only the pre-consumption prefix, not the whole body.)
    assert!(
        !f.code[..=vec_get_pos]
            .iter()
            .any(|i| matches!(i, Lir::LocalSet(0) | Lir::LocalTee(0))),
        "the list param slot 0 is never written BEFORE its last read (no handle copy); got {:?}",
        &f.code[..=vec_get_pos]
    );

    // BYTES.AT shares the same reuse (bytes handle read by `bytes-len` + `bytes-get`): a param bytes
    // value in slot 0 is read directly, never copied into scratch.
    let ast = crate::testkit::parse(
        "(module m (def (at (: bs Bytes) (: i Int64)) (Bytes.at bs i)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "at");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    let blen_pos = f
        .code
        .iter()
        .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_BYTES_LEN))
        .expect("a bounds-check bytes-len");
    assert_eq!(
        f.code[blen_pos - 1],
        Lir::LocalGet(0),
        "the bounds-check bytes-len reads the bytes param slot 0 directly; got {:?}",
        &f.code[..=blen_pos]
    );
    // Same guard + deliberate relaxation as the List.at case: the bytes handle is never COPIED into
    // slot 0 before its last read (bytes-get); after that the borrowed bytes is consumed, so
    // dead-param-slot COALESCING may soundly re-home a later local into slot 0. Guard the prefix.
    let bget_pos = f
        .code
        .iter()
        .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_BYTES_GET))
        .expect("an element bytes-get");
    assert!(
        !f.code[..=bget_pos]
            .iter()
            .any(|i| matches!(i, Lir::LocalSet(0) | Lir::LocalTee(0))),
        "the bytes param slot 0 is never written BEFORE its last read (no handle copy); got {:?}",
        &f.code[..=bget_pos]
    );
}

#[test]
fn a_list_match_on_a_param_reads_the_scrutinee_slot_directly_no_handle_copy() {
    // (def (hd (: xs (List Int64))) (match xs ((list) 0) ((list h .. rest) h))) — the scrutinee is a
    // parameter, resident in slot 0. The match reads its handle for `vec-len` (length dispatch) and the
    // arm bodies' element reads (`vec-get`, BORROW) + rest read (`vec-drop`, `dup`-guarded); all read
    // slot 0 DIRECTLY — the handle is NOT copied into a scratch slot first (the c180 reuse, matching the
    // `MatchSum`/`List.at` discipline). So the FIRST `vec-len` reads `LocalGet(0)`, and slot 0 is never
    // written (a param — the reuse removes the would-be `local.set handle_slot` copy).
    let ast = crate::testkit::parse(
        "(module m (def (hd (: xs (List Int64))) (match xs ((list) 0) ((list h .. rest) h))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "hd");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    // The length dispatch's `vec-len` reads the scrutinee param slot 0 directly (no prior copy).
    let vec_len_pos = f
        .code
        .iter()
        .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_LEN))
        .expect("a length-dispatch vec-len");
    assert_eq!(
        f.code[vec_len_pos - 1],
        Lir::LocalGet(0),
        "the list match's vec-len reads the scrutinee param slot 0 directly; got {:?}",
        &f.code[..=vec_len_pos]
    );
    // The scrutinee param slot 0 is never written — the handle-copy `local.set` is gone.
    assert!(
        !f.code
            .iter()
            .any(|i| matches!(i, Lir::LocalSet(0) | Lir::LocalTee(0))),
        "the scrutinee param slot 0 is never copied (no handle stash); got {:?}",
        f.code
    );
}

#[test]
fn an_option_expect_on_a_param_reads_the_scrutinee_slot_directly_no_handle_copy() {
    // (def (unwrap (: o (Option Int64))) (Option.expect o "v")) — the scrutinee is a parameter, resident
    // in slot 0. `SumExpect` reads its handle TWICE — the disc probe (`sum-disc`) and the present-payload
    // read (`sum-payload`), both BORROWING — so both read slot 0 DIRECTLY, no copy into a scratch slot
    // (the c181 reuse, matching the `MatchSum`/`List.at`/`MatchList` discipline). So `sum-disc` reads
    // `LocalGet(0)`, and slot 0 is never written.
    let ast = crate::testkit::parse(
        "(module m (def (unwrap (: o (Option Int64))) (Option.expect o \"v\")) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "unwrap");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    // The disc probe's `sum-disc` reads the scrutinee param slot 0 directly (no prior copy).
    let disc_pos = f
        .code
        .iter()
        .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_SUM_DISC))
        .expect("a disc probe sum-disc");
    assert_eq!(
        f.code[disc_pos - 1],
        Lir::LocalGet(0),
        "the expect's sum-disc reads the scrutinee param slot 0 directly; got {:?}",
        &f.code[..=disc_pos]
    );
    // The present-payload `sum-payload` also reads slot 0 directly.
    let payload_pos = f
        .code
        .iter()
        .position(|i| matches!(i, Lir::CallImport(op) if *op == OP_SUM_PAYLOAD))
        .expect("a present-payload sum-payload");
    assert_eq!(
        f.code[payload_pos - 1],
        Lir::LocalGet(0),
        "the expect's sum-payload reads the scrutinee param slot 0 directly; got {:?}",
        &f.code[payload_pos - 1..=payload_pos]
    );
    // The scrutinee param slot 0 is never written — the handle-copy `local.set` is gone.
    assert!(
        !f.code
            .iter()
            .any(|i| matches!(i, Lir::LocalSet(0) | Lir::LocalTee(0))),
        "the scrutinee param slot 0 is never copied (no handle stash); got {:?}",
        f.code
    );
}

#[test]
fn multiply_by_power_of_two_strength_reduces_to_a_shift() {
    // (def (f (: n Int64)) (* n 8)) — `* 2^k` becomes `<< k` (here k=3): push n, `shl 3` into $r,
    // then the overflow round-trip (`($r >> 3) != n → trap`) — no `i64.mul`, no division-based
    // guard, no count guard (k is the inline constant 3, always < width). Sequence:
    // get n ; const 3 ; shl ; tee $r ; get $r ; const 3 ; shr_s ; get n ; ne ; if unreachable ; get $r.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) (* n 8)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![
            Lir::LocalGet(0),
            Lir::ConstI64(3),
            Lir::I64Shl,
            Lir::LocalTee(1), // set $r then the round-trip's first read of $r, fused.
            Lir::ConstI64(3),
            Lir::I64ShrS, // arithmetic shift (signed) for the exact round-trip.
            Lir::LocalGet(0),
            Lir::I64Ne,
            Lir::IfIntegerOverflowEnd,
            Lir::LocalGet(1),
        ]
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I64Mul)),
        "the multiply is strength-reduced away, no i64.mul"
    );
}

#[test]
fn a_provably_in_range_shift_elides_its_overflow_guard() {
    let select = |src: &str| {
        let mut db = Db::load(crate::testkit::parse(src));
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code
    };
    // `(* (& x 15) 2)` → `(& x 15) << 1` ∈ [0,30], fits Int64 → NO round-trip guard (`shr ; ne`).
    let mul =
        select("(module m (def (f (: x Int64)) (* (& x 15) 2)) (def (main) 0) (export main))");
    assert!(
        !mul.iter().any(|i| matches!(i, Lir::I64Ne))
            && !mul.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a provably-in-range `* 2^k` drops its shift-overflow guard; got {mul:?}"
    );
    // A user `(<< (& x 15) 2)` ∈ [0,60] likewise.
    let shl =
        select("(module m (def (f (: x Int64)) (<< (& x 15) 2)) (def (main) 0) (export main))");
    assert!(
        !shl.iter().any(|i| matches!(i, Lir::I64Ne)),
        "a provably-in-range `<<` drops its guard; got {shl:?}"
    );
    // SAFETY: a full-range `(<< x 2)` CAN overflow → keeps the round-trip guard.
    let open = select("(module m (def (f (: x Int64)) (<< x 2)) (def (main) 0) (export main))");
    assert!(
        open.iter().any(|i| matches!(i, Lir::I64Ne)),
        "a full-range `<<` keeps its guard; got {open:?}"
    );
    // SAFETY: `(<< (& x 15) 60)` = [0,15]<<60 overflows Int64 → keeps its guard.
    let over =
        select("(module m (def (f (: x Int64)) (<< (& x 15) 60)) (def (main) 0) (export main))");
    assert!(
        over.iter().any(|i| matches!(i, Lir::I64Ne)),
        "an over-range `<<` keeps its guard; got {over:?}"
    );
}

#[test]
fn multiply_by_a_non_power_of_two_keeps_the_checked_multiply() {
    // (* n 3) — 3 is not a power of two, so the strength reduction to a shift does NOT fire: the
    // checked `i64.mul` stays. Its overflow guard, however, is the CONST-MULTIPLIER bound check
    // (`n` must lie in `[MIN/3, MAX/3]` for `n*3` to fit), NOT the general `div_s` round-trip — a
    // constant multiplier lets a compile-time-constant interval test replace the hardware divide.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) (* n 3)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::I64Mul)),
        "a non-power-of-two multiply keeps i64.mul, got: {:?}",
        f.code
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I64Shl)),
        "a non-power-of-two multiply does not become a shift"
    );
    // The const-multiplier overflow guard is a bound check, NOT a `div_s` round-trip.
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I64DivS)),
        "a full-width const multiply's guard is a bound check, not div_s, got: {:?}",
        f.code
    );
}

#[test]
fn const_multiply_guard_is_a_single_unsigned_range_check() {
    // (* n 3) — the const-multiplier overflow guard shifts the fitting interval `[MIN/3, MAX/3]` to
    // start at 0 (`n - MIN/3`) and does ONE unsigned compare `> (MAX/3 - MIN/3)`, so BOTH out-of-
    // interval directions are caught by a single test + a single trap block. It reads `n` ONCE and
    // uses NO signed compares (the old two-`gt_s`/`lt_s` + two-trap-block guard is gone). Parity with
    // the two-compare form is gate-verified at every interval boundary, both signs of C.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) (* n 3)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    // ONE unsigned compare, no signed compares in the guard.
    assert_eq!(
        f.code.iter().filter(|i| matches!(i, Lir::I64GtU)).count(),
        1,
        "the guard is a single unsigned range check, got: {:?}",
        f.code
    );
    assert!(
        !f.code
            .iter()
            .any(|i| matches!(i, Lir::I64GtS | Lir::I64LtS)),
        "the unsigned range check replaces the two signed compares, got: {:?}",
        f.code
    );
    // The interval is shifted by `MIN/3` (the low endpoint) and the bound is its width `MAX/3-MIN/3`.
    let lo = i64::MIN / 3;
    assert!(
        f.code.contains(&Lir::ConstI64(lo)),
        "the guard subtracts the low endpoint MIN/3, got: {:?}",
        f.code
    );
    assert!(
        f.code
            .contains(&Lir::ConstI64((i64::MAX / 3).wrapping_sub(lo))),
        "the guard compares against the interval width MAX/3-MIN/3, got: {:?}",
        f.code
    );
    // Exactly ONE trap block (the two-block guard collapsed to one).
    assert_eq!(
        f.code
            .iter()
            .filter(|i| matches!(i, Lir::IfIntegerOverflowEnd))
            .count(),
        1,
        "the two trap blocks collapse to one, got: {:?}",
        f.code
    );
}

#[test]
fn a_non_const_multiply_keeps_the_div_s_guard() {
    // Only a CONSTANT multiplier gets the bound check; a two-runtime-operand `(* a b)` keeps the
    // general `div_s` round-trip guard (`if a≠0 { r/a≠b → trap }`) — there is no compile-time bound
    // to compare against.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (* a b)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.contains(&Lir::I64DivS),
        "a runtime-operand multiply keeps the div_s guard, got: {:?}",
        f.code
    );
}

#[test]
fn not_over_a_comparison_folds_to_the_complement_op() {
    // (def (f (: a Int64) (: b Int64)) (not (< a b))) — the negation folds into the complement
    // comparison `a >=ₛ b`: get a ; get b ; i64.ge_s — NO i32.eqz.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (not (< a b))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64GeS],
        "not(<) is the single complement ge_s, no eqz"
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I32Eqz)),
        "the eqz is folded away into the complement comparison"
    );
}

#[test]
fn not_over_equality_folds_to_ne() {
    // (not (= a b)) → i64.ne (not i64.eq ; i32.eqz).
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (not (= a b))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(f.code, vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64Ne]);
}

#[test]
fn not_over_an_unsigned_comparison_uses_the_unsigned_complement() {
    // (not (< a b)) over UInt64 → i64.ge_U (the unsigned complement, not the signed ge_s).
    let ast = crate::testkit::parse(
        "(module m (def (f (: a UInt64) (: b UInt64)) (not (< a b))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.contains(&Lir::I64GeU) && !f.code.iter().any(|i| matches!(i, Lir::I32Eqz)),
        "unsigned not(<) is ge_u, no eqz, got: {:?}",
        f.code
    );
}

#[test]
fn if_c_one_zero_materializes_the_bool_without_a_select() {
    // (def (f (: a Int64) (: b Int64)) (if (< a b) 1 0)) — the boolean materialization: the compare
    // already yields 0/1, so the `if` is just that i32 bool widened to the i64 result — NO two consts,
    // NO select, NO branch.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) 1 0)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![
            Lir::LocalGet(0),
            Lir::LocalGet(1),
            Lir::I64LtS,
            Lir::I64ExtendI32U, // widen the 0/1 bool to the Int64 result — no select.
        ]
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::Select)),
        "the 1/0 branches materialize the condition directly, no select"
    );
}

#[test]
fn if_c_zero_one_materializes_the_negated_bool() {
    // (if (< a b) 0 1) — the reversed literals are the NEGATION of the condition. Since the condition
    // is a comparison, the negation folds into the COMPLEMENT comparison (`a >=ₛ b`) rather than
    // `compare ; i32.eqz` — one instruction fewer, no double negation — then widen.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) 0 1)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![
            Lir::LocalGet(0),
            Lir::LocalGet(1),
            Lir::I64GeS, // the complement of `<` — no trailing eqz.
            Lir::I64ExtendI32U,
        ]
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I32Eqz)),
        "the negated materialization folds into the complement, no eqz"
    );
}

#[test]
fn if_not_compare_one_zero_avoids_the_double_negation() {
    // (if (not (< a b)) 1 0) — `lower` branch-swaps this to `(if (< a b) 0 1)`, then the negated
    // materialization would naively stack `i32.eqz` on the compare. Because the condition is a
    // comparison, the negation folds into the complement `a >=ₛ b` — NO `eqz` at all (the fold that
    // prevents an `eqz ; eqz` when `(not (= n 0))` composes with the bool-int form).
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (if (not (< a b)) 1 0)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![
            Lir::LocalGet(0),
            Lir::LocalGet(1),
            Lir::I64GeS,
            Lir::I64ExtendI32U,
        ]
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I32Eqz)),
        "no eqz — the negation folded into the complement comparison"
    );
}

#[test]
fn if_not_eq_zero_one_zero_is_a_single_ne() {
    // (if (not (= n 0)) 1 0) — the ubiquitous "n is nonzero as an int" idiom. Was `eqz ; eqz` (the
    // compare-with-zero peephole then the negation). Now the negation folds the compare's complement:
    // `n ≠ 0` = `n ; const 0 ; i64.ne` — one `ne`, no double eqz.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) (if (not (= n 0)) 1 0)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        !f.code
            .iter()
            .any(|i| matches!(i, Lir::I64Eqz | Lir::I32Eqz)),
        "no eqz double negation, got: {:?}",
        f.code
    );
    assert!(
        f.code.contains(&Lir::I64Ne),
        "nonzero folds to a single i64.ne, got: {:?}",
        f.code
    );
}

#[test]
fn if_with_non_zero_one_constants_keeps_the_select() {
    // (if (< a b) 5 7) — the branches are not 1/0, so the materialization does NOT fire; the leaf
    // branches still lower to a branchless `select` (5, 7, cond).
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) 5 7)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::Select)),
        "non-0/1 constant branches keep the select, got: {:?}",
        f.code
    );
}

#[test]
fn signed_negation_uses_the_a_equals_min_guard_not_the_two_xor_sub() {
    // (def (f (: a Int64)) (- 0 a)) — negation: the machine `0 - a` plus a guard that traps iff
    // `a == MIN` (the one overflow), NOT the general two-`xor` signed-sub guard.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64)) (- 0 a)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I64Xor)),
        "negation's guard is a == MIN, not the two-xor sub guard, got: {:?}",
        f.code
    );
    assert!(
        f.code.contains(&Lir::ConstI64(i64::MIN)) && f.code.contains(&Lir::I64Eq),
        "the guard compares the operand against MIN, got: {:?}",
        f.code
    );
}

#[test]
fn signed_divide_by_power_of_two_strength_reduces_to_the_bias_shift_sequence() {
    // (def (f (: n Int64)) (/ n 8)) — a SIGNED `/ 2^k` (k=3) becomes the branchless round-toward-zero
    // bias sequence, no `i64.div_s`: stash n in $a, then `(n + ((n >>ₛ 63) >>ᵤ 61)) >>ₛ 3`.
    // With dead-param-slot reuse (plain-wasm) the dividend scratch $a re-homes into the param's OWN
    // slot 0 (n is dead after these reads, so `reuse_dead_param_slots` coalesces $a onto slot 0):
    // `LocalGet(0) ; LocalTee(0)` re-stashes n into its own slot (a harmless self-tee) → 0 declared
    // scratch locals instead of 1.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) (/ n 8)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![
            Lir::LocalGet(0),
            Lir::LocalTee(0), // $a = n re-homed into the dead param slot 0 (self-tee), keeping n on the stack.
            Lir::LocalGet(0),
            Lir::ConstI64(63),
            Lir::I64ShrS, // n >>ₛ 63 — all-ones iff n<0.
            Lir::ConstI64(61),
            Lir::I64ShrU, // >>ᵤ (64−3) — 2^3−1 iff n<0, else 0.
            Lir::I64Add,  // n + bias.
            Lir::ConstI64(3),
            Lir::I64ShrS, // >>ₛ 3.
        ]
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I64DivS)),
        "the signed divide is strength-reduced away, no i64.div_s"
    );
}

#[test]
fn signed_remainder_by_power_of_two_strength_reduces_without_rem_s() {
    // (def (f (: n Int64)) (% n 8)) — a SIGNED `% 2^k` reduces to `n − (q << k)` over the same bias
    // quotient, no `i64.rem_s`.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) (% n 8)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I64RemS)),
        "the signed remainder is strength-reduced away, no i64.rem_s, got: {:?}",
        f.code
    );
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::I64Sub))
            && f.code.iter().any(|i| matches!(i, Lir::I64Shl)),
        "remainder is n − (q << k), so a sub over a shifted quotient, got: {:?}",
        f.code
    );
}

#[test]
fn divide_by_a_non_power_of_two_keeps_the_machine_divide() {
    // (/ n 3) — 3 is not a power of two, so the strength reduction does NOT fire and the machine
    // `i64.div_s` stays.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) (/ n 3)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::I64DivS)),
        "a non-power-of-two divide keeps i64.div_s, got: {:?}",
        f.code
    );
}

#[test]
fn a_right_shift_leaves_its_result_on_the_stack_without_a_dead_store() {
    // (def (f (: a Int64)) (>> a 3)) — a `>>` is EXACT (its result only shrinks), so it needs no
    // overflow round-trip and no range-check. The result stays on the stack: just `get a ; const 3 ;
    // shr_s`, with NO `$r` store and NO declared local. (The old code routed EVERY shift through a
    // `set $r ; get $r` round-trip — dead motion + a dead local for `>>`, since only `<<` reads `$r`
    // back for its overflow check.)
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64)) (>> a 3)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![Lir::LocalGet(0), Lir::ConstI64(3), Lir::I64ShrS],
        "a constant-count `>>` is exactly the machine shift — no dead round-trip"
    );
    assert!(
        f.declared.is_empty(),
        "a `>>` claims no result scratch local, got: {:?}",
        f.declared
    );
}

#[test]
fn identical_operands_are_computed_once_via_cse() {
    // (def (f (: a Int64) (: b Int64)) (+ (* a b) (* a b))) — the two operands are the SAME product.
    // CSE computes `(* a b)` ONCE into a slot; the outer add reads that slot for BOTH operands. So
    // the body contains exactly ONE `i64.mul` (not two), and the add's operands are two reads of the
    // shared slot.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (+ (* a b) (* a b))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
        1,
        "the shared product is computed exactly once (CSE)"
    );
    // The add reads the product's slot twice as its two operands (a LocalGet of the same slot). Find
    // the mul's result slot (the LocalSet right after the sole I64Mul... or a LocalTee if the guard
    // fused) and confirm the I64Add is preceded by two reads of it.
    assert_eq!(
        f.code.iter().filter(|i| **i == Lir::I64Add).count(),
        1,
        "one add over the shared product"
    );
}

#[test]
fn a_multi_use_inlined_param_arg_is_computed_once_by_straight_line_cse() {
    // β-reduction SHARES a param's argument occurrence at every use. `(def (g s) (+ s (* s 3)))`
    // inlined with `s = (* a b)` leaves the ONE `(* a b)` node referenced twice — but across DIFFERENT
    // ops (`+` and `*`), so the intra-op arith-CSE (which shares only the two operands of ONE op) does
    // NOT catch it, and `(* a b)` emitted TWICE. Straight-line CSE now computes the shared `(* a b)`
    // ONCE into a slot up-front and reads it at both uses → exactly ONE `i64.mul` for the argument
    // (plus the `(* s 3)` = 2 total muls). Pins the count + relies on the corpus/run for value parity.
    let ast = crate::testkit::parse(
        "(module m (def (g (: s Int64)) (+ s (* s 3))) \
               (def (f (: a Int64) (: b Int64)) (g (* a b))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    // Two muls: the SHARED `(* a b)` computed once + the `(* s 3)`. Before straight-line CSE this was
    // THREE (the `(* a b)` argument duplicated at each of its two uses).
    assert_eq!(
        f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
        2,
        "the inlined multi-use argument `(* a b)` is computed once (2 muls: shared arg + `* s 3`), \
             got: {:?}",
        f.code
    );
}

#[test]
fn a_cse_slotted_checked_arith_rep_writes_directly_to_its_slot() {
    // When the CSE representative is a CHECKED arithmetic op (`+`/`-`/`*`), it is emitted with its
    // result DEST = the CSE slot (via `emit_operand_into`'s `ResultDest::Slot`), so its `$r` IS the
    // slot — the store is direct, with NO `local.get $r ; local.set $cse` register-move. Before this,
    // the checked op wrote its own `$r` scratch, then the CSE pass copied `$r → slot` (a wasted temp +
    // move). `(f x (+ x 1))` inlines `f(a,b)=(+ (* a b) (- a b))`; `b = (+ x 1)` is used twice → CSE'd.
    // Assert there is NO `LocalGet(t) ; LocalSet(s)` pair where `t != s` (a pure register-to-register
    // move) among the emitted code — the CSE arith stores straight into its slot.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (+ (* a b) (- a b))) \
               (def (g (: x Int64)) (f x (+ x 1))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "g");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    let reg_move = f
        .code
        .windows(2)
        .any(|w| matches!((&w[0], &w[1]), (Lir::LocalGet(t), Lir::LocalSet(s)) if t != s));
    assert!(
        !reg_move,
        "the CSE'd `(+ x 1)` writes directly to its slot — no `get t ; set s` register move, got: {:?}",
        f.code
    );
}

#[test]
fn a_single_use_inlined_param_arg_is_not_cse_slotted() {
    // A param used ONCE needs no CSE — the argument is inlined at its single site, same as before.
    // `(def (g s) (* s 5))` given `s = (* a b)` → exactly ONE `(* a b)` for the arg, plus the `(* s 5)`
    // = 2 muls (5 is not a power of two, so it stays a real mul, not a strength-reduced shift). No CSE
    // slot is introduced — straight-line CSE only fires at ≥2 references.
    let ast = crate::testkit::parse(
        "(module m (def (g (: s Int64)) (* s 5)) \
               (def (f (: a Int64) (: b Int64)) (g (* a b))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
        2,
        "single-use arg inlines (2 muls: the arg + `* s 5`), got: {:?}",
        f.code
    );
}

#[test]
fn a_repeated_indexed_read_shares_one_vec_get_via_cse() {
    // `(+ (Option.expect (List.at xs 2)) (Option.expect (List.at xs 2)))` — the SAME bounds-checked
    // indexed read (`vec-get` behind a bounds check, then unbox via `expect`) twice. `List.at` BORROWS
    // the list and is deterministic, and the whole `SumExpect(ListAt …)` is a SCALAR read (the element
    // is an `Int64`), so straight-line CSE computes it ONCE — the emitted body contains exactly ONE
    // `vec-get`, not two (the ~20-instruction bounds-check + read + unwrap sequence is shared). This is
    // the indexed-read analogue of the `List.len` CSE (a repeated count already shares its `vec-len`).
    let ast = crate::testkit::parse(
        "(module m (def (f (: xs (List Int64))) \
               (+ (Option.expect (List.at xs 2) \"v\") (Option.expect (List.at xs 2) \"v\"))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code
            .iter()
            .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_GET))
            .count(),
        1,
        "the repeated indexed read shares one vec-get (CSE), got: {:?}",
        f.code
    );
}

#[test]
fn a_repeated_map_lookup_shares_one_map_lookup_via_cse() {
    // `(+ (Option.expect (Map.lookup m 2)) (Option.expect (Map.lookup m 2)))` — the SAME keyed lookup
    // twice. `Map.lookup` BORROWS the map and is deterministic; the whole `SumExpect(MapLookup …)` is a
    // SCALAR read (the value is an `Int64`), so straight-line CSE computes it ONCE — exactly ONE
    // `map-lookup` (an O(log n) CHAMP walk), not two. The keyed-read analogue of the `List.at` CSE.
    let ast = crate::testkit::parse(
        "(module m (def (f (: m (Map Int64 Int64))) \
               (+ (Option.expect (Map.lookup m 2) \"v\") (Option.expect (Map.lookup m 2) \"v\"))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code
            .iter()
            .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_MAP_LOOKUP))
            .count(),
        1,
        "the repeated keyed lookup shares one map-lookup (CSE), got: {:?}",
        f.code
    );
}

#[test]
fn str_from_bytes_does_not_over_declare_arr_alloc() {
    // `String.from-bytes` emits `str-from-bytes` (decode → handle-or-NULL) then builds `Some(handle)` /
    // `None` via `sum-new`; the `None` payload is the INLINE-unit constant (`IMM_UNIT`), so no
    // `arr-alloc` is ever called. The used-ops collector must therefore NOT import `arr-alloc` for a
    // body whose only heap op is `str-from-bytes` (an earlier version over-declared it "for None's
    // unit", forcing an unnecessary runtime import — PR #404 Copilot review). The bytes come from a
    // PARAMETER so no construction op contributes other imports.
    let ast = crate::testkit::parse(
        "(module m (def (f (: b Bytes)) (String.from-bytes b)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let (_params, body) = function_of(&mut db, "f");
    let mut ops: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    collect_used_ops(&mut db, body, &mut ops);
    assert!(
        ops.contains(OP_STR_FROM_BYTES),
        "str-from-bytes must be imported, got: {ops:?}"
    );
    assert!(
        ops.contains(OP_SUM_NEW),
        "sum-new (Some/None build) must be imported, got: {ops:?}"
    );
    assert!(
        !ops.contains(OP_ARR_ALLOC),
        "arr-alloc must NOT be imported — None uses the inline-unit constant, not an allocation; \
             got: {ops:?}"
    );
}

#[test]
fn fallible_read_ops_do_not_over_declare_arr_alloc() {
    // Every fallible read that returns `(Option T)` — `List.at`, `Map.lookup`, `Bytes.at`,
    // `String.at`, `Bytes.slice` (the family sharing `String.from-bytes`'s shape) — builds its `None`
    // from the inline-unit constant (`IMM_UNIT`), NOT an allocation, so NONE of them calls
    // `arr-alloc` (verified against each emit arm). The used-ops collector must not import `arr-alloc`
    // for a body whose only heap op is one of these reads over PARAMETERS (no construction op
    // contributes other imports). This pins the whole family against the PR #404 over-declaration
    // class (an over-imported op forces an unnecessary component import).
    let cases: &[(&str, &str)] = &[
        (
            "(def (f (: xs (List Int64)) (: i Int64)) (List.at xs i))",
            "List.at",
        ),
        (
            "(def (f (: m (Map Int64 Int64)) (: k Int64)) (Map.lookup m k))",
            "Map.lookup",
        ),
        (
            "(def (f (: b Bytes) (: i Int64)) (Bytes.at b i))",
            "Bytes.at",
        ),
        (
            "(def (f (: s String) (: i Int64)) (String.at s i))",
            "String.at",
        ),
        (
            "(def (f (: b Bytes) (: s Int64) (: l Int64)) (Bytes.slice b s l))",
            "Bytes.slice",
        ),
    ];
    for (def, label) in cases {
        let src = format!("(module m {def} (def (main) 0) (export main))");
        let mut db = Db::load(crate::testkit::parse(&src));
        let (_params, body) = function_of(&mut db, "f");
        let mut ops: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
        collect_used_ops(&mut db, body, &mut ops);
        assert!(
            ops.contains(OP_SUM_NEW),
            "{label}: sum-new (Some/None build) must be imported, got: {ops:?}"
        );
        assert!(
            !ops.contains(OP_ARR_ALLOC),
            "{label}: arr-alloc must NOT be imported — None uses the inline-unit constant, not an \
                 allocation; got: {ops:?}"
        );
    }
}

#[test]
fn str_at_does_not_over_declare_drop() {
    // `String.at` DUPs the string (the slice takes an independent ref and consumes that dup), so the
    // ORIGINAL string is not consumed here — its owner (an enclosing let/param) reclaims it, and the
    // emit calls no `drop` (unlike `Map.lookup`/`Set.contains`, whose boxed KEY is an owned temporary
    // they must drop). The used-ops collector must not import `drop` for a `String.at` body — an
    // over-declaration found auditing the fallible-read family (the same import-minimization class as
    // the arr-alloc over-declares).
    let ast = crate::testkit::parse(
        "(module m (def (f (: s String) (: i Int64)) (String.at s i)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let (_params, body) = function_of(&mut db, "f");
    let mut ops: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    collect_used_ops(&mut db, body, &mut ops);
    assert!(
        ops.contains(OP_DUP) && ops.contains(OP_BYTES_SLICE),
        "String.at must import dup + bytes-slice (the dup-then-consume slice), got: {ops:?}"
    );
    assert!(
        !ops.contains(OP_DROP),
        "drop must NOT be imported — the original string is reclaimed by its owner, not dropped \
             here; got: {ops:?}"
    );
}

#[test]
fn straight_line_cse_value_numbers_distinct_occurrences_across_ops() {
    // VALUE-NUMBERING (not node identity): two DISTINCT `(* a b)` occurrences across DIFFERENT ops —
    // `(+ (* a b) (* (* a b) 3))` — are `core_eq`, so straight-line CSE computes the product ONCE and
    // shares it. Exactly TWO muls remain: the shared `(* a b)` + the `(* … 3)`. Before value-numbering
    // (node-identity only) this was THREE (each hand-written `(* a b)` emitted separately).
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (+ (* a b) (* (* a b) 3))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
        2,
        "value-equal `(* a b)` across ops is computed once (2 muls), got: {:?}",
        f.code
    );
}

#[test]
fn a_cse_slotted_operand_is_read_directly_not_recopied() {
    // A CSE-hoisted subexpression used as an ARITHMETIC OPERAND is read straight from its CSE slot —
    // `operand_src` honors the node's own slot, so no spurious copy into a fresh scratch slot. Before
    // this, `(+ (& x 7) (& x 7))` emitted `local.tee <cse> ; local.tee <scratch> ; local.get <scratch>`
    // (the operand path spilled the already-slotted value again); now it is `local.tee <cse> ;
    // local.get <cse> ; add` — identical to the explicit `(let ((y (& x 7))) (+ y y))`. Assert the two
    // lower to the SAME local count and the SAME emitted code.
    let lir = |src: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(src);
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code
    };
    let cse =
        lir("(module m (def (f (: x Int64)) (+ (& x 7) (& x 7))) (def (main) 0) (export main))");
    let via_let = lir(
        "(module m (def (f (: x Int64)) (let ((y (& x 7))) (+ y y))) (def (main) 0) (export main))",
    );
    assert_eq!(
        cse, via_let,
        "a CSE'd operand emits identically to an explicit let (no extra copy), got: {cse:?}"
    );
    // Concretely: exactly ONE `i64.and` (computed once) and the shared value read straight from its
    // CSE slot — `[get x ; const 7 ; and ; tee <cse> ; get <cse> ; add ; tee <ret>]`. The redundant
    // `local.tee/set <scratch>` that spilled the already-teed value is GONE (was a distinct middle
    // slot); the only two tees are the shared-value store and the result store.
    assert_eq!(
        cse.iter().filter(|i| **i == Lir::I64And).count(),
        1,
        "the shared `(& x 7)` is computed once, got: {cse:?}"
    );
    assert!(
        !cse.iter().any(|i| matches!(i, Lir::LocalSet(_))),
        "no redundant local.set spilling the already-teed CSE value, got: {cse:?}"
    );
}

#[test]
fn straight_line_cse_does_not_hoist_a_let_local_subexpression() {
    // A shared subexpression over a `let`-LOCAL — `(let ((k (+ a b))) (+ (* k k) (* k k)))` — must NOT
    // be hoisted before the body: the local `k`'s slot is only established when the `let` binding is
    // emitted INSIDE the body, so a hoisted `(* k k)` would read an unbound slot ("let-binding reference
    // has no local slot"). `is_cse_shareable` excludes `Core::LocalRef`, so a computation over a
    // let-local is left in place. This must COMPILE (a regression guard — an early value-numbering
    // version crashed here) and value-check. `(let ((k 7)) (+ (* k k) (* k k)))` = 49+49 = 98.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (let ((k (+ a b))) (+ (* k k) (* k k)))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    // The key assertion is that selection SUCCEEDS (no "no local slot" crash from a bad hoist).
    select_function(&mut db, body, &params, &layout)
        .expect("a let-local subexpression must not be hoisted before its binding");
}

#[test]
fn dominator_cse_hoists_a_condition_dominated_subexpression() {
    // `(if (> (* a b) 0) (* a b) (- 0 (* a b)))` — the `(* a b)` in the CONDITION is always evaluated
    // (it DOMINATES both branches), so all three value-equal `(* a b)` collapse to ONE computed slot
    // read in the cond + both branches. Exactly ONE `i64.mul` (was 3). The dominance requirement is
    // what makes hoisting across the `if` sound: the class runs on entry regardless of the branch taken.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (if (> (* a b) 0) (* a b) (- 0 (* a b)))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
        1,
        "a condition-dominated `(* a b)` is computed once and shared across cond+branches, got: {:?}",
        f.code
    );
}

#[test]
fn dominator_cse_does_not_hoist_a_branch_only_subexpression() {
    // `(if (> c 0) (* a b) (- 0 (* a b)))` — `(* a b)` appears ONLY in the two BRANCHES, never in the
    // (always-evaluated) condition, so it is NOT in the dominating frontier. Hoisting it would SPECULATE
    // the product (and, for a trapping op, its trap) onto the code path that runs before the branch is
    // chosen — unsound. So it must be left in place: exactly TWO `i64.mul` (one per branch), not one.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64) (: c Int64)) (if (> c 0) (* a b) (- 0 (* a b)))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code.iter().filter(|i| **i == Lir::I64Mul).count(),
        2,
        "a branch-only shared `(* a b)` (no dominating occurrence) is NOT hoisted, got: {:?}",
        f.code
    );
}

#[test]
fn cse_shares_a_repeated_collection_count() {
    // `(List.len xs)` is a TOTAL O(1) BORROWING scalar read (a `vec-len` runtime import — no rc change,
    // deterministic). Two identical counts of the same list param `(+ (List.len xs) (* (List.len xs) 3))`
    // are `core_eq` and dominate (straight-line body), so CSE computes the `vec-len` ONCE and shares it
    // → exactly ONE `vec-len` CallImport (was two). `xs` is a real PARAM (a list handle live up front),
    // so the read is well-formed at the hoist point. Selects `f` directly (its param is an i32 handle).
    let ast = crate::testkit::parse(
        "(module m (def (f (: xs (List Int64))) (+ ((. List len) xs) (* ((. List len) xs) 3))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code
            .iter()
            .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_VEC_LEN))
            .count(),
        1,
        "a repeated `(List.len xs)` is computed once and shared, got: {:?}",
        f.code
    );
}

#[test]
fn a_repeated_sum_payload_read_is_shared_by_cse() {
    // (match o ((Some x) (+ x x)) ((None) 0)) — the binder `x` resolves to a `Core::SumPayload` at
    // EACH occurrence, so `(+ x x)` names two DISTINCT SumPayload nodes. `core_eq` now recognizes
    // them as equal (same scrutinee + path), so the arith-CSE reads the payload ONCE
    // (`sum-payload ; get-int` a single time) into a slot and shares it for both `+` operands —
    // exactly as a repeated tuple/record field `(+ (. r x) (. r x))` already was. The match is kept
    // runtime by making `f` recursive on a fresh `(None)`.
    let ast = crate::testkit::parse(
        "(module m (def (f (: o (Option Int64)) (: acc Int64)) \
               (match o ((Some x) (f (None) (+ acc (+ x x)))) ((None) acc))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert_eq!(
        f.code
            .iter()
            .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_SUM_PAYLOAD))
            .count(),
        1,
        "the payload `x` is read exactly once and shared across `(+ x x)`, got: {:?}",
        f.code
    );
}

#[test]
fn doubling_add_collapses_the_overflow_guard_to_one_xor() {
    // (def (f (: a Int64)) (+ a a)) — both operands are the SAME source, so the signed-add guard
    // `((r^a)&(r^b))<0` with `b==a` collapses to `(r^a)<0`: ONE xor, no `and`, no second `r^b`.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64)) (+ a a)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code.iter().filter(|i| matches!(i, Lir::I64Xor)).count(),
        1,
        "(+ a a) guard is a single xor (`(r^a)<0`), got: {:?}",
        f.code
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I64And)),
        "the `& (r^b)` half is gone — x & x = x, got: {:?}",
        f.code
    );
}

#[test]
fn a_provably_in_range_arith_op_elides_its_overflow_guard() {
    let select = |src: &str| {
        let mut db = Db::load(crate::testkit::parse(src));
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code
    };
    // `(+ (& x 15) (& y 15))`: both operands ∈ [0,15], sum ∈ [0,30], fits Int64 → NO overflow guard
    // (no `((r^a)&(r^b))<0` sign test).
    let add = select(
        "(module m (def (f (: x Int64) (: y Int64)) (+ (& x 15) (& y 15))) (def (main) 0) (export main))",
    );
    assert!(
        !add.iter().any(|i| matches!(i, Lir::I64Xor))
            && !add.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a provably-in-range add drops its guard; got {add:?}"
    );
    // `(* (& x 15) 3)`: [0,15]×3 = [0,45], fits → NO const-multiplier bound check.
    let mul =
        select("(module m (def (f (: x Int64)) (* (& x 15) 3)) (def (main) 0) (export main))");
    assert!(
        !mul.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a provably-in-range mul drops its bound check; got {mul:?}"
    );
    // A full-range add (either operand unbounded) KEEPS its guard.
    let kept =
        select("(module m (def (f (: x Int64) (: y Int64)) (+ x y)) (def (main) 0) (export main))");
    assert!(
        kept.iter().any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "a full-range add keeps its overflow guard; got {kept:?}"
    );
    // A NARROW result whose interval EXCEEDS the type keeps its range-check: [0,200]+[0,200]=[0,400]
    // > UInt8 255.
    let narrow_over = select(
        "(module m (def (f (: x UInt8) (: y UInt8)) (+ (& x 200) (& y 200))) (def (main) 0) (export main))",
    );
    assert!(
        narrow_over
            .iter()
            .any(|i| matches!(i, Lir::IfIntegerOverflowEnd)),
        "an over-range narrow add keeps its range-check; got {narrow_over:?}"
    );
    // CHAINED: the range PROPAGATES through nested arith — the inner `(+ (& x 15) (& y 15))` bounds to
    // [0,30], so the OUTER `(+ … (& z 15))` sees [0,30]+[0,15]=[0,45] and BOTH adds elide their guard
    // (zero xor across the whole body).
    let chained = select(
        "(module m (def (f (: x Int64) (: y Int64) (: z Int64)) \
               (+ (+ (& x 15) (& y 15)) (& z 15))) (def (main) 0) (export main))",
    );
    assert!(
        !chained.iter().any(|i| matches!(i, Lir::I64Xor)),
        "both adds in a chain elide their guard via range propagation; got {chained:?}"
    );
    // A chain where a middle operand is UNBOUNDED (`y`) keeps BOTH guards.
    let chained_open = select(
        "(module m (def (f (: x Int64) (: y Int64) (: z Int64)) \
               (+ (+ (& x 15) y) (& z 15))) (def (main) 0) (export main))",
    );
    assert!(
        chained_open
            .iter()
            .filter(|i| matches!(i, Lir::I64Xor))
            .count()
            >= 2,
        "an unbounded operand in the chain keeps the guards; got {chained_open:?}"
    );
}

#[test]
fn a_guard_elided_arith_operand_leaves_its_result_on_the_stack_no_dead_store() {
    // When a checked arith's overflow guard is PROVABLY elided (result in range) AND the op is used as
    // an operand/argument (dest = Stack), the machine op's result is already on the stack — it must be
    // left there, NOT round-tripped through `local.set $r ; local.get $r` (which the peephole then
    // fuses to a `local.tee $r` INTO A SLOT NEVER READ — a dead store). `(g (- n 1))` under `n >= 2`
    // (from the branch refinement) elides the `(- n 1)` underflow guard, so the arg should be
    // `... i64.sub ; call` with no `local.tee`/`local.set` of a dead slot between the sub and the call.
    let ast = crate::testkit::parse(
        "(module m \
               (def (g (: n Int64)) (if (< n 2) n (+ (g (- n 1)) 1))) \
               (def (f (: x Int64)) (g x)) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("g").expect("g");
    let (params, body) = function_of(&mut db, "g");
    let code = select_function_of(&mut db, body, &params, &layout, Some(d))
        .expect("select")
        .code;
    // The `(- n 1)` argument: an `I64Sub` immediately followed by the `Call` (or a `return_call`), with
    // NO `LocalTee`/`LocalSet` in between (the guard-elided result flows straight into the call).
    let sub_ix = code
        .iter()
        .position(|i| matches!(i, Lir::I64Sub))
        .expect("the (- n 1) argument subtracts");
    let next = &code[sub_ix + 1];
    assert!(
        matches!(next, Lir::Call(_) | Lir::ReturnCall(_)),
        "a guard-elided (- n 1) argument flows straight into the call — no dead store between \
             the sub and the call; got next = {next:?} in {code:?}"
    );
}

#[test]
fn a_guard_elided_arith_emits_its_operands_inline_with_no_scratch_slots() {
    // `(+ (& x 7) (& y 7))`: both operands ∈ [0,7], sum ∈ [0,14], fits → the overflow guard AND the
    // narrow range-check are elided. With NO guard to re-read the operands or the result, each operand
    // is used EXACTLY ONCE (only the `i64.add` reads it), so a non-reusable operand need not be stashed
    // in a scratch slot: both masked operands emit straight onto the stack. The whole body declares ZERO
    // locals (before, each masked operand was `local.set` into a slot then reloaded, plus a dead `$r`).
    let ast = crate::testkit::parse(
        "(module m (def (f (: x Int64) (: y Int64)) (+ (& x 7) (& y 7))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.declared,
        Vec::<ValType>::new(),
        "a guard-elided masked add needs no scratch slots — operands emit inline; got {:?}",
        f.code
    );
    // The exact inline sequence: mask x, mask y, add — no `local.set`/`local.tee` anywhere.
    assert!(
        !f.code
            .iter()
            .any(|i| matches!(i, Lir::LocalSet(_) | Lir::LocalTee(_))),
        "no operand is stashed in a slot when the guard is elided; got {:?}",
        f.code
    );
    assert!(
        f.code.contains(&Lir::I64Add)
            && f.code.iter().filter(|i| matches!(i, Lir::I64And)).count() == 2,
        "the body is `(& x 7)` inline, `(& y 7)` inline, `i64.add`; got {:?}",
        f.code
    );
}

#[test]
fn distinct_add_operands_keep_the_two_xor_guard() {
    // (+ a b) with DISTINCT operands cannot collapse — both `r^a` and `r^b` are needed.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) (+ a b)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code.iter().filter(|i| matches!(i, Lir::I64Xor)).count(),
        2,
        "distinct operands keep both xors, got: {:?}",
        f.code
    );
}

#[test]
fn a_self_tail_recursive_function_compiles_to_a_loop() {
    // (def (f (: n Int64) (: acc Int64)) (if (= n 0) acc (f (- n 1) (+ acc 1)))) — the self-call is
    // in tail position (the `if`'s else branch), so `select_function_of` (given f's own def index)
    // compiles it as a LOOP: the body opens with `Lir::Loop`, the self-call updates the param slots
    // (`local.set`s) and `br`s back, and there is NO `ReturnCall`.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64) (: acc Int64)) \
               (if (= n 0) acc (f (- n 1) (+ acc 1)))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        matches!(f.code.first(), Some(Lir::Loop(_))),
        "a self-tail-recursive function body opens with a loop"
    );
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::Br(_))),
        "the self-tail-call branches back to the loop top"
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::ReturnCall(_))),
        "no return_call — the self-call became a loop iteration"
    );
}

#[test]
fn a_pass_through_parameter_elides_its_self_move_at_the_loop_back_edge() {
    // (def (go (: n Int64) (: k Int64) (: acc Int64)) (if (= n 0) acc (go (- n 1) k (+ acc k)))) —
    // `k` is re-passed UNCHANGED to its own slot. The back-edge parallel move is `set acc ; set n`
    // only: the `k` arg is neither pushed (no `local.get k` for it) nor stored (no `local.set k`),
    // since a self-move `k ← k` is a no-op. `k`'s slot is read only by `(+ acc k)`, not moved.
    let ast = crate::testkit::parse(
        "(module m (def (go (: n Int64) (: k Int64) (: acc Int64)) \
               (if (= n 0) acc (go (- n 1) k (+ acc k)))) (export go))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("go").expect("def go");
    let (params, body) = function_of(&mut db, "go");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    // Param slots are 0=n, 1=k, 2=acc. The back-edge stores only n and acc — NOT k. Count the
    // `local.set` into slot 1 (k): there must be none in the whole body (k is never re-stored).
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::LocalSet(1))),
        "the pass-through param k (slot 1) is never re-stored, got: {:?}",
        f.code
    );
    // The other two params ARE stored at the back-edge.
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::LocalSet(0)))
            && f.code.iter().any(|i| matches!(i, Lir::LocalSet(2))),
        "n and acc are still updated each iteration"
    );
}

#[test]
fn a_mutually_tail_recursive_pair_compiles_to_a_shared_loop() {
    // even/odd tail-call each other (same signature) — each compiles to ONE `loop` with a `which`
    // dispatch: the body opens with `Lir::Loop`, a cross-call sets `which` + `br`s back, and there
    // is NO `ReturnCall` (the mutual tail-call became a loop iteration, not a real call).
    let ast = crate::testkit::parse(
        "(module m (def (even (: n Int64)) (if (= n 0) true (odd (- n 1)))) \
               (def (odd (: n Int64)) (if (= n 0) false (even (- n 1)))) (export even))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("even").expect("def even");
    let (params, body) = function_of(&mut db, "even");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    // The loop is not the FIRST instruction (the `which` init precedes it), but it is present near
    // the top, the cross-calls `br`, and no `return_call` survives.
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::Loop(_))),
        "a mutually-tail-recursive member compiles to a loop, got: {:?}",
        f.code
    );
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::Br(_))),
        "the mutual tail-call branches back to the loop top"
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::ReturnCall(_))),
        "no return_call — the mutual tail-call became a loop iteration, got: {:?}",
        f.code
    );
}

#[test]
fn mutual_recursion_with_different_signatures_stays_return_call() {
    // `f(n)` tail-calls `g(n,k)` and vice-versa — DIFFERENT arities, so they can't share one set of
    // parameter slots. The shared-loop transform must decline (signature guard) and leave the mutual
    // tail-calls as `return_call` (still O(1) stack, just a real tail call, not a loop `br`).
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) (if (= n 0) 1 (g (- n 1) 2))) \
               (def (g (: n Int64) (: k Int64)) (if (= n 0) k (f (- n 1)))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::Loop(_))),
        "heterogeneous-signature mutual recursion is not merged into a loop, got: {:?}",
        f.code
    );
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::ReturnCall(_))),
        "the cross-call to a different-signature peer stays a return_call"
    );
}

#[test]
fn a_non_recursive_function_is_not_wrapped_in_a_loop() {
    // A plain `(+ a b)` — no self-call, so no loop wrapping even when the def index is supplied.
    let ast =
        crate::testkit::parse("(module m (def (f (: a Int64) (: b Int64)) (+ a b)) (export f))");
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::Loop(_))),
        "a non-recursive function is not wrapped in a loop"
    );
}

#[test]
fn a_dense_scalar_match_emits_a_br_table() {
    // A value-position match over ≥3 dense integer literals (0..4) + a wildcard emits a `br_table`
    // decision tree (and the enclosing `Block`s), not a linear `if (== k)` chain (no `I64Eq` probe).
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) \
               (let ((r (match n (0 100) (1 101) (2 102) (3 103) (4 104) (_ 999)))) r)) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "a dense scalar match emits a br_table, got: {:?}",
        f.code
    );
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::Block(_))),
        "the br_table is wrapped in dispatch blocks"
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I64Eq)),
        "a br_table dispatch has no linear per-arm equality probe, got: {:?}",
        f.code
    );
}

#[test]
fn a_dense_if_equality_chain_lifts_to_a_br_table() {
    // A nested `(if (= n k) …)` dispatch — the SAME integer switch a user could write as a `match`,
    // but spelled with chained `if`s — over ≥3 dense constants lifts to a `br_table`, not the O(n)
    // linear `if (== k)` cascade it would otherwise emit. This is the wasm-specific twin of the
    // `match` br_table (Rust gets the jump table from LLVM).
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) \
               (if (= n 0) 100 (if (= n 1) 101 (if (= n 2) 102 (if (= n 3) 103 999))))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "a dense if-equality chain lifts to a br_table, got: {:?}",
        f.code
    );
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::I64Eq)),
        "the lifted chain dispatches via the table, no linear per-arm I64Eq probe, got: {:?}",
        f.code
    );
}

#[test]
fn a_flipped_if_equality_chain_lifts_to_a_br_table() {
    // The `(= k n)` operand order (constant on the left) is recognized identically — equality is
    // symmetric, so `(= 0 n)` is the same probe as `(= n 0)`.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) \
               (if (= 0 n) 100 (if (= 1 n) 101 (if (= 2 n) 102 (if (= 3 n) 103 999))))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "a flipped-operand if-equality chain lifts to a br_table, got: {:?}",
        f.code
    );
}

#[test]
fn a_let_bound_if_equality_chain_lifts_to_a_br_table() {
    // The scrutinee may be a `let`-bound LocalRef, not only a bare parameter — the recognizer keys
    // on the binder StructId, which is stable for a kept let-binding. `(let ((y …)) (if (= y k) …))`
    // dispatches on `y` and lifts.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) \
               (let ((y (+ n 1))) (if (= y 0) 100 (if (= y 1) 101 (if (= y 2) 102 999))))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "a let-bound if-equality chain lifts to a br_table, got: {:?}",
        f.code
    );
}

#[test]
fn a_two_arm_if_equality_chain_does_not_lift_to_a_br_table() {
    // Below the ≥3-arm threshold the existing branchless-select / structured-if lowering is already
    // at least as good, so a 2-const chain is NOT lifted (no table, and no wasted match machinery).
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) (if (= n 0) 100 (if (= n 1) 101 999))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "a 2-arm if-equality chain is not lifted to a br_table, got: {:?}",
        f.code
    );
}

#[test]
fn a_mixed_variable_if_chain_does_not_lift() {
    // A chain whose links test DIFFERENT variables is not one integer dispatch — the recognizer must
    // stop at the first foreign-variable link (folding only the leading same-variable arms, and only
    // when ≥3 of them). Here just two `n`-arms precede a `k`-arm, so the whole chain stays plain
    // structured `if`s (no br_table) — proving a mixed chain never mis-collapses across variables.
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64) (: k Int64)) \
               (if (= n 0) 100 (if (= n 1) 101 (if (= k 2) 102 999)))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "a mixed-variable chain (only 2 same-var arms) does not lift, got: {:?}",
        f.code
    );
}

#[test]
fn a_sparse_if_equality_chain_does_not_emit_a_br_table() {
    // The if-chain lift routes through the match backend, which only emits a `br_table` for a DENSE
    // range (`try_emit_scalar_br_table`'s `span > 2*count || span > 256` gate). A SPARSE chain — ≥3
    // distinct consts but a huge span (0/1000/50000) — must fall back to the linear `i64.eq` cascade,
    // NOT a 50000-wide table. Pins the density discipline the lift inherits: a regression loosening the
    // gate would emit a gigantic (or invalid) table. The values stay correct either way (the linear
    // chain is exhaustive); this asserts only the SHAPE (no br_table, keeps per-arm equality probes).
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) \
               (if (= n 0) 100 (if (= n 1000) 101 (if (= n 50000) 102 999)))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "a sparse if-equality chain (huge span) does not emit a br_table, got: {:?}",
        f.code
    );
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::I64Eq)),
        "the sparse chain keeps its linear per-arm equality probes, got: {:?}",
        f.code
    );
}

#[test]
fn a_negative_constant_if_equality_chain_lifts_to_a_br_table() {
    // The chain constants may be NEGATIVE — the density gate + table index shift compute over the signed
    // range (`span = max - min + 1` with `min` negative). `(= n -2)/(-1)/0/1` is a dense span-4 range,
    // so it lifts; the shift `n - (-2)` maps -2→0. Pins that negative literals don't break the span math
    // (a regression treating the probe's bit pattern as unsigned would wrongly read a negative as huge).
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) \
               (if (= n -2) 10 (if (= n -1) 11 (if (= n 0) 12 (if (= n 1) 13 99))))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "a dense negative-constant if-equality chain lifts to a br_table, got: {:?}",
        f.code
    );
}

#[test]
fn an_if_equality_chain_stops_at_a_non_equality_link() {
    // The recognizer collects only leading `(= X k)` links on the SAME binder; the FIRST non-equality
    // condition (`(< n 10)`) ends the chain and becomes the DEFAULT arm's body. So `(= n 0)/(1)/(2)`
    // still lift to a br_table (≥3 dense consts) with the whole `(if (< n 10) …)` as the default. Pins
    // that a mid-chain comparison neither aborts the lift nor gets mis-collected as an equality probe (a
    // regression doing either would drop or mis-route the `< 10` arm — a wrong value).
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) \
               (if (= n 0) 100 (if (= n 1) 101 (if (= n 2) 102 (if (< n 10) 500 999))))) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "the leading 3 equality links lift to a br_table (the `< 10` link is the default), got: {:?}",
        f.code
    );
    // The `< 10` comparison survives in the default arm (a signed less-than), not dropped.
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::I64LtS)),
        "the non-equality `< 10` link is emitted in the default arm, got: {:?}",
        f.code
    );
}

#[test]
fn a_br_table_over_a_zero_based_range_skips_the_index_shift() {
    // A dense `br_table` normalizes the scrutinee to a 0-based table index via `scrutinee - min`. When
    // the covered range STARTS AT 0 — the common `(match x (0 …) (1 …) …)` shape — that shift is the
    // identity `x - 0`, so the `const 0 ; sub` is dead and skipped: the scrutinee IS the index. A
    // range NOT starting at 0 keeps the subtract. Assert the min=0 table has NO `I64Sub` while the
    // min=5 table has exactly one (the index shift).
    let lir = |src: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(src);
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, "f");
        select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code
    };
    let min0 = lir(
        "(module m (def (f (: x Int64)) (match x (0 10) (1 20) (2 30) (3 40) (_ 50))) (def (main) 0) (export main))",
    );
    assert!(
        min0.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "the min=0 match still uses a br_table, got: {min0:?}"
    );
    assert!(
        !min0.iter().any(|i| matches!(i, Lir::I64Sub)),
        "a 0-based range skips the `x - 0` index shift, got: {min0:?}"
    );
    // The wrap-aliasing guard (`idx >=u span → default`) is UNAFFECTED — a negative/huge i64 scrutinee
    // still routes to the default, so the out-of-range compare survives.
    assert!(
        min0.iter().any(|i| matches!(i, Lir::I64GeU)),
        "the out-of-range wrap guard is kept, got: {min0:?}"
    );
    let min5 = lir(
        "(module m (def (f (: x Int64)) (match x (5 10) (6 20) (7 30) (_ 40))) (def (main) 0) (export main))",
    );
    assert_eq!(
        min5.iter().filter(|i| matches!(i, Lir::I64Sub)).count(),
        1,
        "a non-zero-based range keeps its `x - min` index shift, got: {min5:?}"
    );
}

#[test]
fn an_exhaustive_sum_match_br_table_elides_the_dead_default_block() {
    // A 3-variant EXHAUSTIVE sum match (Sign: Neg/Zero/Pos, no wildcard) — the disc is provably in
    // [0,3), so the br_table's out-of-range default is dead. The LAST arm serves as the default:
    // `br_table [0, 1] default=2` (2 explicit targets, NOT 3), and there is no separate `$default`
    // block wrapping an `unreachable`. So the table's target list has `m-1 = 2` entries.
    let ast = crate::testkit::parse(
        "(module m (def (f (: s Sign)) \
               (let ((r (match s ((Neg) 10) ((Zero) 20) ((Pos) 30)))) r)) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    let table = f
        .code
        .iter()
        .find_map(|i| match i {
            Lir::BrTable(targets, default) => Some((targets.clone(), *default)),
            _ => None,
        })
        .expect("an exhaustive sum match emits a br_table");
    assert_eq!(
        table.0,
        vec![0, 1],
        "3 variants → 2 explicit targets (arms 0,1); the last arm is the default"
    );
    assert_eq!(
        table.1, 2,
        "the table default targets the last arm (disc 2)"
    );
    // No `unreachable` from a dead default (this match has no arithmetic guards, so ANY unreachable
    // would be the elided dead-default one).
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::Unreachable)),
        "no dead-default unreachable, got: {:?}",
        f.code
    );
}

#[test]
fn a_two_variant_sum_match_with_leaf_bodies_selects_branchlessly() {
    // A 2-variant sum (enum) match with cheap trap-free LEAF arm bodies is `(if (disc == d) A B)` — the
    // sum-discriminant twin of the scalar 2-arm select. `(match f (On 1) (Off 0))` → `1 ; 0 ;
    // <disc> ; i32.eqz ; select`, NOT an `if`/`else` block. Sound: a `Leaf` body is trap-free
    // (`is_select_arm`); a payload-reading arm (`SumPayload`) is NOT trap-free and keeps the `if` — see
    // `an_option_match_with_a_payload_reading_arm_keeps_its_if`.
    let ast = crate::testkit::parse(
        "(module m (type Flag On Off) \
               (def (rank (: f Flag)) (match f (Flag.On 1) (Flag.Off 0))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("rank").expect("rank");
    let (params, body) = function_of(&mut db, "rank");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        f.code.contains(&Lir::Select)
            && !f.code.iter().any(|i| matches!(i, Lir::If(_) | Lir::Else)),
        "a 2-variant enum match with leaf bodies selects branchlessly (no if/else): {:?}",
        f.code
    );
}

#[test]
fn an_option_match_with_a_payload_reading_arm_keeps_its_if() {
    // A 2-arm sum match whose arm READS the payload — `(match o ((Some v) (+ v 1)) ((None) 0))` — must
    // NOT become a branchless `select`: `select` evaluates BOTH arms, so it would read the `Some`
    // payload even when the value is `None` (a `SumPayload` on the wrong variant). `is_select_arm`
    // (via `is_trap_free`) excludes a `SumPayload` read, so the `if`/`else` decision-tree survives.
    let ast = crate::testkit::parse(
        "(module m \
               (def (f (: o (Option Int64))) (match o ((Some v) (+ v 1)) ((None) 0))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        !f.code.contains(&Lir::Select),
        "a payload-reading Option arm keeps the if (no speculative select): {:?}",
        f.code
    );
}

// (corpus companion: `05-compound-types.sexp` "a match arm reading two elements of a boxed payload
// tuple shares the sum-payload prefix" pins the runtime tree/list fold value with the prefix CSE.)
#[test]
fn a_match_arm_reading_two_payload_elements_computes_the_prefix_once() {
    // A `(Pair (tuple a b))` arm binds `a` = SumPayload{p, [Payload, Elem(0)]} and `b` = SumPayload{p,
    // [Payload, Elem(1)]}. Both re-walk the shared `sum-payload(p)` prefix — the per-arm-body prefix
    // CSE computes it ONCE into a slot, so the arm reads BOTH elements off the one `sum-payload` via
    // `arr-get`. Non-recursive so `select_function_of` needs no cross-function emission order. `sum`
    // reads a and b: exactly ONE `sum-payload` (the shared prefix) + TWO `arr-get` (a and b).
    // TWO variants so `Pair`'s payload is genuinely BOXED (a single-variant newtype erases the box,
    // so there is no `sum-payload` prefix to share).
    let ast = crate::testkit::parse(
        "(module m (type P (Pair (Tuple Int64 Int64)) Nil) \
               (def (sum (: p P)) (match p ((P.Pair (tuple a b)) (+ a b)) ((P.Nil) 0))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("sum").expect("sum");
    let (params, body) = function_of(&mut db, "sum");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert_eq!(
        f.code
            .iter()
            .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_SUM_PAYLOAD))
            .count(),
        1,
        "the arm's shared payload prefix is computed ONCE (1 sum-payload, not 2): {:?}",
        f.code
    );
    assert_eq!(
        f.code
            .iter()
            .filter(|i| matches!(i, Lir::CallImport(op) if *op == OP_ARR_GET))
            .count(),
        2,
        "the two tuple elements read via arr-get off the shared prefix: {:?}",
        f.code
    );
}

#[test]
fn a_two_arm_list_match_with_leaf_bodies_selects_branchlessly() {
    // A 2-arm list match — a LENGTH-test arm then a single unconditional cover — with cheap trap-free
    // LEAF bodies is `(if (len ⋈ k) A B)`, the list analogue of the scalar/sum 2-arm select.
    // `(match xs ((list) 0) ((list a .. r) 1))` dispatches on `len == 0` → `0 ; 1 ; (len==0) ; select`,
    // not an `if`/`else` block.
    let ast = crate::testkit::parse(
        "(module m (def (f (: xs (List Int64))) (match xs ((list) 0) ((list a .. r) 1))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        f.code.contains(&Lir::Select)
            && !f.code.iter().any(|i| matches!(i, Lir::If(_) | Lir::Else)),
        "a 2-arm list match with leaf bodies selects branchlessly (no if/else): {:?}",
        f.code
    );
}

#[test]
fn a_list_match_reading_an_element_binder_keeps_its_if() {
    // A 2-arm list match whose cons arm READS an element binder — `(match xs ((list) -1) ((list a .. r)
    // a))` — must NOT become a `select`: `select` evaluates BOTH arms, so it would read element 0 even
    // on an EMPTY list (a `SumPayload` out-of-bounds). `is_select_arm` (via `is_trap_free`) excludes a
    // `SumPayload`, so the length `if` survives.
    let ast = crate::testkit::parse(
        "(module m (def (f (: xs (List Int64))) (match xs ((list) -1) ((list a .. r) a))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        !f.code.contains(&Lir::Select),
        "an element-binder-reading list arm keeps the if (no speculative empty-list read): {:?}",
        f.code
    );
}

#[test]
fn a_sum_match_with_a_wildcard_keeps_its_default_block() {
    // A sum match with FEWER explicit arms than variants + a wildcard (Color: Red/Green/Blue + `_`
    // covering Yellow) DOES need a real default block — the table default routes the uncovered disc
    // there. So the br_table has all 3 explicit targets AND a distinct default depth (= 3), and the
    // default block exists.
    let ast = crate::testkit::parse(
        "(module m (type Color Red Green Blue Yellow) \
               (def (f (: c Color)) \
                 (let ((r (match c ((Red) 1) ((Green) 2) ((Blue) 3) (_ 9)))) r)) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    let table = f
        .code
        .iter()
        .find_map(|i| match i {
            Lir::BrTable(targets, default) => Some((targets.clone(), *default)),
            _ => None,
        })
        .expect("br_table");
    assert_eq!(
        table.0,
        vec![0, 1, 2],
        "3 explicit disc arms each get a target; the default is separate"
    );
    assert_eq!(
        table.1, 3,
        "the default routes past the 3 arms to the $default block"
    );
}

#[test]
fn a_sparse_scalar_match_keeps_the_linear_probe_chain() {
    // A sparse range (0 and 100 — span 101 ≫ 2·2) is NOT worth a jump table; it keeps the linear
    // `if (== k)` chain (an `I64Eq` probe, no `br_table`).
    let ast = crate::testkit::parse(
        "(module m (def (f (: n Int64)) \
               (let ((r (match n (0 1) (100 2) (7 3) (_ 0))) ) r)) (export f))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let d = db.def_by_name("f").expect("def f");
    let (params, body) = function_of(&mut db, "f");
    let f = select_function_of(&mut db, body, &params, &layout, Some(d)).expect("select");
    assert!(
        !f.code.iter().any(|i| matches!(i, Lir::BrTable(_, _))),
        "a sparse scalar match keeps the linear chain (no br_table), got: {:?}",
        f.code
    );
    assert!(
        f.code.iter().any(|i| matches!(i, Lir::I64Eq)),
        "the linear probe chain compares the scrutinee per arm"
    );
}

#[test]
fn peephole_fuses_set_then_get_of_the_same_local_into_tee() {
    // `local.set N ; local.get N` (store then immediately re-read the SAME local) → `local.tee N`.
    let mut code = vec![
        Lir::I64Add,
        Lir::LocalSet(2),
        Lir::LocalGet(2), // same local as the set → fuses
        Lir::LocalGet(0),
        Lir::I64Xor,
    ];
    peephole(&mut code);
    assert_eq!(
        code,
        vec![Lir::I64Add, Lir::LocalTee(2), Lir::LocalGet(0), Lir::I64Xor]
    );
}

#[test]
fn peephole_leaves_a_set_get_of_different_locals_alone() {
    // A `local.get` of a DIFFERENT local must NOT fuse (it is a genuine read of another value), and
    // a `local.set` not immediately followed by a matching `local.get` is untouched.
    let mut code = vec![
        Lir::LocalSet(3),
        Lir::LocalGet(4), // different local → no fuse
        Lir::LocalSet(5),
        Lir::I64Add, // set not followed by a get → no fuse
    ];
    let before = code.clone();
    peephole(&mut code);
    assert_eq!(code, before);
}

#[test]
fn peephole_does_not_fuse_across_a_block_boundary() {
    // A block marker (`End`) between the set and the get keeps them non-adjacent, so no fuse — a
    // `local.get` opening a different block never merges with a `local.set` closing another.
    let mut code = vec![Lir::LocalSet(2), Lir::End, Lir::LocalGet(2)];
    let before = code.clone();
    peephole(&mut code);
    assert_eq!(code, before);
}

#[test]
fn a_parameterized_comparison_selects_to_a_signed_compare() {
    // (def (lt (: a Int64) (: b Int64)) (< a b)) — a runtime signed comparison, result Bool (i32).
    // A comparison never overflows, so no scratch/guard — just push both and compare.
    let ast = crate::testkit::parse(
        "(module m (def (lt (: a Int64) (: b Int64)) (< a b)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "lt");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.code,
        vec![Lir::LocalGet(0), Lir::LocalGet(1), Lir::I64LtS]
    );
    assert!(f.declared.is_empty());
    assert_eq!(f.ret, Ty::Bool);
}

#[test]
fn equality_with_zero_selects_to_eqz() {
    // `(= n 0)` on a 64-bit param is `i64.eqz` (one instruction: push n, eqz) — NOT
    // `local.get 0 ; i64.const 0 ; i64.eq` (three). The zero operand is recognized at the compare
    // emit site; the commuted `(= 0 n)` folds the same way, and a NON-zero rhs keeps `i64.eq`.
    let check = |src: &str, name: &str, want: Vec<Lir>| {
        let ast = crate::testkit::parse(src);
        let mut db = Db::load(ast);
        let layout = layout_of(&mut db);
        let (params, body) = function_of(&mut db, name);
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        assert_eq!(f.code, want, "{src}");
    };
    check(
        "(module m (def (f (: n Int64)) (= n 0)) (def (main) 0) (export main))",
        "f",
        vec![Lir::LocalGet(0), Lir::I64Eqz],
    );
    // Commuted: `(= 0 n)` → the non-zero operand (n) then eqz.
    check(
        "(module m (def (f (: n Int64)) (= 0 n)) (def (main) 0) (export main))",
        "f",
        vec![Lir::LocalGet(0), Lir::I64Eqz],
    );
    // A ≤32-bit operand uses i32.eqz.
    check(
        "(module m (def (f (: n Int32)) (= n 0)) (def (main) 0) (export main))",
        "f",
        vec![Lir::LocalGet(0), Lir::I32Eqz],
    );
    // A NON-zero literal keeps the general equality (push both, i64.eq) — eqz is zero-only.
    check(
        "(module m (def (f (: n Int64)) (= n 5)) (def (main) 0) (export main))",
        "f",
        vec![Lir::LocalGet(0), Lir::ConstI64(5), Lir::I64Eq],
    );
}

#[test]
fn a_nested_checked_op_shares_scratch_minimally() {
    // (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) — a nested checked op. The outer
    // mul's LHS is the inner add; instead of computing the add into its OWN $r and copying that to
    // the mul's $a, the add is emitted with `ResultDest::Slot($a)` so its result store writes $a
    // directly (no `local.get $r_inner ; local.tee $a` copy, no separate $r_inner slot). Slots:
    // outer mul $a=3 (the inner add writes here), $b=c=slot 2 (a direct param, no scratch), $r=4;
    // the inner add reuses $a=3 as its own $r and its a,b are direct params → no scratch of its own.
    // So (before coalescing) only slots 3 and 4 are declared — 2 locals, down from 3 before the
    // dest-threading. AFTER coalescing with dead-param-slot reuse it drops to ONE declared: params
    // a,b,c are all read once (each dead after its read), so a scratch re-homes into a dead param
    // slot (`reuse_dead_param_slots`, plain-wasm) — the dest-threading win compounds with slot reuse.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(f.declared, vec![ValType::I64; 1]);
}

#[test]
fn a_nested_strength_reduced_multiply_writes_the_operand_slot_directly() {
    // (def (f (: a Int64)) (+ (* a 2) 1)) — `(* a 2)` strength-reduces to `a << 1` and is the LHS
    // operand of the enclosing `+`. Instead of computing the shift into its OWN $r and copying that
    // into the add's $a (`local.get $r_inner ; local.tee $a`, plus a dead $r_inner slot), the shift is
    // emitted with `ResultDest::Slot($a)` so its `local.set` IS the store into the add's operand slot —
    // exactly like the nested checked `+`/`-`/`*` path. The add's RHS is the inline constant `1` (no
    // scratch). So the shift's own $r is the add's $a slot, and (before local-slot coalescing) that
    // slot + the add's $r are declared: 2 locals, down from 3 before the dest-threading (the
    // eliminated copy freed a local). AFTER coalescing with dead-param-slot reuse it drops to ONE
    // declared: the sole param `a` is dead after its last read (the shift's operand + the overflow
    // check), so the add's $r is soundly re-homed into the dead param slot 0 (`reuse_dead_param_slots`,
    // plain-wasm) — the closing `LocalGet(0)` reads that re-homed result, not the param.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64)) (+ (* a 2) 1)) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert_eq!(
        f.declared,
        vec![ValType::I64; 1],
        "dest-threading + dead-param-slot reuse: the shift writes the add's operand slot directly \
             (no $r_inner copy) and the add's $r re-homes into the dead param slot → 1 declared; got {:?}",
        f.code
    );
    // The shift is present (strength reduction fired) and there is NO `i64.mul`.
    assert!(
        f.code.contains(&Lir::I64Shl) && !f.code.iter().any(|i| matches!(i, Lir::I64Mul)),
        "the `* 2` is a shift, not a mul; got {:?}",
        f.code
    );
    // No `local.get N ; local.tee M` handoff copy between the shift's result and the add's operand —
    // the shift writes the operand slot directly, so its result is consumed in place. (A `local.get`
    // immediately followed by `local.tee` was the copy the dest-threading removes.)
    let copy = f
        .code
        .windows(2)
        .any(|w| matches!(w, [Lir::LocalGet(_), Lir::LocalTee(_)]));
    assert!(
        !copy,
        "no get-then-tee handoff of the shift result into the add operand slot; got {:?}",
        f.code
    );
}

// ── value-heap H2d: Perceus — a kept heap binding constructs then DROPS ───────────────────────

#[test]
fn a_projection_only_tuple_folds_and_builds_no_heap() {
    // (def (f (: a Int64) (: b Int64)) (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) — `t` is ONLY
    // ever projected (never used as a whole value), so it does NOT need to exist on the heap: each
    // projection folds straight through to its element (the param), and the body is just `(+ a b)`.
    // No `arr-alloc`, no `box`/`arr-set`, no `drop` — a projection-only compound emits ZERO heap ops
    // (`should_keep_binding` does not keep a projection-only compound). The GENUINE heap-alloc →
    // escape → walk → drop (Perceus) path is exercised by the recursive-escape resource tests
    // (`a_recursive_runtime_tuple_escapes_to_the_host` + the `live-objects == 0` balance probe),
    // where the compound is returned WHOLE and must actually be built.
    let ast = crate::testkit::parse(
        "(module m (def (f (: a Int64) (: b Int64)) \
               (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        !f.code.contains(&Lir::CallImport("arr-alloc")),
        "a projection-only tuple must not be built on the heap"
    );
    assert!(
        !f.code.contains(&Lir::CallImport("drop")),
        "nothing is built, so nothing is dropped"
    );
    // It is exactly the checked add of the two params — the same code `(+ a b)` emits directly.
    assert!(
        f.code.contains(&Lir::I64Add) && f.code.contains(&Lir::LocalGet(0)),
        "the body folds to `(+ a b)` over the params"
    );
}

#[test]
fn a_scalar_let_binding_is_not_dropped() {
    // A scalar (`Int64`) `let` binding owns no heap cell, so NO drop is emitted for it — reclamation
    // is only for heap values. `(let ((s (+ a b))) (+ s s))` — `s` is a kept i64, never dropped.
    let ast = crate::testkit::parse(
        "(module m (def (g (: a Int64) (: b Int64)) \
               (let ((s (+ a b))) (+ s s))) (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "g");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        !f.code.contains(&Lir::CallImport("drop")),
        "a scalar binding owns no heap cell and must not be dropped"
    );
}

// ── value-heap: Map.lookup / Set.contains BORROW the key — a BORROWED String key is NOT dropped ──

#[test]
fn a_borrowed_string_map_lookup_key_is_not_dropped() {
    // A `Map.lookup` whose KEY is a BORROWED String — here a `String` PARAMETER the caller owns —
    // must NOT be dropped after the borrowing lookup: `map-lookup` reads the key without consuming it,
    // and dropping the param's reference would free a value the caller still holds (a use-after-free).
    // This is the ownership face of the two-live-matched-String-payloads MISCOMPILE: a tree-walker
    // looking up a node's OWN key AND its child's key (both live sum-payload String projections) had
    // the second borrowed key freed under its owner, flipping its comparison and dropping a per-node
    // decision (a silent wrong count). No `box`/`bytes-compact` runs for a String key (it is already a
    // handle, and a borrowed String is a flat leaf), so the un-owned key must be left to its owner.
    // BOTH the map and the key are BORROWED params the caller owns — so this body must drop NEITHER
    // (`map-lookup` borrows both). Using a param MAP (not an inline `Map.insert`) isolates the borrowed
    // -key concern from the owned-temporary-map reclaim (a fresh inline map IS an owned temporary the
    // emit now correctly drops — see `an_owned_temporary_map_lookup_map_is_reclaimed`).
    //
    // WARNING: EXACTLY ONE drop is now expected — but it is the OWNED OPTION SHELL, not the borrowed key/map.
    // `Map.lookup` returns a FRESH owned `Option` (Some boxes a scalar copy of the value; None is fresh);
    // in a TAIL match (this body's whole match is `pv`'s result) the wrapper-scrutinee shell reclaim
    // deep-drops that dead Option shell. Its payload is a SCALAR (Int64, copied out), so the deep drop
    // frees only the Option shell — never the borrowed `op` key (not inside the shell) nor `mm` (borrowed
    // by the lookup). The pre-reclaim assertion "drop NOTHING" was too strong (it predated the tail-shell
    // reclaim); the invariant that actually guards the two-live-matched-keys UAF is that the borrowed KEY
    // and MAP survive — which value-correctness + the heap-valued repeated-lookup probe confirm. So assert
    // the lookup emits and (the real guard) that at most the single owned-shell drop appears, NOT a drop
    // per borrowed operand.
    let ast = crate::testkit::parse(
        "(module m (def (pv (: mm (Map String Int64)) (: op String)) \
               (match (Map.lookup mm op) \
                 (((. Option Some) p) p) (((. Option None) _) 0))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "pv");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    let drops = f
        .code
        .iter()
        .filter(|i| matches!(i, Lir::CallImport("drop")))
        .count();
    assert!(
        drops <= 1,
        "at most the OWNED Option-shell reclaim may drop (a scalar-payload shell frees only itself); a \
             second drop would be the borrowed key/map freed under their owner (the two-live-keys UAF); got: {:?}",
        f.code
    );
    assert!(
        f.code.contains(&Lir::CallImport("map-lookup")),
        "the lookup must still emit"
    );
}

#[test]
fn an_owned_temporary_map_lookup_map_is_reclaimed() {
    // The COLLECTION-operand reclaim: a `Map.lookup` whose MAP is a fresh OWNED TEMPORARY (built inline,
    // used once) must be dropped after the borrowing lookup, or it leaks. WARNING: the drop must come AFTER the
    // value is dup'd out (the Some arm) — not right after `map-lookup` (that would free the value the
    // val-slot still borrows → UAF). Here the key is a constant (also owned → also dropped), so we get
    // ≥2 drops (key + map). Pins that the owned-temporary map is reclaimed.
    let ast = crate::testkit::parse(
        "(module m (def (f (: d Int64)) \
               (match (Map.lookup (Map.insert (map) \"a\" 1) \"a\") \
                 (((. Option Some) p) p) (((. Option None) _) 0))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    let drops = f
        .code
        .iter()
        .filter(|i| matches!(i, Lir::CallImport("drop")))
        .count();
    assert!(
        drops >= 2,
        "an owned-temporary map (built inline) AND its owned constant key must both be dropped after \
             the borrowing lookup (≥2 drops); got {drops}: {:?}",
        f.code
    );
}

#[test]
fn an_owned_string_map_lookup_key_is_dropped() {
    // The complement: a `Map.lookup` whose KEY is an OWNED temporary — a CONSTANT String literal,
    // which materializes a FRESH owned byte-leaf handle — MUST be dropped after the borrowing lookup,
    // or the leaf leaks. So exactly one `drop` (the owned key) is emitted. This pins that the ownership
    // gate did not over-correct into leaking every key.
    let ast = crate::testkit::parse(
        "(module m (def (f (: d Int64)) \
               (match (Map.lookup (Map.insert (map) \"a\" 1) \"a\") \
                 (((. Option Some) p) p) (((. Option None) _) 0))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        f.code.contains(&Lir::CallImport("drop")),
        "an owned constant-String lookup key must be dropped after the borrowing lookup, or it \
             leaks; got: {:?}",
        f.code
    );
}

#[test]
fn a_borrowed_string_set_contains_element_is_not_dropped() {
    // The `Set.contains` twin of `a_borrowed_string_map_lookup_key_is_not_dropped`: `set-contains`
    // BORROWS its element, so a BORROWED String element (a `String` param the caller owns) must NOT be
    // dropped after the membership probe — dropping it would free the caller's value.
    // BOTH the set and the element are BORROWED params — so drop NEITHER. Using a param SET (not an
    // inline `Set.of`) isolates the borrowed-element concern from the owned-temporary-set reclaim (a
    // fresh inline set IS an owned temporary the emit now correctly drops).
    let ast = crate::testkit::parse(
        "(module m (def (has (: s (Set String)) (: e String)) \
               (Set.contains s e)) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "has");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    assert!(
        !f.code.contains(&Lir::CallImport("drop")),
        "a borrowed String set-contains element AND a borrowed set param must not be dropped; \
             got: {:?}",
        f.code
    );
    assert!(
        f.code.contains(&Lir::CallImport("set-contains")),
        "the membership probe must still emit"
    );
}

// ── PANIC-SAFETY: a panic during a collect_dup_sites run must NOT leave DUP_OCCURRENCE_ORACLE = Some
// (a stale oracle would contaminate the next same-thread collect_dup_sites → a false early-prune → a
// dropped dup site → latent leak/UAF; Copilot PR#942). The OracleGuard's Drop restores the prior value
// on unwind. ──
#[test]
fn a_panic_during_collect_dup_sites_restores_the_oracle() {
    // Oracle starts clear.
    assert!(
        DUP_OCCURRENCE_ORACLE.with(|o| o.borrow().is_none()),
        "precondition: oracle clear before the run"
    );
    // Install a guard then panic while it is live — Drop must restore the prior (None).
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let index: HashMap<StructId, usize> = HashMap::new();
        let _g = OracleGuard::install((index, HashMap::new()));
        assert!(
            DUP_OCCURRENCE_ORACLE.with(|o| o.borrow().is_some()),
            "oracle is Some while the guard is live"
        );
        panic!("simulate a mark_binder_dups assertion firing mid-run");
    }));
    assert!(r.is_err(), "the closure must have panicked");
    assert!(
        DUP_OCCURRENCE_ORACLE.with(|o| o.borrow().is_none()),
        "the OracleGuard's Drop must restore the oracle to None on unwind — else a stale oracle \
             contaminates the next same-thread collect_dup_sites (Copilot PR#942)"
    );
}

// ── SUBSTRATE: the shared occurrence oracle (build_occurrence_bitsets) MUST agree bit-for-bit with the
// per-binder binder_occurs it replaces, at EVERY node — the O(N) traversal-share fold's correctness rests
// on this equivalence (a divergent bit would change WHICH dup sites mark = a Perceus soundness bug). ──
#[test]
fn occurrence_bitset_oracle_agrees_with_per_binder_binder_occurs() {
    // A multi-binder body with nested lets, a match, and shared/consumed heap uses — exercises the
    // union + memo + arm paths of the oracle.
    let ast = crate::testkit::parse(
        "(module m (def (g (: n Int64) (: p (List Int64))) \
               (let ((x1 (list n))) \
               (let ((x2 (List.push x1 n))) \
                 (+ (List.len x1) \
                    (+ (List.len x2) \
                       (+ (List.len p) (List.len (List.push p n)))))))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let _layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "g");
    // Track every retain-candidate binder (the same set collect_dup_sites uses) + the params.
    let mut binders: Vec<StructId> = Vec::new();
    collect_retain_candidate_binders(&mut db, body, &mut binders);
    for (p, _) in &params {
        if !binders.contains(p) {
            binders.push(*p);
        }
    }
    assert!(!binders.is_empty(), "the body must have tracked binders");
    let index: HashMap<StructId, usize> =
        binders.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let bitsets = build_occurrence_bitsets(&mut db, body, &index);
    // For every node the oracle memoized, every tracked binder's bit must equal binder_occurs(node, b).
    let nodes: Vec<StructId> = bitsets.keys().copied().collect();
    for node in nodes {
        let bits = bitsets.get(&node).unwrap().clone();
        for (i, &b) in binders.iter().enumerate() {
            let oracle = (bits[i / 64] >> (i % 64)) & 1 == 1;
            let mut cache: HashMap<StructId, bool> = HashMap::new();
            let reference = binder_occurs(&mut db, node, b, &mut cache);
            assert_eq!(
                oracle, reference,
                "occurrence bitset disagrees with binder_occurs at node {:?} binder {:?} \
                     (oracle={}, reference={}) — the traversal-share substrate is unsound",
                node, b, oracle, reference
            );
        }
    }
}

// ── CHARACTERIZATION: pin the `collect_dup_sites`/emit output over a MULTI-BINDER body — the golden the
// planned `collect_dup_sites` O(binders×body-nodes)→O(N) traversal-share refactor (Option-C, the
// sread-eval ~1360-def provider-emit cliff, see the v-compiler-perf finding memo) MUST preserve
// BYTE-IDENTICALLY. Three heap `List` lets, each pushed-into then len-read in the same `+` group. NOTE
// (empirically pinned): in THIS shape the current compiler emits ZERO retain-`dup`s — the `List.push`
// result is tee'd to a fresh slot + immediately len'd+dropped, and the later `List.len xi` reads xi's
// ORIGINAL slot, so no consume-then-reuse survives to force a retain (the doc's `(List.len (List.push e
// 9))` retain case doesn't fire for a fresh-inline-list binding here). The value of pinning it is that
// the O(N) refactor must keep this count at 0 (not spuriously START marking) AND keep the drop-count
// stable — a traversal-share bug that changed WHICH binders are visited would perturb both. The positive
// dup-emit path is covered end-to-end by the v-memory-safety #45 UAF battery (--ignored 63/0 +
// cad/sread-eval oracles), the required co-verify for any change to this pass. ──
#[test]
fn multi_binder_body_pins_dup_and_drop_counts_for_traversal_share_refactor() {
    let ast = crate::testkit::parse(
        "(module m (def (g (: n Int64)) \
               (let ((x1 (list n))) \
               (let ((x2 (list n))) \
               (let ((x3 (list n))) \
                 (+ (+ (List.len (List.push x1 n)) (List.len x1)) \
                    (+ (+ (List.len (List.push x2 n)) (List.len x2)) \
                       (+ (List.len (List.push x3 n)) (List.len x3)))))))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "g");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    let dups = f
        .code
        .iter()
        .filter(|op| **op == Lir::CallImport("dup"))
        .count();
    let drops = f
        .code
        .iter()
        .filter(|op| **op == Lir::CallImport("drop"))
        .count();
    // Golden pinned from the current (pre-refactor) emit: 0 retain-dups, 3 drops (one per push-result
    // temporary). The traversal-share refactor MUST reproduce both exactly.
    //
    // WARNING: WHAT drops==3 ENCODES: the 3 drops are the 3 `List.push` RESULT temporaries. The 3 `xi`
    // let-bindings themselves are NOT dropped (push BORROWS xi; the fresh `(list n)` is never
    // reclaimed) — a bounded 3-cell LEAK, the known borrowed-through-scope let/param-drop KNOWN-GAP
    // (the general Perceus param-drop pass is not yet implemented in this backend; a known gap tracked
    // by v-memory-safety). This golden CONSTANT-encodes that gap. So a FUTURE leak-fix that legitimately
    // reclaims the `xi` lets would move drops 3→6 — that is a leak IMPROVEMENT, NOT a traversal-share
    // regression. If this fires with drops==6 (not the fold's concern — the fold is pure traversal-share
    // and won't touch reclamation), re-baseline to 6 rather than treating it as a site-set drift.
    // (0 retain-dups is value-CORRECT here regardless: the RRB vector is PERSISTENT, so `List.push`
    // returns a fresh vector and the later `List.len xi` reads the intact original — persistence, not
    // rc, carries correctness, so no retain is needed. The dup count is the traversal-share invariant.)
    assert_eq!(
        dups, 0,
        "traversal-share refactor changed the retain-dup count (was 0); site-set drift = soundness risk: {:?}",
        f.code
    );
    assert_eq!(
        drops, 3,
        "traversal-share refactor changed the drop count (was 3): {:?}",
        f.code
    );
}

#[test]
fn set_to_list_drops_its_baked_descriptor_after_the_borrowing_op() {
    // `Set.to-list` bakes a shape descriptor as an owned `Bytes` (`bytes-alloc`/`bytes-set`) and passes
    // it to `set-to-list`, which only BORROWS it (the runtime reads it as an inspector; see
    // `op_set_to_list` — "BORROWS `s` and `desc`"). So the emit MUST `drop` that owned descriptor
    // temporary after the op, or every `Set.to-list` call leaks the descriptor cell. Pin that a `drop`
    // FOLLOWS the op (past the desc `local.get`).
    let ast = crate::testkit::parse(
        "(module m (def (f (: s (Set Int64))) (List.len (Set.to-list s))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    let to_list_at = f
        .code
        .iter()
        .position(|op| *op == Lir::CallImport("set-to-list"))
        .expect("the emit must call set-to-list");
    assert!(
        f.code[to_list_at + 1..].contains(&Lir::CallImport("drop")),
        "the baked descriptor Bytes is BORROWED by set-to-list, so a `drop` must follow the op to \
             reclaim the owned descriptor temporary; got: {:?}",
        f.code
    );
}

#[test]
fn map_to_list_drops_its_baked_descriptor_after_the_borrowing_op() {
    // The map companion: `map-to-list` likewise BORROWS the baked descriptor, so the emit must drop it
    // after the op.
    let ast = crate::testkit::parse(
        "(module m (def (f (: m (Map Int64 Int64))) (List.len (Map.to-list m))) \
               (def (main) 0) (export main))",
    );
    let mut db = Db::load(ast);
    let layout = layout_of(&mut db);
    let (params, body) = function_of(&mut db, "f");
    let f = select_function(&mut db, body, &params, &layout).expect("select");
    let to_list_at = f
        .code
        .iter()
        .position(|op| *op == Lir::CallImport("map-to-list"))
        .expect("the emit must call map-to-list");
    assert!(
        f.code[to_list_at + 1..].contains(&Lir::CallImport("drop")),
        "the baked descriptor Bytes is BORROWED by map-to-list, so a `drop` must follow the op; \
             got: {:?}",
        f.code
    );
}

// ── lgx1-fix part-2 fence narrowing (`result_reaches_binder_or_heapchild`) — the UAF-guard coverage the
//    corpus does NOT pin. PAIRWISE/PASCAL corpus cases protect the LEAK-FIX side (fence does NOT fire for a
//    fresh construction); these two pin the UAF-GUARD side (fence FIRES for a heap-child extraction of the
//    accumulator) + the extraction-set coverage. A future removal of an op from the extraction set would drop
//    the first test to false → under-cover → reintroduce the epilogue-deep-drop UAF, uncaught by the corpus. ──

#[test]
fn result_reaches_binder_fires_for_a_heap_child_extraction_of_the_param() {
    // A terminal that EXTRACTS a heap CHILD of the accumulator and returns it — `(. acc 0)` (a tuple
    // projection) over a `Tuple (List Int64) Int64` (the projected first element is a heap child of `acc`) — MUST fire the
    // predicate so the part-2 epilogue-drop fence FIRES; else the deep-drop of `acc`'s slot frees the escaped
    // child → UAF (the sread/tr3 axis-B; v-mem rc-trace control (a)). Pins ListAt/SumExpect-reaching-binder
    // coverage: removing an extraction op from `result_reaches_binder_or_heapchild` would flip this to false.
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (g (: acc (Tuple (List Int64) Int64))) (. acc 0)) (def (main) 0) (export main))",
    ));
    let (params, body) = function_of(&mut db, "g");
    let acc = params[0].0;
    assert!(
        result_reaches_binder_or_heapchild(&mut db, body, acc),
        "a terminal extracting + returning a heap child of the param (. acc 0) MUST \
         fire the fence — else the epilogue deep-drop of acc frees the escaped child (UAF)"
    );
}

#[test]
fn result_reaches_binder_does_not_fire_for_a_fresh_construction_consuming_the_param() {
    // The other side of the narrowing: a FRESH construction that merely CONSUMES the accumulator —
    // `(List.push acc 9)` — is NOT an extraction (it builds a fresh owned list, not a live view into `acc`),
    // so the predicate must NOT fire → the fence does NOT suppress → the fresh owned result is reclaimed (the
    // PAIRWISE/PASCAL leak-fix). Pins that the fence stays narrow: a change making a fresh construction fire
    // the predicate would re-introduce the over-suppress leak the fix removed.
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (h (: acc (List Int64))) (List.push acc 9)) (def (main) 0) (export main))",
    ));
    let (params, body) = function_of(&mut db, "h");
    let acc = params[0].0;
    assert!(
        !result_reaches_binder_or_heapchild(&mut db, body, acc),
        "a fresh construction consuming the param (List.push acc 9) is NOT an extraction → must NOT fire the \
         fence (else the fresh owned result over-suppresses = the PAIRWISE/PASCAL leak)"
    );
}
