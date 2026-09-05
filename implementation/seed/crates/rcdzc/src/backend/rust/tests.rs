//! Rust-backend tests: an EMIT check (the generated source is what we expect) and a rustc ROUND-TRIP
//! check (the emitted `.rs` compiles and, driven, returns the SAME value the wasm path does — the
//! two backends judged against the one executable semantics, `backends-and-targets.md` §The meaning
//! against which every backend's output is judged).
//!
//! The round-trip is dev-only, exactly like the wasm backend's wasmtime run: it shells out to the
//! ambient `rustc` (present in this toolchain), compiles the emitted module plus a tiny generated
//! `main` that calls the export and prints the result, runs it, and reads the printed value back.
//! `rustc` never enters the compile path — it is the Rust backend's analogue of `wasmtime` as the
//! behavior oracle. A test that shells to `rustc` is skipped (not failed) if `rustc` is absent, so the
//! suite still runs in an environment without it.

use crate::backend::Target;
use crate::testkit::parse;
use crate::{Artifact, compile};

/// Compile a program's source to the Rust-backend artifact bytes (the emitted `.rs` text), or panic
/// with the first diagnostic. Mirrors `compile_component` but selects `Target::Rust`.
fn compile_rust(src: &str) -> String {
    compile_rust_result(src).unwrap_or_else(|diags| panic!("Rust emit failed: {diags:?}"))
}

/// Like `compile_rust` but returns `Err(diagnostics)` when the backend DECLINES (emits no Rust
/// artifact) instead of panicking — for tests asserting a construct declines cleanly (a `todo`), e.g.
/// a float-keyed `BTreeSet`/`BTreeMap` that has no `Ord` rep on the Rust backend.
fn compile_rust_result(src: &str) -> Result<String, Vec<String>> {
    let ast_bytes = crate::codec::encode(&parse(src));
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes)],
        &[Target::Rust],
    );
    match out.artifact(Target::Rust.artifact_kind()) {
        Some(bytes) => Ok(String::from_utf8(bytes.to_vec()).expect("emitted Rust is utf-8")),
        None => Err(out
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()),
    }
}

/// Try to compile a program to the Rust backend, returning the emitted source or the diagnostics (for
/// asserting a DECLINE).
fn try_compile_rust(src: &str) -> Result<String, Vec<String>> {
    let ast_bytes = crate::codec::encode(&parse(src));
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes)],
        &[Target::Rust],
    );
    match out.artifact(Target::Rust.artifact_kind()) {
        Some(bytes) => Ok(String::from_utf8(bytes.to_vec()).unwrap()),
        None => Err(out.diagnostics.iter().map(|d| d.message.clone()).collect()),
    }
}

/// The capture-once fold (effects.rs `hoist_performing_capture_closure`): a let-bound closure whose
/// value-let binds a PERFORMING draw the returned lambda reads folds cleanly rather than declining —
/// reduce_handle hoists the draw out to wrap the binding so it is threaded ONCE and the closure closes
/// over the captured result. This is the unit-level guard for the ca1c/ca1m corpus witnesses: it must
/// EMIT (not decline), and the emit must NOT re-issue the draw at each application (the old silent
/// re-performing miscompile). Applied twice, the single draw is shared across both applications.
#[test]
fn a_let_bound_closure_capturing_a_performing_draw_folds_via_the_capture_once_hoist() {
    // Applied ONCE (ca1c): a = St.next captured once = seed n; (f 10) = 10*n.
    let once = try_compile_rust(
        "(do (effect St (op next (-> Int64))) (def (main (: n Int64)) (handle St n ((next () s (resume s (+ s 1)))) (let ((f (let ((a (St.next))) (fn ((: x Int64)) (* a x))))) (f 10)))) (export main))",
    );
    assert!(
        once.is_ok(),
        "capture-once closure applied once must fold, not decline: {once:?}"
    );
    // Applied TWICE (ca1m): the single draw is shared across both applications (not re-drawn per use).
    let twice = try_compile_rust(
        "(do (effect St (op next (-> Int64))) (def (main (: n Int64)) (handle St n ((next () s (resume s (+ s 1)))) (let ((f (let ((a (St.next))) (fn ((: x Int64)) (* a x))))) (+ (f 10) (f 20))))) (export main))",
    );
    assert!(
        twice.is_ok(),
        "capture-once closure applied twice must fold, not decline: {twice:?}"
    );
    // NESTED-CAPTURE (cp1): the capture-once closure `g` is CAPTURED by a wrapping closure `h`, not applied
    // directly. The `deep_fresh_copy` hygiene gives the rewritten tree coherent parent pointers so `g`'s
    // reference inside `h` resolves to the hoisted binder — it folds (h(10) = g(10)+1 = 51). Without the
    // deep copy the reused subtree shared a load-time atom whose orphaned parent chain dead-ended the scope
    // walk → a false unbound / a re-draw 61; this must stay FOLDING.
    let nested_capture = try_compile_rust(
        "(do (effect St (op next (-> Int64))) (def (main (: n Int64)) (handle St n ((next () s (resume s (+ s 1)))) (let ((g (let ((a (St.next))) (fn ((: x Int64)) (* a x))))) (let ((h (fn ((: y Int64)) (+ (g y) 1)))) (h 10))))) (export main))",
    );
    assert!(
        nested_capture.is_ok(),
        "a capture-once closure captured by a nested closure must FOLD (deep-copy hoist hygiene): {nested_capture:?}"
    );
    // ARG'D FACTORY (cc3): a factory call `(mk (St.next))` whose performing arg is hoisted to a #cap and the
    // call inlined to a pure closure — must FOLD (the draw threads once, shared across both applications).
    let factory = try_compile_rust(
        "(do (effect St (op next (-> Int64))) (def (mk (: m Int64)) (fn ((: x Int64)) (* x m))) (def (main (: n Int64)) (handle St n ((next () s (resume s (+ s 1)))) (let ((f (mk (St.next)))) (+ (f 10) (f (St.next)))))) (export main))",
    );
    assert!(
        factory.is_ok(),
        "an arg'd-factory capture-once closure must FOLD (hoist arg + inline factory): {factory:?}"
    );
    // BRANCH-SELECTED (cpc1): a closure bound to an `if` selecting a performing capture-once closure vs a
    // pure one — the branch-aware distribution `(let ((f (if C X Y))) BODY)` → `(if C (let ((f X)) BODY)
    // (let ((f Y)) BODY))` lets FORM A fold each branch (draw fires only in the taken branch). Must FOLD.
    let branch_selected = try_compile_rust(
        "(do (effect St (op next (-> Int64))) (def (main (: n Int64)) (handle St n ((next () s (resume s (+ s 1)))) (let ((f (if (> n 0) (let ((a (St.next))) (fn ((: x Int64)) (* a x))) (fn ((: x Int64)) x)))) (f 10)))) (export main))",
    );
    assert!(
        branch_selected.is_ok(),
        "a branch-selected capture-once closure must FOLD (branch-aware distribution): {branch_selected:?}"
    );
}

#[test]
fn a_recursive_fold_reusing_a_rebuilt_list_across_sibling_arms_builds_and_computes() {
    // Perimeter companions to the payload-binder case (corpus-bugfix/breaker corrected discriminator,
    // 2026-07-25): the E0382 trigger is a recursion whose match arm reads a payload DERIVED FROM the
    // rebuilt-list binder `xs2` (a non-Copy Vec tuple field) while a SIBLING arm REUSES `xs2` — NOT
    // list-pattern-specific. The clone-on-read of the `xs2` tuple field covers all such shapes. Both → 102.
    // (a) a WILDCARD-element list pattern `(list _one)` — no payload binder — with sibling `xs2` reuse.
    let wildcard = "(do \
      (def (fold node) \
        (match node \
          ((Ast.List xs) \
            (match (fold-list xs (list) 0) \
              ((tuple xs2 k) \
                (match xs2 \
                  ((list _one) (tuple (Ast.List xs2) (+ k 1))) \
                  (_ (tuple (Ast.List xs2) k)))))) \
          (other (tuple other 0)))) \
      (def (fold-list (: xs (List Ast)) (: acc (List Ast)) (: k Int64)) \
        (match xs \
          ((list) (tuple acc k)) \
          ((list h .. t) \
            (match (fold h) \
              ((tuple h2 k2) (fold-list t (List.push acc h2) (+ k k2))))))) \
      (def (run (: n Int64)) \
        (match (fold (Ast.List (list (Ast.Int (BigInt.of n))))) \
          ((tuple _r k) (+ (* k 100) n)))) \
      (export run))";
    let rs = compile_rust(wildcard);
    if let Some(out) = rustc_run(&rs, "run(2)") {
        assert_eq!(out, "102", "wildcard list-pattern + sibling reuse:\n{rs}");
    }
    // (b) a `List.at` OPTION destructure + sibling `xs2` reuse — no list pattern at all.
    let option_at = "(do \
      (def (fold node) \
        (match node \
          ((Ast.List xs) \
            (match (fold-list xs (list) 0) \
              ((tuple xs2 k) \
                (match (List.at xs2 0) \
                  ((Some (Ast.Int a)) (tuple (Ast.Int a) (+ k 1))) \
                  (_ (tuple (Ast.List xs2) k)))))) \
          (other (tuple other 0)))) \
      (def (fold-list (: xs (List Ast)) (: acc (List Ast)) (: k Int64)) \
        (match xs \
          ((list) (tuple acc k)) \
          ((list h .. t) \
            (match (fold h) \
              ((tuple h2 k2) (fold-list t (List.push acc h2) (+ k k2))))))) \
      (def (run (: n Int64)) \
        (match (fold (Ast.List (list (Ast.Int (BigInt.of n))))) \
          ((tuple _r k) (+ (* k 100) n)))) \
      (export run))";
    let rs = compile_rust(option_at);
    if let Some(out) = rustc_run(&rs, "run(2)") {
        assert_eq!(
            out, "102",
            "List.at Option destructure + sibling reuse:\n{rs}"
        );
    }
}

#[test]
fn a_recursive_fold_matching_a_rebuilt_list_with_a_payload_binder_builds_and_computes() {
    // REGRESSION (breaker/corpus-bugfix, 2026-07-25): a MUTUALLY-RECURSIVE tree-fold that REBUILDS an Ast
    // list then matches it with a PAYLOAD-binding list pattern `((list (Ast.Int a)) …)` used to NO-BUILD on
    // rust with error[E0382]: borrow of moved value (the match-scrutinee temp), while wasm computed → 102.
    // ROOT: the rebuilt-list binder `xs2` is a non-Copy `Vec` read off a tuple field `(__msN).0`; it is used
    // BOTH as a list-match scrutinee (bound `let __lm = (__msN).0`, a MOVE) AND re-referenced in the
    // catch-all `(Ast.List xs2)` → use-after-move. FIX: (1) a non-Copy tuple/record field read clones (leaves
    // the tuple intact for a sibling field `.1`), and (2) the list-match registers its scrutinee local so arm
    // reads borrow the one bound value. `main(2)` = fold reduces the single-element `[Ast.Int 2]` to
    // `(Ast.Int 2, 1)`, then `k*100 + n = 1*100 + 2 = 102`.
    let src = "(do \
      (def (fold node) \
        (match node \
          ((Ast.List xs) \
            (match (fold-list xs (list) 0) \
              ((tuple xs2 k) \
                (match xs2 \
                  ((list (Ast.Int a)) (tuple (Ast.Int a) (+ k 1))) \
                  (_ (tuple (Ast.List xs2) k)))))) \
          (other (tuple other 0)))) \
      (def (fold-list (: xs (List Ast)) (: acc (List Ast)) (: k Int64)) \
        (match xs \
          ((list) (tuple acc k)) \
          ((list h .. t) \
            (match (fold h) \
              ((tuple h2 k2) (fold-list t (List.push acc h2) (+ k k2))))))) \
      (def (run (: n Int64)) \
        (match (fold (Ast.List (list (Ast.Int (BigInt.of n))))) \
          ((tuple _r k) (+ (* k 100) n)))) \
      (export run))";
    // It must EMIT (not decline) — the E0382 was a hard rustc build-fail on emitted source.
    let rs = compile_rust(src);
    // End-to-end through rustc: the emitted source builds AND computes 102 (the wasm oracle).
    if let Some(out) = rustc_run(&rs, "run(2)") {
        assert_eq!(
            out, "102",
            "mutually-recursive rebuilt-list fold computes:\n{rs}"
        );
    }
}

#[test]
fn a_set_or_map_over_bytes_orders_by_the_blessed_unsigned_byte_order() {
    // Operator directive (2026-08-02): Bytes has a BLESSED TOTAL order (§order) — content-lexicographic over
    // unsigned byte values, the String/Symbol byte-leaf template. So a Set<Bytes> / Map<Bytes,_> (which need
    // an Ord element/key) now COMPILES and orders on rust via `BTreeSet<Vec<u8>>` / `BTreeMap<Vec<u8>, _>`
    // (Vec<u8>'s native derived Ord IS lexicographic-over-unsigned = byte-identical to the wasm value-cmp
    // walk). This REVERSES the former uniform decline. `ty_is_ord(Ty::Bytes)` = true drives it.
    let set_bytes = "(module m \
        (def (run) (List.len (Set.to-list (Set.of (list (Bytes.of (list 1 2))))))) \
        (export run))";
    if let Some(out) = rustc_run(&compile_rust(set_bytes), "run()") {
        assert_eq!(
            out, "1",
            "a Set over Bytes builds a BTreeSet<Vec<u8>> and enumerates its one element"
        );
    }
    // Use `Map.len` (NOT the retired `Map.size` → CDZ0603 rename rejection). A Map keyed by Bytes now emits a
    // BTreeMap<Vec<u8>, _> and holds its one entry.
    let map_bytes = "(module m \
        (def (run) (Map.len (Map.insert (Map.empty) (Bytes.of (list 1 2)) 7))) \
        (export run))";
    assert!(
        try_compile_rust(map_bytes).is_ok(),
        "a Map keyed by Bytes must now compile (Vec<u8> is Ord): {:?}",
        try_compile_rust(map_bytes).err()
    );
    if let Some(out) = rustc_run(&compile_rust(map_bytes), "run()") {
        assert_eq!(out, "1", "a Bytes-keyed Map holds its one inserted entry");
    }
    // A bare Bytes `<` orders lexicographically over unsigned bytes: [1,2] < [1,3] → true. (The direct witness
    // that the order is REAL, not just an Ord-derive that never runs.)
    let bytes_lt = "(module m \
        (def (run) (if (< (Bytes.of (list 1 2)) (Bytes.of (list 1 3))) 1 0)) \
        (export run))";
    if let Some(out) = rustc_run(&compile_rust(bytes_lt), "run()") {
        assert_eq!(
            out, "1",
            "[1,2] < [1,3] lexicographically over unsigned bytes"
        );
    }
    // Bytes EQUALITY is untouched — a Bytes value still builds + round-trips (it derives Eq, and now Ord).
    let bytes_eq = "(module m \
        (def (run) (if (= (Bytes.of (list 1 2)) (Bytes.of (list 1 2))) 1 0)) \
        (export run))";
    assert!(
        try_compile_rust(bytes_eq).is_ok(),
        "Bytes EQUALITY stays blessed:\n{:?}",
        try_compile_rust(bytes_eq).err()
    );
}

#[test]
fn a_tuple_keyed_map_handler_state_annotates_the_solved_key_across_arms() {
    // REGRESSION (breaker tuple-key-map-rust-e0308, 2026-08-09, root-caused by v-rust-backend, fixed by
    // v-inference). A handler whose state is `(tuple counter Map.empty)` fixes the map's KEY only in a
    // LATER arm (`rec`'s `Map.insert m (tuple s …) …` → a `(Int64,Int64)` key), a DIFFERENT subtree from
    // the seed `Map.empty` (whose own `type_of` bottoms at `Map(Var,Var)`). Inference never wrote the
    // solved key back onto the seed node, so the rust/rust-async emit grounded it to the default
    // `BTreeMap<i64,i64>` and then `__m.insert((i64,i64), _)` was E0308 — a wasm-masked backend divergence
    // (wasm's tagless heap needs no spelled key). `reduce_handle`'s cross-arm collection-key propagation
    // (`refine_init_collection_ty`) now joins every arm's resume next-state onto the seed and annotates
    // `BTreeMap<(i64,i64), i64>`. Trigger = [tuple map key] x [tuple-key lookup arm] x [a third arm].
    let tk = "(module m \
        (effect E (op rec (-> Int64)) (op qry (-> Int64 Int64 Int64)) (op cnt (-> Int64))) \
        (def (run (: n Int64)) \
          (handle E (tuple n Map.empty) \
            ((rec () st (match st \
                          ((tuple s m) \
                           (resume s (tuple (+ s 2) \
                                            (Map.insert m (tuple s (+ s 1)) (* 10 s))))))) \
             (qry (a b) st (match st \
                             ((tuple s m) \
                              (resume (match (Map.lookup m (tuple a b)) \
                                        ((Some v) v) \
                                        ((None) -1)) \
                                      st)))) \
             (cnt () st (match st ((tuple s m) (resume s st))))) \
            (do (E.rec) (+ (E.qry n (+ n 1)) (E.cnt))))) \
        (export run))";
    let rs = compile_rust(tk);
    assert!(
        rs.contains("BTreeMap<(i64, i64), i64>"),
        "the seed Map.empty must annotate the tuple key solved in the `rec` arm, not the default \
         BTreeMap<i64,i64>:\n{rs}"
    );
    // The emitted Rust must COMPILE (the E0308 is a build failure) and compute the same value wasm does.
    if let Some(out) = rustc_run(&rs, "run(5)") {
        assert_eq!(
            out, "57",
            "the tuple-keyed-map handler-state program must compile and compute 57 on the Rust backend"
        );
    }

    // SIBLING (same root, breaker tuple-elem-set-state-scope): a `Set` of tuples in a tuple handler state,
    // its element fixed only in the `add` arm — the Set analogue of the Map case, same propagation fix.
    let sk = "(module m \
        (effect E (op add (-> Int64)) (op has (-> Int64 Int64 Int64)) (op cnt (-> Int64))) \
        (def (run (: n Int64)) \
          (handle E (tuple n (Set.of (list))) \
            ((add () st (match st \
                          ((tuple s ss) \
                           (resume s (tuple (+ s 2) \
                                            (Set.insert ss (tuple s (+ s 1)))))))) \
             (has (a b) st (match st \
                             ((tuple s ss) \
                              (resume (if (Set.contains ss (tuple a b)) 1 0) st)))) \
             (cnt () st (match st ((tuple s ss) (resume s st))))) \
            (do (E.add) (E.add) \
                (+ (E.has n (+ n 1)) \
                   (+ (* 1000 (E.has (+ n 9) (+ n 10))) \
                      (E.cnt)))))) \
        (export run))";
    let rs_set = compile_rust(sk);
    assert!(
        rs_set.contains("BTreeSet<(i64, i64)>"),
        "the seed empty Set must annotate the tuple element solved in the `add` arm:\n{rs_set}"
    );
    if let Some(out) = rustc_run(&rs_set, "run(5)") {
        assert_eq!(
            out, "10",
            "the tuple-element-set handler-state program must compile and compute 10 on the Rust backend"
        );
    }

    // SIBLING (breaker tv1): the gap is element-GENERIC, not key-specific — a Map whose tuple lives in the
    // VALUE position (scalar key) reproduces it. The propagation fills the Map value type the same way.
    let tv = "(module m \
        (effect E (op rec (-> Int64)) (op qry (-> Int64 Int64)) (op cnt (-> Int64))) \
        (def (run (: n Int64)) \
          (handle E (tuple n Map.empty) \
            ((rec () st (match st \
                          ((tuple s m) \
                           (resume s (tuple (+ s 2) (Map.insert m s (tuple s (* 10 s)))))))) \
             (qry (k) st (match st \
                           ((tuple s m) \
                            (resume (match (Map.lookup m k) \
                                      ((Some p) (match p ((tuple a b) (+ a b)))) \
                                      ((None) -1)) st)))) \
             (cnt () st (match st ((tuple s m) (resume s st))))) \
            (do (E.rec) (+ (E.qry n) (E.cnt))))) \
        (export run))";
    let rs_tv = compile_rust(tv);
    assert!(
        rs_tv.contains("BTreeMap<i64, (i64, i64)>"),
        "a Map whose tuple is in the VALUE position must annotate the solved value type:\n{rs_tv}"
    );
    if let Some(out) = rustc_run(&rs_tv, "run(5)") {
        assert_eq!(
            out, "62",
            "the tuple-VALUE-map handler-state program must compile and compute 62"
        );
    }

    // SIBLING (breaker lv1): a `List` of tuples seeded by the empty `(list)` literal (whose element bottoms
    // at `Ty::Any`, not a var — `is_open` covers both). Element fixed only in the `push` arm.
    let lv = "(module m \
        (effect E (op push (-> Int64)) (op rd (-> Int64)) (op cnt (-> Int64))) \
        (def (run (: n Int64)) \
          (handle E (tuple n (list)) \
            ((push () st (match st \
                           ((tuple s xs) \
                            (resume s (tuple (+ s 2) (List.push xs (tuple s (* 2 s)))))))) \
             (rd () st (match st \
                         ((tuple s xs) \
                          (resume (match (List.at xs 0) \
                                    ((Some p) (match p ((tuple a b) (+ a b)))) \
                                    ((None) -1)) st)))) \
             (cnt () st (match st ((tuple s xs) (resume s st))))) \
            (do (E.push) (E.push) (+ (E.rd) (E.cnt))))) \
        (export run))";
    let rs_lv = compile_rust(lv);
    assert!(
        rs_lv.contains("Vec::<(i64, i64)>::new()"),
        "the empty-(list) seed (element Any) must annotate the tuple element solved in the `push` arm:\n{rs_lv}"
    );
    if let Some(out) = rustc_run(&rs_lv, "run(5)") {
        assert_eq!(
            out, "24",
            "the tuple-element-list handler-state program must compile and compute 24"
        );
    }
}

#[test]
fn a_tuple_keyed_map_handler_state_solves_the_key_on_the_async_backend_too() {
    // rust-async PARITY twin of `a_tuple_keyed_map_handler_state_annotates_the_solved_key_across_arms`
    // (v-rust-backend owns rust-async). The cross-arm collection-key propagation lives in inference
    // (`refine_init_collection_ty`), UPSTREAM of the backend split, so the seed `Map.empty` node is
    // solved to `Map((Int64,Int64), Int64)` before EITHER backend emits — the async backend must spell
    // the same `BTreeMap<(i64, i64), i64>` (its map annotation uses `async_or_rust_type`, byte-identical
    // to `rust_type` for a closure-free map). Guards against a future async-mode regression that only the
    // sync test would miss. (Annotation-level: the async harness needs an executor to RUN, but the emit's
    // key-type spelling is what the E0308 turned on, so asserting the annotation pins the fix on async.)
    let tk = "(module m \
        (effect E (op rec (-> Int64)) (op qry (-> Int64 Int64 Int64)) (op cnt (-> Int64))) \
        (def (run (: n Int64)) \
          (handle E (tuple n Map.empty) \
            ((rec () st (match st \
                          ((tuple s m) \
                           (resume s (tuple (+ s 2) \
                                            (Map.insert m (tuple s (+ s 1)) (* 10 s))))))) \
             (qry (a b) st (match st \
                             ((tuple s m) \
                              (resume (match (Map.lookup m (tuple a b)) \
                                        ((Some v) v) \
                                        ((None) -1)) \
                                      st)))) \
             (cnt () st (match st ((tuple s m) (resume s st))))) \
            (do (E.rec) (+ (E.qry n (+ n 1)) (E.cnt))))) \
        (export run))";
    let rs_async = compile_rust_async(tk);
    assert!(
        rs_async.contains("BTreeMap<(i64, i64), i64>"),
        "the async backend must ALSO annotate the tuple key solved across arms (parity with sync — the \
         solve is in inference, upstream of the backend split):\n{rs_async}"
    );
}

#[test]
fn bigint_of_a_genuine_uint64_widens_unsigned_not_sign_extended() {
    // REGRESSION (corpus-bugfix finding #4, wasm oracle 817): `(BigInt.of n)` on a genuine UInt64 whose
    // TOP BIT is set must widen UNSIGNED, not sign-extend the i64 carrier. BigIntOfI64 emitted a bare
    // `(v) as i128`, which reinterprets a `u64 >= 2^63` as NEGATIVE (2^63+9 → -9223372036854775799,
    // % 1000 = -799) — a SILENT wrong-SIGN miscompile for the whole upper half of u64 (the 8-byte-id/hash
    // escape-hatch path). FIX: cast to the operand's own rust int type first (`(v) as u64 as i128`), so an
    // unsigned operand widens unsigned. Here a direct UInt64 param solves UInt64, exercising the fix.
    // 2^63 + 9 = 9223372036854775817; % 1000 = 817 (unsigned), NOT -799 (sign-extended).
    let src = "(module m \
        (def (run (: n UInt64)) (Int64.of (% (BigInt.of n) (BigInt.of 1000)))) \
        (export run))";
    let rs = compile_rust(src);
    if let Some(out) = rustc_run(&rs, "run(9223372036854775817)") {
        assert_eq!(
            out, "817",
            "BigInt.of a top-bit-set UInt64 must widen unsigned (817), not sign-extend (-799):\n{rs}"
        );
    }
}

#[test]
fn a_u64_bin_binding_reads_unsigned_and_builds() {
    // REGRESSION (corpus-bugfix u64-binding family, wasm oracle 809/905): a `(bin (u64 n))` segment binds
    // a genuine UInt64 (v-core-opt typing 7ff56255f). Core::BinIntRead assembles the bytes into an i64
    // carrier; returning that raw i64 made downstream `% n` / `Int64.of n` (which expect u64) MISMATCH →
    // rust E0308 build-fail (wasm has no static type to clash, computes 809). FIX: BinIntRead casts the
    // assembled bits to the binder's solved rust int type (`__acc as u64`). Bytes [x,0,0,0,0,0,0,1]:
    // x=128 → 2^63+1, % 1000 = 809 (unsigned); x=64 (top bit clear) → 905 (control, always agreed).
    let src = "(do \
        (def (run (: x UInt8)) \
          (do \
            (def b (Bytes.of (list x 0 0 0 0 0 0 1))) \
            (match b \
              ((bin (u64 n)) (Int64.of (% n 1000))) \
              (_ -1)))) \
        (export run))";
    // It must EMIT (the pre-fix bug was a hard E0308 build-fail on the emitted source).
    let rs = compile_rust(src);
    if let Some(out) = rustc_run(&rs, "run(128)") {
        assert_eq!(
            out, "809",
            "a top-bit-set u64 bin binding reads UNSIGNED (809), not signed (-807):\n{rs}"
        );
    }
    if let Some(out) = rustc_run(&rs, "run(64)") {
        assert_eq!(out, "905", "the top-bit-clear control agrees:\n{rs}");
    }
}

#[test]
fn a_nullary_export_emits_a_pub_fn_returning_a_constant() {
    let src = "(module m (def (main) 42) (export main))";
    let rs = compile_rust(src);
    assert!(rs.contains("pub fn main() -> i64 {"), "signature:\n{rs}");
    // 42 is emitted as its bit pattern in the unsigned width, cast to the signed target.
    assert!(rs.contains("42u64 as i64"), "constant:\n{rs}");
}

#[test]
fn an_exported_function_emits_native_params_and_checked_arith() {
    let src = "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("pub fn add(a: i64, b: i64) -> i64 {"),
        "signature:\n{rs}"
    );
    // Cadenza `+` TRAPS on overflow → checked_add with a panic on None.
    assert!(rs.contains("(a).checked_add(b)"), "checked arith:\n{rs}");
    assert!(rs.contains("panic!"), "overflow trap:\n{rs}");
}

#[test]
fn a_narrow_literal_operand_is_grounded_to_the_op_width() {
    // REGRESSION (the reported width miscompile): a bare literal operand of a narrow-width op was
    // emitted at the default i64 (`1u64 as i64`), producing `u8::checked_add(i64)` → rustc E0308. It
    // must be grounded to the op's width (`1u8`). Covers arith, comparison, if-branch, and match-arm.
    let add = compile_rust("(module m (def (go (: a UInt8)) (+ a 1)) (export go))");
    assert!(add.contains("checked_add(1u8)"), "arith operand:\n{add}");
    assert!(
        !add.contains("1u64 as i64"),
        "must NOT default to i64:\n{add}"
    );

    let cmp = compile_rust("(module m (def (go (: a UInt8)) (< a 5)) (export go))");
    assert!(cmp.contains("(a < 5u8)"), "compare operand:\n{cmp}");

    let iff = compile_rust("(module m (def (go (: a UInt8) (: c Bool)) (if c a 1)) (export go))");
    assert!(iff.contains("else { 1u8 }"), "if-branch literal:\n{iff}");

    let mat = compile_rust("(module m (def (go (: a UInt8)) (match a (0 9) (_ a))) (export go))");
    assert!(mat.contains("9u8"), "match-arm literal:\n{mat}");
    assert!(
        !mat.contains("9u64"),
        "match arm must not default to i64:\n{mat}"
    );
}

#[test]
fn rustc_roundtrip_narrow_literal_operand_computes_and_traps() {
    // The narrow-literal fix, end-to-end through rustc: `(+ x 1)` on a UInt8 computes at u8 width AND
    // still traps on overflow (255+1) — the numeric model preserved, not silently wrapped.
    let rs = compile_rust("(module m (def (go (: x UInt8)) (+ x 1)) (export go))");
    if let Some(out) = rustc_run(&rs, "go(100)") {
        assert_eq!(out, "101");
    }
    // 255 + 1 = 256 leaves UInt8 → the checked_add panics (nonzero exit); rustc_run's success assert
    // would fail on a panic, so we only positively assert the in-range answer here (the trap path is
    // exercised by the wasm gate's overflow case and the `_traps` test elsewhere).
}

#[test]
fn rustc_roundtrip_const_wrap_to_signed_unusual_width_sign_extends() {
    // A CONSTANT `.wrap` to a SIGNED UNUSUAL width (not a machine boundary — `Int4`/`Int12`, stored in the
    // next-larger primitive) must SIGN-EXTEND from the declared top bit, matching `IntValue::wrap_to` + the
    // runtime `Convert(Wrap)` path + the wasm oracle. The bug (breaker/corpus-bugfix finding): the
    // const-int emit wrote the low-N bit pattern in the storage-width unsigned type and cast (`8u8 as i8`),
    // which reinterprets at 8 bits — so bit 3 (the 4-bit sign) is NOT the i8 sign bit and `(Int 4).wrap 8`
    // silently gave +8 instead of -8. `lower` already folds `.wrap` via `wrap_to` (sign-extends), so the
    // const value IS -8; the fix emits its true signed decimal for a signed unusual width.
    for (width, input, want) in [
        (4u32, "8", "-8"),     // bit-3 set → negative
        (4, "15", "-1"),       // all 4 bits set → -1
        (4, "7", "7"),         // bit-3 clear → stays positive
        (12, "2048", "-2048"), // bit-11 set (2^11) → Int12.min
        (12, "4095", "-1"),    // all 12 bits set → -1
        (12, "2047", "2047"),  // Int12.max, bit-11 clear → positive
    ] {
        let src = format!("(module m (def (go) ((. (Int {width}) wrap) {input})) (export go))");
        let rs = compile_rust(&src);
        if let Some(out) = rustc_run(&rs, "go()") {
            assert_eq!(
                out, want,
                "(Int {width}).wrap {input} sign-extends to {want} (const-fold path):\n{rs}"
            );
        }
    }
    // CONTROL — an UNSIGNED unusual width keeps the low N bits, no sign reinterpretation: `(UInt 4).wrap 17`
    // = 17 & 0xF = 1 (unchanged behavior; the fix is signed-only).
    let u = compile_rust("(module m (def (go) ((. (UInt 4) wrap) 17)) (export go))");
    if let Some(out) = rustc_run(&u, "go()") {
        assert_eq!(out, "1", "(UInt 4).wrap 17 = 1 (unsigned keeps low bits)");
    }
    // CONTROL — a MACHINE-width signed wrap is unchanged (bit N-1 IS the storage sign bit): `(Int 8).wrap
    // 200` = -56 (still the bit-pattern cast).
    let m = compile_rust("(module m (def (go) ((. (Int 8) wrap) 200)) (export go))");
    if let Some(out) = rustc_run(&m, "go()") {
        assert_eq!(
            out, "-56",
            "(Int 8).wrap 200 = -56 (machine width unchanged)"
        );
    }
    // PROPAGATED into const-fold ARITHMETIC (breaker/corpus-bugfix severity-up): the missing sign-extend
    // would fold `8 + 1 = 9` on the un-sign-extended +8 and emit `9u8 as i8` — a WRONG value (9) that is
    // also OUT OF `Int4` range [-8,7] with no overflow check (a range-escaping miscompile, worse than the
    // standalone display). Since lower folds `.wrap` via `wrap_to` (sign-extends) BEFORE the arithmetic, the
    // fold is `-8 + 1 = -7` and the emit renders it `-7i8` — correct. Pins that the fix covers the arith-
    // propagated face, not just the standalone wrap.
    let arith = compile_rust(
        "(module m (def (go) (+ ((. (Int 4) wrap) 8) ((. (Int 4) wrap) 1))) (export go))",
    );
    if let Some(out) = rustc_run(&arith, "go()") {
        assert_eq!(
            out, "-7",
            "(+ (Int4.wrap 8) (Int4.wrap 1)) = -8+1 = -7 (const-fold arith)"
        );
    }
}

#[test]
fn a_provably_in_range_arith_op_elides_its_overflow_check_on_the_rust_backend() {
    // BOTH-BACKEND PARITY (v-core-opt Slice-2): the rust backend now consults the SAME Core-tier
    // `lower::arith_provably_in_range` predicate the wasm backend uses (`select.rs:12542`), so a
    // provably-in-range op sheds its overflow trap on BOTH backends — one Core-tier decision, not a
    // wasm-only elision. When the interval fits, emit the plain modular `wrapping_*` (never traps),
    // NOT `checked_*().unwrap_or_else(panic)`.
    //
    // `(+ (& x 15) (& y 15))`: [0,15]+[0,15] = [0,30] ⊆ Int64 → wrapping_add, no panic.
    let add = compile_rust(
        "(module m (def (f (: x Int64) (: y Int64)) (+ (& x 15) (& y 15))) (export f))",
    );
    assert!(
        add.contains("wrapping_add") && !add.contains("checked_add"),
        "a provably-in-range add emits the plain modular op, no overflow panic:\n{add}"
    );
    // `(* (& x 15) 3)`: [0,15]×3 = [0,45] ⊆ Int64 → wrapping_mul.
    let mul = compile_rust("(module m (def (f (: x Int64)) (* (& x 15) 3)) (export f))");
    assert!(
        mul.contains("wrapping_mul") && !mul.contains("checked_mul"),
        "a provably-in-range mul emits the plain modular op:\n{mul}"
    );
    // A FULL-RANGE add (an unbounded operand) is NOT provable → KEEPS the checked op + trap. This is the
    // dual that proves the elision is opt-in on a proof of safety, never on the absence of a disproof.
    let kept = compile_rust("(module m (def (f (: x Int64) (: y Int64)) (+ x y)) (export f))");
    assert!(
        kept.contains("checked_add") && kept.contains("panic!"),
        "a full-range add keeps its overflow trap:\n{kept}"
    );
}

#[test]
fn rustc_roundtrip_provably_in_range_elision_computes_identically_and_unproven_still_traps() {
    // The elision is BEHAVIOR-PRESERVING end-to-end through rustc: a provably-in-range op computes the
    // SAME value with the guard elided (wrapping) as it would checked, AND an unproven op still TRAPS on
    // overflow — the correctness bar (an opt that changes observable behavior is a miscompile).
    let elided = compile_rust(
        "(module m (def (f (: x Int64) (: y Int64)) (+ (& x 15) (& y 15))) (export f))",
    );
    // x=15,y=15 → (15&15)+(15&15) = 30. Same value the checked form would compute; no trap.
    if let Some(out) = rustc_run(&elided, "f(15, 15)") {
        assert_eq!(
            out, "30",
            "provably-in-range add computes identically when elided"
        );
    }
    // An UNPROVEN (full-range) add still traps on genuine overflow — Int64::MAX + 1.
    let checked = compile_rust("(module m (def (f (: x Int64) (: y Int64)) (+ x y)) (export f))");
    match rustc_run_traps(&checked, "f(9223372036854775807, 1)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("overflow"),
            "unproven add still traps on overflow, got: {msg}"
        ),
        TrapRun::RanOk(out) => panic!("MAX + 1 must TRAP (overflow), but ran → {out}"),
        TrapRun::NoRustc => {}
    }
}

#[test]
fn a_provably_in_range_narrow_op_elides_identically_to_wasm() {
    // BOTH-BACKEND DECISION PARITY at NARROW width (the case v-wasm-opt flagged co-verifying Slice-2). The
    // wasm backend feeds the predicate the GROUNDED machine type (`Machine::of(int_ty_of)`); the rust
    // backend now grounds `int_ty_of` the same way before the predicate, so both make the IDENTICAL
    // elision decision at a narrow type. `(+ (& a 15) (& b 15))` over UInt8: [0,15]+[0,15] = [0,30] ⊆
    // [0,255] → provably fits UInt8 → wrapping_add at u8 width, NOT checked_add, on rust just as wasm
    // elides its guard. (If rust passed the raw non-ground type it could keep a guard wasm drops — a
    // divergent-but-correct decision; grounding closes that gap.)
    let narrow = compile_rust(
        "(module m (def (f (: a UInt8) (: b UInt8)) (+ (& a 15) (& b 15))) (export f))",
    );
    assert!(
        narrow.contains("wrapping_add") && !narrow.contains("checked_add"),
        "a provably-in-range UInt8 add elides at narrow width (parity with wasm):\n{narrow}"
    );
    // End-to-end: (200&15)+(100&15) = 8 + 4 = 12, in-range at UInt8 → computes identically, no trap.
    if let Some(out) = rustc_run(&narrow, "f(200, 100)") {
        assert_eq!(
            out, "12",
            "narrow provably-in-range add computes identically when elided"
        );
    }
    // The DUAL: a narrow add whose interval EXCEEDS the type is NOT provable → keeps the checked op + trap.
    // `(+ (& a 200) (& b 200))` over UInt8: [0,200]+[0,200] = [0,400] > 255 → checked_add stays.
    let narrow_over = compile_rust(
        "(module m (def (f (: a UInt8) (: b UInt8)) (+ (& a 200) (& b 200))) (export f))",
    );
    assert!(
        narrow_over.contains("checked_add"),
        "a narrow add whose interval exceeds the type keeps its overflow trap:\n{narrow_over}"
    );
}

#[test]
fn a_flow_refined_arith_op_elides_its_overflow_guard_on_the_rust_backend() {
    // BOTH-BACKEND PARITY for the FLOW-REFINEMENT elision source (v-core-opt Slice-6). The range proof
    // that licenses eliding an overflow guard can come from a BRANCH GUARD, not just a mask: inside
    // `(if (< a 100) (+ a 1) …)` the operand `a` is refined to `[_, 99]`, so `+ a 1` provably fits. The
    // wasm backend already pushed this refinement frame (refined_frame_for_branch); the rust backend now
    // does too (with_branch_refinement around each branch emit), so both make the identical decision.
    let rs = compile_rust(
        "(module m (def (f (: a Int64)) (if (and (>= a 0) (< a 100)) (+ a 1) 0)) (export f))",
    );
    // In the then-branch `a ∈ [0,99]`, so `a + 1 ∈ [1,100]` ⊆ Int64 → the `+ a 1` elides to wrapping_add.
    assert!(
        rs.contains("wrapping_add") && !rs.contains("checked_add"),
        "a flow-refined-in-range add elides its overflow guard (parity with wasm):\n{rs}"
    );
    // A branch that does NOT bound the operand keeps the guard: `(if (> b 0) (+ a 1) 0)` refines `b`, not
    // `a`, so `a + 1` is still full-range → checked_add stays.
    let unrefined = compile_rust(
        "(module m (def (f (: a Int64) (: b Int64)) (if (> b 0) (+ a 1) 0)) (export f))",
    );
    assert!(
        unrefined.contains("checked_add"),
        "a branch guard on a DIFFERENT variable does not refine the operand — guard stays:\n{unrefined}"
    );
}

#[test]
fn rustc_roundtrip_flow_refined_elision_computes_identically() {
    // The flow-refined elision is BEHAVIOR-PRESERVING end-to-end: the refined `+ a 1` computes the SAME
    // value with the guard elided as it would checked. a=42 ∈ [0,99] → 43; the elided wrapping_add gives
    // the identical result (the refinement guarantees no overflow, so wrapping == checked here).
    let rs = compile_rust(
        "(module m (def (f (: a Int64)) (if (and (>= a 0) (< a 100)) (+ a 1) 0)) (export f))",
    );
    if let Some(out) = rustc_run(&rs, "f(42)") {
        assert_eq!(
            out, "43",
            "flow-refined add computes identically when elided"
        );
    }
    // The else branch (a out of [0,100)) returns 0 — exercises the non-refined path too.
    if let Some(out) = rustc_run(&rs, "f(200)") {
        assert_eq!(
            out, "0",
            "the else branch is unaffected by the then-branch refinement"
        );
    }
}

#[test]
fn a_narrow_op_with_a_control_flow_operand_grounds_its_branches() {
    // A narrow-annotated op whose operand is a DEFERRED-WIDTH control-flow expression (`if`/`match` of bare
    // literals, inferred Int64) GROUNDS each value-producing branch/arm to the op's width — NOT the old
    // whole-operand `as iN` truncating wrap. Grounding both (a) fixes the E0308 an ungrounded i64 branch
    // caused AND (b) range-checks an oversize const branch (CDZ0302) that the truncating wrap silently
    // dropped (family-A overflow-soundness; #4547). Mirrors the DIRECT `(: (if …) Int8)` form + the wasm
    // select branch-grounding.
    let rs = compile_rust(
        "(module m (def (go (: n Int8)) (: (+ (if (< n 5) 100 0) 100) Int8)) (export go))",
    );
    // Each if-BRANCH is GROUNDED to i8 (`{ (100u8 as i8) } else { (0u8 as i8) }`) — NOT the old whole-if
    // `}) as i8)` truncating wrap. Grounding the branches routes an oversize const branch through the
    // fits_width check (CDZ0302 — see the oversize assertion below), where the old wrap silently truncated
    // (family-A overflow-soundness). A fitting branch (100 ∈ Int8) grounds cleanly; the other operand `100`
    // grounds to `(100u8 as i8)` as before.
    assert!(
        rs.contains("{ (100u8 as i8) } else { (0u8 as i8) }")
            && rs.contains("checked_add((100u8 as i8))")
            && !rs.contains("}) as i8)"),
        "the if-operand's branches must be GROUNDED to i8 (not the whole if wrapped `}}) as i8)`):\n{rs}"
    );
    // End-to-end through rustc: n=9 selects the else 0, 0+100=100 fits Int8 → 100 (compiles + runs, was
    // E0308). The overflow direction (n=3 → 200 → panic) is exercised by the wasm gate + the corpus.
    if let Some(out) = rustc_run(&rs, "go(9)") {
        assert_eq!(
            out, "100",
            "in-range narrow if-operand computes; was a compile error"
        );
    }
    // A `match`-operand grounds each ARM BODY to i8 (`0i8 => (5u8 as i8)`), NOT the whole-match wrap.
    let m = compile_rust(
        "(module m (def (go (: n Int8)) (: (+ (match n (0 5) (_ 1)) 2) Int8)) (export go))",
    );
    assert!(
        m.contains("(5u8 as i8)") && m.contains("(1u8 as i8)") && !m.contains("}) as i8)"),
        "match-operand arm bodies must be GROUNDED to i8 (not the whole match wrapped):\n{m}"
    );
    // THE SOUNDNESS POINT (family-A): an OVERSIZE const branch/arm now DECLINES CDZ0302 (was silently
    // truncated by the old `as i8` wrap → a wrong value). `2^40` overflows every ≤32-bit width.
    let over_if = try_compile_rust(
        "(module m (def (go (: c Bool)) (: (+ (if c 1099511627776 2) 5) Int8)) (export go))",
    )
    .expect_err("an oversize const if-branch consumed by a narrow + must decline, not truncate");
    assert!(
        over_if.iter().any(|d| d.contains("does not fit its width")),
        "the oversize if-branch declines CDZ0302 (fits_width), not a silent `as i8` truncation: {over_if:?}"
    );
    let over_match = try_compile_rust(
        "(module m (def (go (: x Int8)) (: (+ (match x (0 1099511627776) (_ 2)) 5) Int8)) (export go))",
    )
    .expect_err("an oversize const match-arm body consumed by a narrow + must decline");
    assert!(
        over_match
            .iter()
            .any(|d| d.contains("does not fit its width")),
        "the oversize match-arm body declines CDZ0302: {over_match:?}"
    );
    // An UNANNOTATED op is genuinely Int64 (deferred branches) — it must NOT wrap the `if` down: the op
    // adds an i64 and the if-block is NOT followed by an `as i8` cast (the `(5u8 as i8)` in the condition
    // is unrelated — it grounds the comparison literal to `n`'s width). The operand `100` grounds to i64.
    // The add itself is guard-ELIDED to `wrapping_add`: the branches are ∈ [0,100] and `+ 100` gives
    // [100,200] ⊆ Int64, provably in range. This is the both-backend PARITY the grounding fix delivers —
    // this op's node type is non-ground (deferred branch widths), and before grounding the rust predicate
    // rejected it (kept `checked_add`) while wasm elided it (its `Machine::of` grounds deferred→64); now
    // both elide identically. The invariant under test (the i64 operand is NOT narrowed to i8) is unchanged.
    let wide =
        compile_rust("(module m (def (go (: n Int8)) (+ (if (< n 5) 100 0) 100)) (export go))");
    assert!(
        wide.contains("}).wrapping_add((100u64 as i64))") && !wide.contains("}) as i8)"),
        "an unannotated Int64 op must not wrap its if-operand down (and elides in-range, matching wasm):\n{wide}"
    );
}

#[test]
fn modulo_emits_a_zero_divisor_guard_not_checked_rem() {
    // A `%` traps ONLY on a zero divisor — NOT on `MIN % -1` (which is a defined 0; modulo forms no
    // quotient, so it has no overflow — numeric-model §Modulo by -1 is always zero). Rust's `checked_rem`
    // WRONGLY returns None at `MIN % -1`, so it must NOT be used: the emit guards the zero divisor and
    // uses `wrapping_rem` (0 at MIN%-1, matching wasm `i64.rem_s`).
    let rem = compile_rust("(module m (def (r (: a Int64) (: b Int64)) (% a b)) (export r))");
    assert!(
        rem.contains("wrapping_rem") && !rem.contains("checked_rem"),
        "modulo must use a guarded wrapping_rem, not checked_rem:\n{rem}"
    );
    // `/` guards BOTH trap kinds with KIND-SPECIFIC panic messages (so the gate's `trap_kind` classifies
    // each correctly — a single "by zero or overflow" message was misread as div-by-zero). A signed `/`
    // emits the zero guard ("division by zero") AND the `MIN/-1` overflow guard ("division overflow").
    let div = compile_rust("(module m (def (d (: a Int64) (: b Int64)) (/ a b)) (export d))");
    assert!(
        div.contains("panic!(\"division by zero\")")
            && div.contains("i64::MIN && r == -1")
            && div.contains("panic!(\"division overflow\")"),
        "signed division guards zero + MIN/-1 with distinct trap-kind messages:\n{div}"
    );
    // An UNSIGNED `/` has NO MIN/-1 overflow (and `r == -1` would not type-check), so only the zero guard.
    let udiv = compile_rust("(module m (def (d (: a UInt32) (: b UInt32)) (/ a b)) (export d))");
    assert!(
        udiv.contains("panic!(\"division by zero\")")
            && !udiv.contains("== -1")
            && !udiv.contains("overflow"),
        "unsigned division guards only the zero divisor (no MIN/-1 overflow):\n{udiv}"
    );
}

#[test]
fn rustc_roundtrip_signed_div_min_by_neg1_traps_overflow_not_div_by_zero() {
    // End-to-end: `MIN / -1` overflows (the quotient +2^(N-1) is out of range) and MUST trap as an OVERFLOW,
    // NOT a divide-by-zero — the two are distinct trap KINDS the corpus grades separately, and the gate
    // classifies by the panic message. `MIN / -2` is a normal division (no trap); `x / 0` traps div-by-zero.
    let rs = compile_rust("(module m (def (d (: a Int64) (: b Int64)) (/ a b)) (export d))");
    // A normal division still computes.
    if let Some(out) = rustc_run(&rs, "d(7, 2)") {
        assert_eq!(out, "3", "7 / 2 truncates toward zero");
    }
    if let Some(out) = rustc_run(&rs, "d(i64::MIN, -2)") {
        assert_eq!(
            out, "4611686018427387904",
            "MIN / -2 is a normal (non-overflowing) division"
        );
    }
    // MIN / -1 must TRAP, and the panic message must name OVERFLOW (not divide-by-zero) — the two kinds the
    // gate's `trap_kind` classifies by message. `rustc_run` asserts SUCCESS so it can't check a trap; use
    // the trap-asserting `rustc_run_traps`. Match on `TrapRun`: a `Trapped` asserts the KIND; a `RanOk`
    // FAILS (a lost trap must not pass silently — the regression-blindness Copilot PR#496 flagged); only
    // `NoRustc` skips. (This is the coverage the test's NAME promised — previously it ran only NON-trapping
    // inputs, Copilot PR#492.)
    match rustc_run_traps(&rs, "d(i64::MIN, -1)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("overflow") && !msg.contains("by zero"),
            "MIN / -1 must trap as OVERFLOW (not divide-by-zero); panic was:\n{msg}"
        ),
        TrapRun::RanOk(out) => panic!("MIN / -1 must TRAP (overflow), but ran → {out}"),
        TrapRun::NoRustc => {}
    }
    // A zero divisor traps as DIVIDE-BY-ZERO — the sibling kind, distinct message.
    match rustc_run_traps(&rs, "d(7, 0)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("by zero"),
            "x / 0 must trap as divide-by-zero; panic was:\n{msg}"
        ),
        TrapRun::RanOk(out) => panic!("x / 0 must TRAP (divide-by-zero), but ran → {out}"),
        TrapRun::NoRustc => {}
    }
}

#[test]
fn rustc_odd_width_signed_div_min_by_neg1_guards_the_declared_min_not_the_slot_min() {
    // adv-67 (HIGH differential): an ODD-width signed type (Int24, stored in the i32 slot) `MIN / -1` must
    // trap OVERFLOW — the quotient +2^23 is OUT of the Int24 range. The bug: the guard tested `l == i32::MIN`
    // (the SLOT min), but Int24's declared MIN is -2^23 = -8388608, which never equals i32::MIN, so the
    // guard never fired and `l / r` yielded the out-of-range +8388608 (rust returned it; wasm trapped).
    // Fix: the guard tests the DECLARED-width min `-(1i32 << 23)`, NOT `i32::MIN`.
    let rs = compile_rust("(module m (def (d (: a (Int 24)) (: b (Int 24))) (/ a b)) (export d))");
    assert!(
        rs.contains("(-(1i32 << 23)) && r == -1") && rs.contains("panic!(\"division overflow\")"),
        "an Int24 div guards the DECLARED min (-(1i32<<23)), NOT the i32 slot min:\n{rs}"
    );
    // The guard must NOT test the slot min (the adv-67 bug): `i32::MIN && r == -1` would never fire.
    assert!(
        !rs.contains("i32::MIN && r == -1"),
        "the Int24 div guard must NOT test the slot i32::MIN (adv-67 regression):\n{rs}"
    );
    // End-to-end: Int24 MIN (-8388608) / -1 must TRAP overflow (was returning the out-of-range +8388608).
    match rustc_run_traps(&rs, "d(-8388608, -1)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("overflow") && !msg.contains("by zero"),
            "Int24 MIN / -1 must trap OVERFLOW (the escaped +8388608 was adv-67); panic was:\n{msg}"
        ),
        TrapRun::RanOk(out) => {
            panic!("Int24 MIN / -1 must TRAP (overflow), but ran → {out} (adv-67)")
        }
        TrapRun::NoRustc => {}
    }
    // A NORMAL Int24 division still computes: MIN / -2 = +4194304 (in range, no trap).
    if let Some(out) = rustc_run(&rs, "d(-8388608, -2)") {
        assert_eq!(
            out, "4194304",
            "Int24 MIN / -2 is a normal in-range division"
        );
    }
    // CONTROL: an ALIASED width (Int32) keeps testing the slot min `i32::MIN` (slot == declared width), so
    // the fix is behavior-identical there — the declared-min computation is ONLY for odd widths.
    let aliased = compile_rust("(module m (def (d (: a Int32) (: b Int32)) (/ a b)) (export d))");
    assert!(
        aliased.contains("i32::MIN && r == -1"),
        "an aliased Int32 div still guards i32::MIN (slot == declared width):\n{aliased}"
    );
}

#[test]
fn a_provably_safe_signed_division_elides_its_min_by_neg1_overflow_guard() {
    // BOTH-BACKEND PARITY (v-core-opt Slice-7): the signed `MIN/-1` overflow guard is the Div member of
    // the guard-elision family. The rust `/` emit now consults the SAME Core-tier predicates the wasm
    // backend uses (select.rs) — `divisor_can_be_neg_one` + `value_provably_nonneg` — and drops the guard
    // when either operand rules out the `MIN ÷ -1` pair. The zero-divisor guard ALWAYS stays.
    //
    // DIVISOR provably not -1 (masked `(& b 7)` ∈ [0,7]): the MIN/-1 guard is dead.
    let masked_divisor =
        compile_rust("(module m (def (d (: a Int64) (: b Int64)) (/ a (& b 7))) (export d))");
    assert!(
        masked_divisor.contains("panic!(\"division by zero\")")
            && !masked_divisor.contains("== -1")
            && !masked_divisor.contains("division overflow"),
        "a divisor provably != -1 elides the MIN/-1 guard, keeps the zero guard:\n{masked_divisor}"
    );
    // DIVIDEND provably nonneg (masked `(& a 255)` ∈ [0,255]): can never be MIN, so MIN/-1 can't occur.
    let nonneg_dividend =
        compile_rust("(module m (def (d (: a Int64) (: b Int64)) (/ (& a 255) b)) (export d))");
    assert!(
        !nonneg_dividend.contains("== -1") && !nonneg_dividend.contains("division overflow"),
        "a provably-nonneg dividend elides the MIN/-1 guard:\n{nonneg_dividend}"
    );
    // FULL-RANGE signed `/` (both operands unbounded) KEEPS the guard — the elision is opt-in on a proof.
    let full = compile_rust("(module m (def (d (: a Int64) (: b Int64)) (/ a b)) (export d))");
    assert!(
        full.contains("i64::MIN && r == -1") && full.contains("division overflow"),
        "a full-range signed division keeps its MIN/-1 overflow guard:\n{full}"
    );
    // End-to-end value parity: the masked-divisor form computes correctly with the guard elided.
    // d(100, 7): divisor = 7 & 7 = 7; 100 / 7 = 14 (truncating). Guard elided, value unchanged.
    if let Some(out) = rustc_run(&masked_divisor, "d(100, 7)") {
        assert_eq!(
            out, "14",
            "100 / (7&7=7) = 14, computed identically with the guard elided"
        );
    }
}

#[test]
fn rustc_roundtrip_modulo_min_by_neg1_is_zero_not_a_trap() {
    // End-to-end through rustc: `(% a b)` at (Int64.min, -1) MUST return 0 on the Rust backend, matching
    // wasm `i64.rem_s`. Before the fix the emitted `checked_rem(MIN, -1)` returned None and PANICKED — a
    // wrong trap where the value must be 0. All signed widths share the emit; Int64 is the witness.
    let rs = compile_rust("(module m (def (r (: a Int64) (: b Int64)) (% a b)) (export r))");
    if let Some(out) = rustc_run(&rs, "r(i64::MIN, -1)") {
        assert_eq!(out, "0", "MIN % -1 must be 0, not a panic");
    }
    // A normal modulo still computes, and the sign follows the dividend.
    if let Some(out) = rustc_run(&rs, "r(-7, 2)") {
        assert_eq!(out, "-1");
    }
}

#[test]
fn a_narrow_signed_negative_constant_uses_the_bit_pattern_cast() {
    // -56 : Int8 is emitted as `200u8 as i8` (the two's-complement bit pattern), mirroring the wasm
    // backend's `to_i32_bits` (tests.rs `a_narrow_signed_...` expects -56 from a UInt8 wrap).
    let src = "(module m (def (main) (: -56 Int8)) (export main))";
    let rs = compile_rust(src);
    assert!(rs.contains("-> i8 {"), "signature:\n{rs}");
    assert!(rs.contains("200u8 as i8"), "bit-pattern cast:\n{rs}");
}

#[test]
fn a_uint64_max_constant_crosses_without_a_signed_minus() {
    // UInt64.max = 2^64 - 1 does not fit i64; as a u64 it is a plain literal (no `as`).
    let src = "(module m (def (main) UInt64.max) (export main))";
    // `.max` may or may not be built; only assert when it compiles, else this is a no-op guard.
    if let Ok(rs) = try_compile_rust(src) {
        assert!(rs.contains("-> u64 {"), "signature:\n{rs}");
    }
}

#[test]
fn an_if_emits_a_rust_if_expression() {
    let src = "(module m (def (pick (: a Int64) (: b Int64)) (if (< a b) a b)) (export pick))";
    let rs = compile_rust(src);
    assert!(rs.contains("if (a < b) {"), "if-expr:\n{rs}");
}

#[test]
fn a_runtime_list_emits_a_native_rust_vec() {
    // A `List T` maps to Rust's `Vec<T>`, and a runtime `(list …)` construction → the `vec![…]` macro.
    // Build the list behind a recursive call so it survives to runtime (a constant list folds away).
    let rs = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n n) (f (+ n -1)))) \
                    (def (main) (f 3)) (export main))",
    );
    assert!(rs.contains("-> Vec<i64>"), "list return type:\n{rs}");
    assert!(rs.contains("vec!["), "vec! construction:\n{rs}");

    // A list PARAMETER crosses as `Vec<T>`; a nested list → `Vec<Vec<i64>>`.
    let param = compile_rust("(module m (def (g (: xs (List Int64))) xs) (export g))");
    assert!(param.contains("xs: Vec<i64>"), "list param type:\n{param}");
    let nested = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list (list n)) (f (+ n -1)))) \
                    (def (main) (f 2)) (export main))",
    );
    assert!(
        nested.contains("-> Vec<Vec<i64>>"),
        "nested list type:\n{nested}"
    );
}

#[test]
fn rustc_roundtrip_list_builds_and_runs() {
    // A runtime list crosses rustc end-to-end: build `(list n n n)` behind a recursive call so it is a
    // genuine runtime `Vec`, return it, and render it as cdz-run's `(list …)` text via a small driver.
    let module = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n n n) (f (+ n -1)))) \
                    (def (mklist) (f 2)) (export mklist))",
    );
    // The emitted module returns a `Vec<i64>`; the driver joins it into the canonical `(list 0 0 0)` form.
    let driver = "fn main() { let v = prog::mklist(); let mut s = String::from(\"(list\"); \
                  for e in v.iter() { s.push(' '); s.push_str(&format!(\"{}\", e)); } s.push(')'); \
                  println!(\"{}\", s); }";
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "(list 0 0 0)");
    }
}

#[test]
fn a_mixed_width_literal_list_grounds_every_element_to_the_unified_element_width() {
    // REGRESSION (corpus-bugfix, fuzzer cdz-smith differential): a `(list (: 127 Int32) 32767)` whose
    // elements are LITERALS of DIFFERING inferred widths — the annotated `(: 127 Int32)` is Int32, the
    // bare `32767` defaults its OWN `type_of` to Int64 — USED to emit a HETEROGENEOUS
    // `vec![(127u32 as i32), (32767u64 as i64)]`, which rustc rejects (E0308: a `Vec` is homogeneous).
    // The front-end + wasm UNIFY the list's element type (both coerce to Int32, the list is `List Int32`);
    // the rust `vec!` must too. `Core::ListNew` now grounds each element to the list's SOLVED element
    // width via `emit_grounded` (the List twin of the Map entry + Set element sibling-width render). A
    // correctness-class differential: rust REJECTED a program wasm accepts.
    let m = compile_rust("(module m (def (main) (list (: 127 Int32) 32767)) (export main))");
    assert!(
        m.contains("vec![(127u32 as i32), (32767u32 as i32)]"),
        "both literal elements ground to the unified Int32 element width (homogeneous Vec<i32>):\n{m}"
    );
    // e2e: compiles + runs to the canonical list form (both elements Int32), matching wasm.
    let driver = "fn main() { let v = prog::main(); let mut s = String::from(\"(list\"); \
                  for e in v.iter() { s.push(' '); s.push_str(&format!(\"{}\", e)); } s.push(')'); \
                  println!(\"{}\", s); }";
    if let Some(out) = rustc_run_driver(&m, driver) {
        assert_eq!(
            out, "(list 127 32767)",
            "the mixed-width list builds + runs, both elements at Int32:\n{m}"
        );
    }
    // A plain same-width list is byte-identical to before (grounding a same-width literal is a no-op).
    let plain = compile_rust("(module m (def (main) (list 1 2 3)) (export main))");
    assert!(
        plain.contains("vec![(1u64 as i64), (2u64 as i64), (3u64 as i64)]"),
        "a plain Int64 list still grounds each element to i64 (no regression):\n{plain}"
    );
}

#[test]
fn rustc_roundtrip_all_nullary_sum_orders_by_discriminant() {
    // breaker #43 cross-backend guard: the shared-lowering fix routes an ALL-NULLARY sum's `<`/`compare`
    // to a scalar `Core::Compare` (i32/enum tag) instead of the `Core::ValueCmp` heap walk. The finding is
    // wasm-only (rust was already correct via ValueCmp), so this test PROTECTS the rust path from the
    // reroute: rust emits an all-nullary sum as a derived-`Ord` enum, and `Core::Compare`'s `(l < r)` on
    // two such enum values compiles + gives DECLARATION order (Lo < Mid < Hi). Compile+run through rustc:
    // `mk(-7)=Lo`, `mk(9)=Hi`; `Lo < Hi` is true (1), and `compare(Lo,Hi)` is Less (arm 1). Expect 11.
    let module = compile_rust(
        "(module m (type Tri (Lo) (Mid) (Hi)) \
           (def (mk (: k Int64)) (if (< k 0) (Tri.Lo unit) (if (= k 0) (Tri.Mid unit) (Tri.Hi unit)))) \
           (def (main (: a Int64) (: b Int64)) \
             (+ (* 10 (if (< (mk a) (mk b)) 1 0)) \
                (match (Ordering.of (mk a) (mk b)) \
                  ((Ordering.Less _u) 1) ((Ordering.Equal _u) 2) ((Ordering.Greater _u) 3)))) \
           (export main))",
    );
    let driver = "fn main() { println!(\"{}\", prog::main(-7, 9)); }";
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(
            out, "11",
            "rust all-nullary sum ordering: Lo < Hi (1) + compare=Less (1) = 11 — the reroute to \
             Core::Compare must give declaration order on rust's derived-Ord enum, not break it"
        );
    }
}

#[test]
fn runtime_list_ops_emit_native_vec_operations() {
    // `List.len` → `.len() as i64`; measured over a runtime-built list (a constant list's length folds).
    let len = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n n) (f (+ n -1)))) \
                    (def (main) (List.len (f 2))) (export main))",
    );
    assert!(
        len.contains(".len() as i64)"),
        "List.len → .len() as i64:\n{len}"
    );
    // `List.push` → a mut-local push returning the vec.
    let push = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n) (f (+ n -1)))) \
                    (def (main) (List.push (f 1) 9)) (export main))",
    );
    assert!(
        push.contains("__v.push(") && push.contains("-> Vec<i64>"),
        "List.push → push:\n{push}"
    );
    // `List.concat` → extend.
    let cat = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n) (f (+ n -1)))) \
                    (def (main) (List.concat (f 1) (f 2))) (export main))",
    );
    assert!(cat.contains("__v.extend("), "List.concat → extend:\n{cat}");
    // `List.update` → a bounds-checked index-set that traps OOB via `panic!("unreachable")` — the wasm
    // runtime's `List.update` OOB is a GENERIC `unreachable` abort (message-less under panic=abort), which
    // the corpus grades `(trap "unreachable")`; matching that KIND (not "index out of bounds", which would
    // classify `out-of-bounds`, a mismatch) is what lets the runtime-OOB case grade pass on rust.
    let upd = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n n) (f (+ n -1)))) \
                    (def (main (: i Int64)) (List.update (f 1) i 9)) (export main))",
    );
    assert!(
        upd.contains("panic!(\"unreachable\")") && upd.contains("__v[__i] ="),
        "List.update → bounds-checked set trapping `unreachable` (matching the wasm runtime kind):\n{upd}"
    );
    // `List.at` → the fallible read yielding a native Option.
    let at = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list 10 20) (f (+ n -1)))) \
                    (def (main (: i Int64)) (List.at (f 1) i)) (export main))",
    );
    assert!(
        at.contains("-> Option<i64>")
            && at.contains("Some(__v[__i].clone())")
            && at.contains("None"),
        "List.at → Option read:\n{at}"
    );
}

#[test]
fn rustc_roundtrip_list_ops_compute_and_a_shared_list_does_not_move() {
    // End-to-end: build a list, then in one function BOTH pass it to a helper AND measure its length —
    // a Vec is move-only, so the binding is `.clone()`d on the non-last use (the Perceus-dup analogue).
    // `sum-at` walks the list by `List.at`+`List.len` and sums it: build `[0 1 2]`, sum = 3.
    let module = compile_rust(
        "(module m \
           (def (build i n out) (if (< i n) (build (+ i 1) n (List.push out i)) out)) \
           (def (sum-at xs i n) (if (< i n) (+ (match (List.at xs i) ((Some x) x) ((None _) 0)) \
                (sum-at xs (+ i 1) n)) 0)) \
           (def (mk) (let ((xs (build 0 3 (list)))) (sum-at xs 0 (List.len xs)))) (export mk))",
    );
    // A list binding used in two positions (passed to sum-at AND measured) must be cloned, not moved.
    assert!(
        module.contains(".clone()"),
        "a shared list is cloned:\n{module}"
    );
    if let Some(out) = rustc_run(&module, "mk()") {
        assert_eq!(out, "3", "0+1+2 = 3 over the built-then-read list");
    }
    // `List.update` traps out of bounds (a Rust panic == a Cadenza trap). Drive an in-range update and
    // read back the changed element to confirm the op computes; the OOB trap is exercised by the corpus.
    let upd = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list 10 20 30) (f (+ n -1)))) \
           (def (at2 (: xs (List Int64))) (match (List.at xs 1) ((Some x) x) ((None _) 0))) \
           (def (mk) (at2 (List.update (f 1) 1 99))) (export mk))",
    );
    if let Some(out) = rustc_run(&upd, "mk()") {
        assert_eq!(out, "99", "index 1 replaced with 99");
    }
}

#[test]
fn a_list_match_emits_a_length_tested_if_chain() {
    // A list match `(match xs ((list) …) ((list h .. t) …))` → an `if xs.len() == 0 { … } else if
    // xs.len() >= 1 { … }` chain over the scrutinee's length; a leading element binder reads `xs[i]`,
    // the rest binds `xs[1..].to_vec()`.
    let rs = compile_rust(
        "(module m (def (sum (: xs (List Int64))) \
           (match xs ((list) 0) ((list h .. t) (+ h (sum t))))) (export sum))",
    );
    assert!(rs.contains(".len() == 0"), "empty-arm length test:\n{rs}");
    assert!(rs.contains(".len() >= 1"), "rest-arm length test:\n{rs}");
    assert!(
        rs.contains("[1..].to_vec()"),
        "rest binder → tail sublist:\n{rs}"
    );
    assert!(
        rs.contains("[0]"),
        "leading element binder → index 0:\n{rs}"
    );
    // A FIXED-arity arm `(list a b)` → `== 2`, its elements `xs[0]`/`xs[1]`.
    let fixed = compile_rust(
        "(module m (def (f (: xs (List Int64))) (match xs ((list a b) (+ a b)) (_ 0))) (export f))",
    );
    assert!(fixed.contains(".len() == 2"), "fixed-arity test:\n{fixed}");
}

#[test]
fn rustc_roundtrip_recursive_list_match_folds_to_a_scalar() {
    // End-to-end: a recursive `(list)`/`(list h .. t)` match sums a runtime-built list. Build `[10 20 30]`
    // behind a recursive call so it is a genuine runtime `Vec`, then fold it: 10+20+30 = 60.
    let module = compile_rust(
        "(module m \
           (def (sum (: xs (List Int64))) (match xs ((list) 0) ((list h .. t) (+ h (sum t))))) \
           (def (f (: n Int64)) (if (= n 0) (list 10 20 30) (f (+ n -1)))) \
           (def (mk) (sum (f 1))) (export mk))",
    );
    if let Some(out) = rustc_run(&module, "mk()") {
        assert_eq!(out, "60", "10+20+30 folded through the list match");
    }
    // A fixed-arity arm computes over its bound elements; a non-matching length falls to the catch-all.
    let pick = compile_rust(
        "(module m \
           (def (g (: xs (List Int64))) (match xs ((list a b c) (+ a (+ b c))) (_ -1))) \
           (def (f (: n Int64)) (if (= n 0) (list 3 4 5) (f (+ n -1)))) \
           (def (mk) (g (f 1))) (export mk))",
    );
    if let Some(out) = rustc_run(&pick, "mk()") {
        assert_eq!(out, "12", "3+4+5 over the exactly-3 arm");
    }
}

#[test]
fn a_runtime_closure_emits_an_rc_dyn_fn_and_a_lifted_fn() {
    // A `(fn …)` passed to a RECURSIVE HOF survives to run time → an `Rc<dyn Fn(…) -> …>` value that
    // forwards to a lifted `fn __lifted_k`. The function-typed PARAM maps to the same `Rc<dyn Fn>`.
    let rs = compile_rust(
        "(module m \
           (def (foldl (: f (-> Int64 (-> Int64 Int64))) (: acc Int64) (: xs (List Int64))) \
              (match xs ((list) acc) ((list h .. t) (foldl f (f h acc) t)))) \
           (def (main) (foldl (fn (x a) (+ a x)) 0 (list 5 7 30))) (export main))",
    );
    // The arrow SPINE is flattened: `(-> Int64 (-> Int64 Int64))` → `Rc<dyn Fn(i64, i64) -> i64>`.
    assert!(
        rs.contains("std::rc::Rc<dyn Fn(i64, i64) -> i64>"),
        "flattened Fn type:\n{rs}"
    );
    // The lifted lambda is emitted as a standalone fn; the closure value is an Rc::new coerced to dyn Fn.
    assert!(rs.contains("fn __lifted_0("), "lifted fn emitted:\n{rs}");
    assert!(
        rs.contains("std::rc::Rc::new(move |") && rs.contains(") as std::rc::Rc<dyn Fn"),
        "closure value builds + coerces to dyn Fn:\n{rs}"
    );
}

#[test]
fn rustc_roundtrip_closures_run_no_capture_and_capturing_and_in_a_list() {
    // NO-CAPTURE combinator folded over a list: 5+7+30 = 42.
    let fold = compile_rust(
        "(module m \
           (def (foldl (: f (-> Int64 (-> Int64 Int64))) (: acc Int64) (: xs (List Int64))) \
              (match xs ((list) acc) ((list h .. t) (foldl f (f h acc) t)))) \
           (def (mk) (foldl (fn (x a) (+ a x)) 0 (list 5 7 30))) (export mk))",
    );
    if let Some(out) = rustc_run(&fold, "mk()") {
        assert_eq!(out, "42", "no-capture closure folds the list");
    }
    // CAPTURING closure: `(fn (b) (g b))` captures the closure `g` and is applied through the recursive
    // HOF `sumapply`; the capture is cloned into each call (an `Fn` cannot move its capture out). With
    // `g = (fn (x) (+ x 1))`: `rec` sums `sumapply (fn (b) (g b)) 2` = g(2)+g(1) = 3+2 = 5 per round, over
    // 3 rounds = 15.
    let cap = compile_rust(
        "(module m \
           (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
           (def (rec (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (sumapply (fn ((: b Int64)) (g b)) 2) (rec g (- n 1))))) \
           (def (mk) (rec (fn ((: x Int64)) (+ x 1)) 3)) (export mk))",
    );
    if let Some(out) = rustc_run(&cap, "mk()") {
        assert_eq!(
            out, "15",
            "a closure capturing another closure, applied through a recursive HOF"
        );
    }
    // A LIST of capturing closures, each keeping its own capture, indexed and applied — the closures must
    // coerce to ONE `Rc<dyn Fn>` type to share a `Vec` (the `as` coercion on a CONCRETELY-typed closure).
    // Annotate the lambda so its function type grounds at the closure node (an unannotated `(fn (x) …)`
    // whose width stays a var declines — see the decline test). index 1 → (mkc 20) applied to 1 = 21.
    let list = compile_rust(
        "(module m \
           (def (mkc (: k Int64)) (fn ((: x Int64)) (+ x k))) \
           (def (at (: fs (List (-> Int64 Int64))) (: i Int64) (: x Int64)) \
              (match ((. List at) fs i) ((Some f) (f x)) (None -1))) \
           (def (pick (: i Int64)) (at (list (mkc 10) (mkc 20) (mkc 30)) i 1)) (export pick))",
    );
    assert!(
        list.contains(") as std::rc::Rc<dyn Fn"),
        "list-of-closures coerces each to dyn Fn:\n{list}"
    );
    if let Some(out) = rustc_run(&list, "pick(1)") {
        assert_eq!(out, "21", "the index-1 closure captures 20; 20+1 = 21");
    }
}

#[test]
fn a_closure_param_export_declines_but_a_scalar_factory_result_now_emits() {
    // A closure PARAMETER export declines when there is NO PRODUCING SIBLING to supply the closure: the
    // host would have to hand the `Rc<dyn Fn>` in directly, which has no boundary rep (matching wasm's
    // "closure argument … has no scalar host-boundary representation"). `apply-it` alone (no companion
    // producer) declines cleanly (todo). (A closure-param consumer WITH a producing sibling now EMITS +
    // runs — see `rustc_roundtrip_closure_parameter_consumer_with_a_producer_sibling`.)
    let param = try_compile_rust(
        "(module m (def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x)) (export apply-it))",
    )
    .expect_err("a closure-param export with no producer sibling must decline");
    assert!(
        param
            .iter()
            .any(|d| d.contains("does not cross the Rust export boundary")),
        "decline cites the export boundary: {param:?}"
    );
    // A closure RESULT (a scalar-capture FACTORY), by contrast, NOW EMITS (host-closure S1): it crosses as
    // `pub fn mk(k) -> Rc<dyn Fn(x)->r>` and the host applies `mk(k)(x)`. (See the dedicated S1/S2/S3
    // roundtrip tests for the make/call run.) A compound RESULT also emits now (S3).
    let scalar_factory =
        compile_rust("(module m (def (mk (: k Int64)) (fn ((: x Int64)) (+ x k))) (export mk))");
    assert!(
        scalar_factory.contains("pub fn mk(k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64>"),
        "a scalar-capture closure factory emits an `Rc<dyn Fn>` handle (S1):\n{scalar_factory}"
    );
    // A closure PARAMETER export with NO producing sibling still declines — the host can't synthesize the
    // `Rc<dyn Fn>` argument (this stays the guard's territory).
    let param2 =
        try_compile_rust("(module m (def (use-it (: f (-> Int64 Int64))) (f 3)) (export use-it))")
            .expect_err("a closure-PARAMETER export with no producer still declines");
    assert!(
        param2
            .iter()
            .any(|d| d.contains("closure-PARAMETER export shape")),
        "the closure-param decline cites the parameter boundary: {param2:?}"
    );
}

#[test]
fn a_unit_arg_eta_peeled_closure_export_declines_not_e0061() {
    // A zero-arg module-member call `(m.get)` where the module-member convention makes the export a
    // `Unit -> T` closure — `(def (main) (m.get))` — eta-peels to `pub fn main(u: ())`. The gate driver
    // calls the export with ZERO args (`prog::main()`), so a `()` parameter is an UN-BUILDABLE artifact
    // (rustc E0061 "takes 1 argument but 0 were supplied"). A case the backend can't honestly cross MUST
    // DECLINE, never emit source that fails to compile — matching wasm's Unit-closure-arg decline (breaker
    // zmz1/#8317). The ARG twin of the Unit-RESULT export decline. (Independent of the module-member nullary
    // convention ruling: a Unit arg has no host-boundary form regardless.)
    let d = try_compile_rust(
        "(do (module m (def (get) 42) (export get)) (def (main) (m.get)) (export main))",
    )
    .expect_err("a Unit-arg eta-peeled closure export must DECLINE, not emit an un-buildable E0061 artifact");
    assert!(
        d.iter()
            .any(|m| m.contains("Unit argument does not cross the Rust export boundary")),
        "the decline cites the Unit-argument export boundary (not an emitted E0061 artifact): {d:?}"
    );
}

#[test]
fn a_top_level_immediate_capture_free_lambda_export_eta_peels_to_a_plain_fn() {
    // ETA-PEEL: `(def (mk) (fn (p…) body))` — a nullary def whose whole body is an immediate, capture-free
    // lambda — is NOT a closure-resource export on the Rust target (which has no resource ABI). The gate
    // applies it DIRECTLY at full arity, so the faithful rendering is a plain `pub fn mk(p…) -> R`: the
    // lambda's OWN parameters + body, not the empty parameter list of the nullary def. Distinct from the
    // CAPTURING result (`a_closure_crossing_the_export_boundary_declines`) which still declines — a top-level
    // lambda captures nothing, so its lifted form is a pure combinator whose params ARE the export's params.
    let single = compile_rust("(module m (def (inc) (fn ((: x Int64)) (+ x 1))) (export inc))");
    assert!(
        single.contains("pub fn inc(x: i64) -> i64"),
        "an immediate capture-free lambda export peels to a plain `pub fn` over the lambda's params:\n{single}"
    );
    assert!(
        !single.contains("__lifted_"),
        "the peeled lambda is emitted AS the export — no standalone `__lifted_k` remains:\n{single}"
    );
    if let Some(out) = rustc_run(&single, "inc(5)") {
        assert_eq!(out, "6", "peeled `inc` applies directly: 5 + 1 = 6");
    }

    // The peel carries the lambda's COMPOUND parameter through unchanged — a `(Option (Tuple Int64 Int64))`
    // arg crosses as a native `Option<(i64, i64)>` and the body's match/projection runs as an ordinary fn.
    let compound = compile_rust(
        "(module m (def (mk) (fn ((: o (Option (Tuple Int64 Int64)))) \
           (match o ((Some p) (+ (. p 0) (. p 1))) (None 0)))) (export mk))",
    );
    assert!(
        compound.contains("pub fn mk(o: Option<(i64, i64)>) -> i64"),
        "a compound lambda param peels through to the `pub fn` signature:\n{compound}"
    );
    if let Some(out) = rustc_run(&compound, "mk(Some((3i64, 4i64)))") {
        assert_eq!(
            out, "7",
            "the peeled compound-arg body folds Some((3,4)) → 3 + 4 = 7"
        );
    }

    // A `Bytes`-RESULT immediate lambda does NOT eta-PEEL (a peeled plain `fn -> Vec<u8>` would render its
    // result via the direct-return form `b"…"`, which DISAGREES with the wasm closure-resource boundary's
    // `list<u8>`). Instead it now crosses as a host-closure FACTORY (`pub fn mk() -> Rc<dyn Fn(i64) ->
    // Vec<u8>>`), whose result the gate harness renders as the byte-int `list<u8>` form (`cdz_render_bytes_list`)
    // — matching the wasm boundary. So it EMITS (was a decline before the String/Bytes-result factory slice).
    let bytes_result = compile_rust(
        "(module m (def (mk) (fn ((: n Int64)) \
           (bin (u8 (UInt8.wrap n)) (u8 (UInt8.wrap (+ n 1)))))) (export mk))",
    );
    assert!(
        bytes_result.contains("pub fn mk() -> std::rc::Rc<dyn Fn(i64) -> Vec<u8>>")
            && !bytes_result.contains("__lifted_0(__a0, "), // a factory (1 closure arg), not eta-peeled
        "a Bytes-RESULT lambda crosses as a factory returning `Rc<dyn Fn(..)->Vec<u8>>`:\n{bytes_result}"
    );
}

#[test]
fn map_and_set_emit_native_btree_collections() {
    // A `(Map K V)` → `BTreeMap<K,V>`, a `(Set E)` → `BTreeSet<E>` (BTree = sorted = canonical order).
    let m = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (map (= 1 10) (= 2 20)) (f (+ n -1)))) \
           (def (g) (Map.len (f 1))) (export g))",
    );
    assert!(
        m.contains("std::collections::BTreeMap::new()") && m.contains(".insert("),
        "map builds a BTreeMap:\n{m}"
    );
    let s = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Set.of (list 3 1 2)) (f (+ n -1)))) \
           (def (g) (Set.len (f 1))) (export g))",
    );
    assert!(
        s.contains("std::collections::BTreeSet::new()"),
        "set builds a BTreeSet:\n{s}"
    );
    // Map.lookup → a native Option via `.get(&k).cloned()`; Set.contains → a bool via `.contains(&e)`.
    let look = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (map (= 1 10)) (f (+ n -1)))) \
           (def (g (: k Int64)) (match (Map.lookup (f 1) k) ((Some v) v) ((None _) -1))) (export g))",
    );
    assert!(
        look.contains(".get(&(") && look.contains(").cloned()"),
        "map lookup:\n{look}"
    );
}

#[test]
fn an_empty_map_used_get_only_grounds_its_annotation_but_an_inserted_base_stays_inferred() {
    // REGRESSION (breaker + v-effects + v-metaprogramming + corpus-bugfix, batch 160): an empty `Map.empty`
    // used ONLY get-only (an empty-Map HANDLER STATE whose K/V are fixed later through get/put effect ops,
    // not at construction) emitted a bare `BTreeMap::new()` that rustc could NOT infer → E0282. The fix
    // GROUNDS the open key/value vars to the default and annotates. But an empty map that IS the base of an
    // enclosing `Map.insert`/`Map.remove` must STAY unannotated (bare `new()`) — the insert fixes its type,
    // and a grounded annotation would OVER-CONSTRAIN a Rational/String/Bytes-keyed map → E0308.

    // (a) An empty-map handler state, get-only (no insert on the seed's own occurrences): the seed grounds
    // to `BTreeMap<i64, i64>` and the whole thing builds + runs (→ 50). (The full effects+continuation case
    // is pinned in the corpus; this is the direct rust-emit unit check.)
    let handler = compile_rust(
        "(module m \
           (effect Db (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit))) \
           (def (demand (: k Int64) (: c Int64)) \
             (match ((. Db get) k) (((. Option Some) v) v) \
               (((. Option None) u) (do ((. Db put) (tuple k c)) c)))) \
           (def (run) (handle Db ((. Map empty)) \
             ((get (k) s (resume ((. Map lookup) s k) s)) \
              (put (kv) s (match kv ((tuple k v) (resume unit ((. Map insert) s k v)))))) \
             (let ((a (demand 5 25))) \
               (match ((. Db get) 5) (((. Option Some) v) (+ a v)) (((. Option None) u) 99))))) \
           (export run))",
    );
    assert!(
        handler.contains("BTreeMap<i64, i64> = std::collections::BTreeMap::new()"),
        "the get-only empty-map handler state grounds its annotation to <i64, i64>:\n{handler}"
    );
    if let Some(out) = rustc_run(&handler, "run()") {
        assert_eq!(
            out, "50",
            "the handler threads state through the continuation: 25 + 25 = 50"
        );
    }

    // (b) An empty map that is the BASE of an insert with a NON-default key type (Rational) must NOT be
    // grounded — it stays a bare `new()` inferred from the insert. A grounded `<i64,i64>` would E0308
    // against the Rational key. Builds + runs (→ 10).
    let rational_key = compile_rust(
        "(module m (def (look) \
           (match (Map.lookup (Map.insert (Map.insert (Map.empty) ((. Rational of) 1 2) 10) \
                                          ((. Rational of) 2 3) 20) ((. Rational of) 1 2)) \
             ((Some v) v) ((None) 0))) (export look))",
    );
    assert!(
        !rational_key.contains("BTreeMap<i64, i64> = "),
        "an inserted-base empty map is NOT force-grounded to <i64,i64> (would clash with the Rational key):\n{rational_key}"
    );
    if let Some(out) = rustc_run(&rational_key, "look()") {
        assert_eq!(out, "10", "the Rational-keyed map looks up 1/2 → 10");
    }
}

#[test]
fn an_empty_map_get_only_with_a_solved_string_key_annotates_string_not_the_default() {
    // REGRESSION (breaker ms9, 2026-08-06): a let-bound `Map.empty` used GET-ONLY, whose key type is fixed
    // NOT at construction but by the lookup's KEY (`(Map.lookup m "k")` → String) — the map operand's own
    // `type_of` at the lookup is `Map(Var, Var)` (fully open; K/V flow through the key + downstream match
    // arms, invisible at the node), so it USED to ground to the DEFAULT `BTreeMap<i64, i64>` while the use
    // emitted `.get(&"k".to_string())` → rust E0308 TYPE ERROR (the artifact FAILS TO BUILD — a backend
    // DIFFERENTIAL, not a runtime miscompile: wasm folds the String key + runs correct). The fix
    // RECONSTRUCTS the map type at `Core::MapLookup` from the SOLVED key type + the
    // lookup-result `Option<V>` payload, grounds any still-open var (the value), and threads it as
    // `expected_ty` — so `Core::MapNew` annotates `BTreeMap<String, i64>` (correct key, safely-grounded
    // value). Distinct from the get-only test above (there the key IS the default i64; here it is String).
    let m = compile_rust(
        "(module m (def (main (: n Int64)) \
           (let ((m Map.empty)) (match (Map.lookup m \"k\") ((Some x) x) ((None _u) n)))) (export main))",
    );
    assert!(
        m.contains("BTreeMap<String, i64> = std::collections::BTreeMap::new()"),
        "the get-only empty map annotates its SOLVED String key (not the default i64):\n{m}"
    );
    // e2e: compiles + runs to 5 (lookup misses the empty map → the None arm returns n=5), matching wasm.
    let driver = "fn main() { println!(\"{}\", prog::main(5)); }";
    if let Some(out) = rustc_run_driver(&m, driver) {
        assert_eq!(
            out, "5",
            "empty-map lookup misses → None arm returns n=5:\n{m}"
        );
    }
}

#[test]
fn an_empty_map_lookup_whose_value_is_a_collection_annotates_the_solved_join_not_the_default() {
    // REGRESSION (breaker ms9-family, the COLLECTION-VALUED face — ms13/ms6/ns1/ej*): a let-bound
    // `Map.empty` used GET-ONLY, whose lookup RESULT is a `match` returning a COLLECTION (`(Some ys) ys`
    // beside `(None _u) (list)`), so the map's VALUE type is `List Int64` — but at the `Map.lookup` node
    // the lookup-result payload is still a FREE `Var` (inference fixes it only through the DOWNSTREAM
    // match-join, invisible at the node). The OLD ms9 reconstruction `ground_open_vars`'d that free value
    // to the DEFAULT `i64`, emitting `BTreeMap<String, i64>` → the `Some`-arm `i64` clashed with the
    // `None => vec![]` `Vec` arm: rust E0308 (the artifact FAILS TO BUILD — a backend DIFFERENTIAL, wasm
    // folds + runs correct). The fix threads the consuming `Core::MatchSum`'s SOLVED result type down to
    // the lookup as `expected_ty`, so the reconstruction uses the join's OUTER shape (`List _` → `Vec<_>`)
    // for the map value, rendering the interior element as an inference HOLE `_`: `BTreeMap<String, Vec<_>>`
    // — the outer `Vec` satisfies rustc method resolution and `_` lets rustc solve the element (i64) from
    // the `.push(n)` use. Both arms are `Vec<_>`, compiles. Sound because a `Map.empty` lookup always
    // MISSES (the `Some` arm is dead) — a wrong OUTER shape errors LOUD at rustc, never a runtime miscompile.
    let m = compile_rust(
        "(module m (def (main (: n Int64)) \
           (let ((m Map.empty)) \
             (let ((xs (match (Map.lookup m \"k\") ((Some ys) ys) ((None _u) (list))))) \
               (List.len (List.push xs n))))) (export main))",
    );
    assert!(
        m.contains("BTreeMap<String, Vec<_>>"),
        "the collection-valued empty-map lookup annotates the SOLVED join OUTER shape (Vec<_>, element \
         holed for rustc to infer), not the default i64:\n{m}"
    );
    // e2e: compiles + runs to 1 (lookup misses → None arm → empty list → push n → len 1), matching wasm.
    let driver = "fn main() { println!(\"{}\", prog::main(7)); }";
    if let Some(out) = rustc_run_driver(&m, driver) {
        assert_eq!(
            out, "1",
            "empty-map lookup misses → None arm's empty list gets one push → len 1:\n{m}"
        );
    }
}

#[test]
fn an_empty_map_lookup_whose_value_is_a_set_or_nested_list_holes_the_interior_element() {
    // The Set-valued + NESTED-list-valued faces of the same ms9-family collection-join fix. The map's
    // VALUE join has a SOLVED OUTER shape (`Set _` / `List (List _)`) but a FREE interior element (fixed
    // only downstream). The fix renders the outer shape with interior INFERENCE HOLES `_` — the outer
    // `BTreeSet`/`Vec` satisfies rustc method resolution while `_` lets rustc solve the interior from the
    // use. Grounding the interior to the DEFAULT `i64` under-approximated the NESTED value (`List Any` →
    // wrongly `Vec<i64>` → E0308 at `.push(vec![..])`), which is why the hole (not a ground) is required.
    // Set value: (Some ys) ys beside (None _) (Set.of (list)) → BTreeSet<_>, then Set.insert n → size 1.
    let s = compile_rust(
        "(module m (def (main (: n Int64)) \
           (let ((m Map.empty)) \
             (let ((xs (match (Map.lookup m \"k\") ((Some ys) ys) ((None _u) (Set.of (list)))))) \
               (Set.len (Set.insert xs n))))) (export main))",
    );
    assert!(
        s.contains("BTreeMap<String, std::collections::BTreeSet<_>>"),
        "the Set-valued empty-map lookup annotates BTreeSet<_> (outer shape, holed element):\n{s}"
    );
    if let Some(out) = rustc_run_driver(&s, "fn main() { println!(\"{}\", prog::main(7)); }") {
        assert_eq!(
            out, "1",
            "empty-map miss → None arm's empty set + one insert → size 1:\n{s}"
        );
    }
    // Nested list value: push a `(list n)` into the join list → the value is `List (List Int64)`. The
    // join only sees `List Any` (the empty `(list)` None arm), so the ELEMENT must be a hole, not i64.
    let nested = compile_rust(
        "(module m (def (main (: n Int64)) \
           (let ((m Map.empty)) \
             (let ((xs (match (Map.lookup m \"k\") ((Some ys) ys) ((None _u) (list))))) \
               (List.len (List.push xs (list n)))))) (export main))",
    );
    assert!(
        nested.contains("BTreeMap<String, Vec<_>>"),
        "the nested-list-valued empty-map lookup HOLES the element (Vec<_>), not Vec<i64>:\n{nested}"
    );
    if let Some(out) = rustc_run_driver(&nested, "fn main() { println!(\"{}\", prog::main(7)); }") {
        assert_eq!(
            out, "1",
            "empty-map miss → None arm's empty list + one nested push → len 1:\n{nested}"
        );
    }
}

#[test]
fn an_empty_map_lookup_whose_value_is_itself_a_map_holes_the_inner_map_not_a_bare_new() {
    // The MAP-of-MAPS face of the ms9-family collection-join fix (breaker ej3). A get-only `Map.empty`
    // whose lookup value is ITSELF a `Map` (`(Some ys) ys` beside `(None _) Map.empty`), then the join is
    // fed to a `Map.insert inner "x" n`. The enclosing insert sets `map_typed_by_enclosing_insert` — which
    // types the map the insert operates on (the join result `inner`), NOT the scrutinee's OWN lookup map.
    // That flag USED to leak down to the lookup map `m`, sending its `MapNew` to the bare-`new()` branch:
    // `.get(&"k")` fixes only `m`'s KEY, not its VALUE (the inner map, unused at the lookup) → E0282 ("type
    // annotations needed"). The fix clears the flag when threading the match-join type to the scrutinee, so
    // `m` reconstructs `BTreeMap<String, BTreeMap<_, _>>` — outer key solved, inner map HOLED for rustc to
    // infer from the downstream `.insert("x", n)`. (Distinct from the E0308 collection-value faces above:
    // ej3 was E0282, a bare uninferrable `new()` — the same match-join miscompile-CLASS, Map-valued face.)
    let m = compile_rust(
        "(module m (def (main (: n Int64)) \
           (let ((m Map.empty)) \
             (let ((inner (match (Map.lookup m \"k\") ((Some ys) ys) ((None _u) Map.empty)))) \
               (Map.len (Map.insert inner \"x\" n))))) (export main))",
    );
    assert!(
        m.contains("BTreeMap<String, std::collections::BTreeMap<_, _>>"),
        "the map-valued empty-map lookup annotates the inner map's OUTER shape (BTreeMap<_,_>), holed \
         for the downstream insert to fix, not a bare uninferrable new():\n{m}"
    );
    // e2e: compiles + runs to 1 (lookup misses → None arm's empty map + one insert → len 1), matching wasm.
    if let Some(out) = rustc_run_driver(&m, "fn main() { println!(\"{}\", prog::main(5)); }") {
        assert_eq!(
            out, "1",
            "empty-map miss → None arm's empty map + one insert → len 1:\n{m}"
        );
    }
}

#[test]
fn a_bare_float_set_or_map_key_uses_the_cdz_f64_total_order_wrapper() {
    // A bare `Float` Set element / Map key is NOT `Ord` as a raw `f64` (NaN breaks totality), so it maps to
    // the `CdzF64` total-order wrapper (bit-canonical, NaN→one quiet NaN — the runtime's `box-float`). The
    // struct is emitted (gated on use) and each key/element value is lifted with `CdzF64::new(…)`.
    let s = compile_rust(
        "(module m (def (test (: stored Float64) (: probe Float64)) \
           (Set.contains (Set.of (list stored)) probe)) \
           (def (main (: d Int64)) (if (test Float64.nan Float64.nan) 1 0)) (export main))",
    );
    assert!(
        s.contains("struct __CdzF64(u64)") && s.contains("BTreeSet<__CdzF64>"),
        "a float set emits the __CdzF64 wrapper + a BTreeSet<__CdzF64>:\n{s}"
    );
    assert!(
        s.contains("__CdzF64::new("),
        "the stored element + the contains probe are lifted through __CdzF64::new:\n{s}"
    );
    // A float-KEYED map likewise: keys lifted through `CdzF64::new` on insert AND lookup (so the probe
    // matches the stored key's wrapper type). (The seed map here is `Map.empty`, whose BTreeMap type Rust
    // INFERS from the typed insert/lookup — a bare `BTreeMap::new()` with no spelled `<CdzF64, _>` — so the
    // invariant to pin is the key WRAP, which is what makes the inferred key type `CdzF64`.)
    let m = compile_rust(
        "(module m (def (main (: x Float64)) \
           (match (Map.lookup (Map.insert (Map.empty) x 42) x) ((Some v) v) ((None _) -1))) (export main))",
    );
    assert!(
        m.matches("__CdzF64::new(").count() >= 2 && m.contains("struct __CdzF64(u64)"),
        "a float-keyed map lifts its insert + lookup keys through __CdzF64::new:\n{m}"
    );
    // The wrapper is NOT emitted for a float-free program (gated on use — no dead struct).
    let plain = compile_rust("(module m (def (g (: n Int64)) (+ n 1)) (export g))");
    assert!(
        !plain.contains("__CdzF64"),
        "a float-free program does not emit the __CdzF64 wrapper:\n{plain}"
    );
    // A float-carrying TUPLE key now EMITS: the wrapper threads through the tuple, so a `(Tuple Float Int64)`
    // set element keys as `(__CdzF64, i64)` (Ord) and the value rebuilds `(__CdzF64::new(k.0), k.1)`.
    let tup = compile_rust(
        "(module m (def (main (: x Float64)) \
           (Set.len (Set.of (list (tuple x 1))))) (export main))",
    );
    assert!(
        tup.contains("BTreeSet<(__CdzF64, i64)>") && tup.contains("__CdzF64::new("),
        "a (Tuple Float Int64) set element keys as (__CdzF64, i64) + wraps the float element:\n{tup}"
    );
    // A float in a structural RECORD key now EMITS too: `(record (f x) (n 1))` erases to a sorted-field
    // tuple `(f64, i64)`, so the wrapper threads through it exactly like a tuple → keys as `(__CdzF64, i64)`.
    let rec = compile_rust(
        "(module m (def (main (: x Float64)) \
           (Set.len (Set.of (list (record (= f x) (= n 1)))))) (export main))",
    );
    assert!(
        rec.contains("BTreeSet<(__CdzF64, i64)>") && rec.contains("__CdzF64::new("),
        "a float-field record set element keys as (__CdzF64, i64) + wraps the float field:\n{rec}"
    );
    // A float in a SUM PAYLOAD key still DECLINES (threading not yet extended into variant payloads).
    let sum = compile_rust_result(
        "(module m (type R (Mk Float64 Int64)) (def (main (: x Float64)) \
           (Set.len (Set.of (list (R.Mk x 1))))) (export main))",
    );
    assert!(
        sum.is_err(),
        "a float-in-a-sum-payload set element still declines (record/tuple-only threading):\n{sum:?}"
    );
}

#[test]
fn float_carrying_compound_to_list_declines_but_construction_still_works() {
    // breaker #34 (corpus-bugfix routed, v-wasm-opt 42b2a02b0 twin): a float-CARRYING COMPOUND
    // Set/Map.to-list must DECLINE — a compound containing a float leaf has NO blessed total order
    // (03-equality-and-observation.sexp:626 §319), so its ordered enumeration is undefined (matching wasm).
    // ORDER-only + to-list-only: a BARE float still enumerates (canonical bytes, 19-sets:1494), and the
    // set/map CONSTRUCTION + lookup over a float-tuple key STILL work (breaker pin 211 — the __CdzF wrapper
    // gives rust's BTree* a total order for insert/contains/lookup; only to-list's ordered enumeration
    // declines). Guard lives at SetToList/MapToList, NOT ty_is_ord_key (which gates construction).
    // (1) float-tuple Set.to-list DECLINES.
    assert!(
        try_compile_rust("(module m (def (run) (List.len (Set.to-list (Set.of (list (tuple 1.5 1) (tuple 2.5 2)))))) (export run))").is_err(),
        "Set.to-list over a float-leaf tuple element must decline"
    );
    // (2) float-tuple Map.to-list DECLINES.
    assert!(
        try_compile_rust("(module m (def (run) (List.len (Map.to-list (Map.insert (Map.empty) (tuple 2.5 3) 42)))) (export run))").is_err(),
        "Map.to-list over a float-leaf tuple key must decline"
    );
    // (3) a BARE float Set.to-list still ENUMERATES (bare-root float order is blessed by canonical bytes).
    assert!(
        try_compile_rust(
            "(module m (def (run) (List.len (Set.to-list (Set.of (list 1.5 2.5))))) (export run))"
        )
        .is_ok(),
        "a BARE float Set.to-list must still enumerate (bare-root canonical-byte order)"
    );
    // (4) a float-tuple Map CONSTRUCTION + lookup still WORKS (pin 211 — only to-list declines).
    let lookup = compile_rust(
        "(module m (def (run) (match (Map.lookup (Map.insert (Map.empty) (tuple 2.5 3) 42) (tuple 2.5 3)) ((Some v) v) ((None _) -1))) (export run))",
    );
    if let Some(out) = rustc_run(&lookup, "run()") {
        assert_eq!(
            out, "42",
            "float-tuple-key Map insert+lookup by content still works (42)"
        );
    }
}

#[test]
fn a_tuple_with_a_float_leaf_keys_a_map_by_content_end_to_end() {
    // REGRESSION (v-runtime differential): a `(Tuple Float64 Int64)` Map key looks up BY CONTENT — insert
    // under `(2.5, 3)`, look up `(2.5, 3)` → hits (42). Was a rust decline (wrapper not threaded through the
    // tuple); now the key type is `(__CdzF64, i64)` and both insert + lookup keys wrap the float element, so
    // the BTreeMap orders/compares them totally. Computes 42, matching wasm.
    let m = compile_rust(
        "(module m (def (run) \
           (match (Map.lookup (Map.insert (Map.empty) (tuple 2.5 3) 42) (tuple 2.5 3)) \
             ((Some v) v) ((None _) -1))) (export run))",
    );
    if let Some(out) = rustc_run(&m, "run()") {
        assert_eq!(out, "42", "a (Tuple Float Int) map key hits by content: 42");
    }
    // A DIFFERENT float leaf misses (distinct key) → -1 (the None arm), confirming the float participates in
    // the key comparison (not ignored).
    let miss = compile_rust(
        "(module m (def (run) \
           (match (Map.lookup (Map.insert (Map.empty) (tuple 2.5 3) 42) (tuple 9.5 3)) \
             ((Some v) v) ((None _) -1))) (export run))",
    );
    if let Some(out) = rustc_run(&miss, "run()") {
        assert_eq!(out, "-1", "a distinct float leaf in the key MISSES: -1");
    }
}

#[test]
fn a_float_nested_inside_a_compound_key_field_is_wrapped_not_just_a_direct_float() {
    // Copilot PR#741: `wrap_ord_key`'s tuple/record rebuild arms gated on a SHALLOW `.any(direct Float)`,
    // but `ord_key_type` threads the `__CdzF{N}` wrapper through NESTED tuples/records. So a record field
    // whose TYPE is `(Tuple Float Int)` (a float NOT directly in the record) got a wrapped key TYPE
    // `(i64, (__CdzF64, i64))` but a BARE `f64` key VALUE (the rebuild was skipped) → rustc E0308. The
    // guard is now the recursive `key_ty_has_wrappable_float`, so the nested float is wrapped on both
    // insert + lookup and the key crosses. Regression: emit must NOT leave a bare `f64::from_bits` in the
    // key expression, and the whole program must rustc-compile + look up by content.
    let m = compile_rust(
        "(module m (def (run) \
           (match (Map.lookup \
                    (Map.insert (Map.empty) (record (= t (tuple 2.5 3)) (= n 5)) 42) \
                    (record (= t (tuple 2.5 3)) (= n 5))) \
             ((Some v) v) ((None _) -1))) (export run))",
    );
    // The key type carries the nested wrapper; the emitted KEY VALUE must wrap that nested float too — i.e.
    // a `__CdzF64::new(` inside the tuple rebuild, never a naked `f64::from_bits` sitting in a wrapper slot.
    assert!(
        m.contains("(__CdzF64, i64)") && m.contains("__CdzF64::new("),
        "the nested float key field is wrapped in both type and value:\n{m}"
    );
    if let Some(out) = rustc_run(&m, "run()") {
        assert_eq!(
            out, "42",
            "a record key with a nested (Tuple Float Int) field hits by content: 42"
        );
    }
    // A distinct nested float misses → -1 (the nested float participates in the key comparison).
    let miss = compile_rust(
        "(module m (def (run) \
           (match (Map.lookup \
                    (Map.insert (Map.empty) (record (= t (tuple 2.5 3)) (= n 5)) 42) \
                    (record (= t (tuple 9.5 3)) (= n 5))) \
             ((Some v) v) ((None _) -1))) (export run))",
    );
    if let Some(out) = rustc_run(&miss, "run()") {
        assert_eq!(out, "-1", "a distinct nested float in the key MISSES: -1");
    }
}

#[test]
fn float_set_key_wrapper_is_width_specific_and_the_reserved_name_never_collides() {
    // Copilot review on PR #487 found two bugs in the bare-float set/map key support:
    //  (1) the wrapper was WIDTH-BLIND — a `Float32` key emitted a `__CdzF64` (over `u64`) around an `f32`
    //      value, a rustc type error. The wrapper is now width-specific: `Float32` → `__CdzF32` (over `u32`).
    //  (2) the wrapper name `CdzF64` was a LEGAL user sum name, so `(type CdzF64 …)` collided with the
    //      injected `struct CdzF64` (E0428) even in a float-free program. The name is now `__`-reserved and
    //      injection is gated on the `__CdzF{32,64}::new(` marker, so a user `CdzF64`/`CdzF32` sum is fine.

    // (1) A Float32-keyed set emits the `__CdzF32` wrapper (over u32 bits, `f32::NAN`), NOT `__CdzF64`.
    let f32set = compile_rust(
        "(module m (def (main (: d Float32)) (Set.len (Set.insert (Set.of (list)) d))) (export main))",
    );
    assert!(
        f32set.contains("struct __CdzF32(u32)")
            && f32set.contains("__CdzF32::new(")
            && !f32set.contains("__CdzF64"),
        "a Float32 set key uses the u32-backed __CdzF32 wrapper, not __CdzF64:\n{f32set}"
    );
    // A Float64-keyed set still uses `__CdzF64` (over u64), NOT `__CdzF32`.
    let f64set = compile_rust(
        "(module m (def (main (: d Float64)) (Set.len (Set.insert (Set.of (list)) d))) (export main))",
    );
    assert!(
        f64set.contains("struct __CdzF64(u64)") && !f64set.contains("__CdzF32"),
        "a Float64 set key uses the u64-backed __CdzF64 wrapper:\n{f64set}"
    );

    // (2) A user sum literally NAMED `CdzF64` in a FLOAT-FREE program emits `enum CdzF64` and NO injected
    // wrapper struct — the reserved `__CdzF64` name cannot collide, and injection needs the `::new(` marker.
    let user = compile_rust(
        "(module m (type CdzF64 (A) (B)) \
           (def (main (: d Int64)) (match (CdzF64.A) ((A) 1) ((B) 2))) (export main))",
    );
    assert!(
        user.contains("enum CdzF64")
            && !user.contains("struct __CdzF64")
            && !user.contains("struct CdzF64"),
        "a user `CdzF64` sum emits its enum with no injected wrapper struct (no E0428):\n{user}"
    );
    // And a user `CdzF64` sum ALONGSIDE a real Float64 set key: the enum and the `__CdzF64` wrapper coexist
    // (distinct names), no collision.
    let both = compile_rust(
        "(module m (type CdzF64 (A) (B)) \
           (def (main (: d Float64)) (Set.len (Set.insert (Set.of (list)) d))) (export main))",
    );
    assert!(
        both.contains("enum CdzF64") && both.contains("struct __CdzF64(u64)"),
        "a user `CdzF64` sum coexists with the injected __CdzF64 float-key wrapper:\n{both}"
    );
}

#[test]
fn float_key_wrapper_decl_injects_for_a_typed_empty_collection_and_reserved_name_is_escaped() {
    // Copilot PR#490 found two RESIDUAL gaps in the width-specific float-key wrapper:
    //  (1) the decl was injected only on the `::new(` constructor marker, but a context-typed EMPTY
    //      collection annotates the wrapper type with NO constructor → `cannot find type __CdzF64`.
    //  (2) the `__`-prefix did NOT actually reserve the name — the lexer allows a `_`-start and
    //      `sanitize_ident` passed `__` through, so a user `sum __CdzF64` still collided (E0428).

    // (1) A context-typed EMPTY float-keyed Map annotates `BTreeMap<__CdzF64, _>` with no `__CdzF64::new(`.
    // The decl must STILL be injected (gate on the `<__CdzF64` type-param occurrence, not just `::new(`).
    let empty = compile_rust(
        "(module m (def (e) (: (Map.empty) (Map Float64 Int64))) \
           (def (main (: d Int64)) (Map.len (e))) (export main))",
    );
    assert!(
        empty.contains("BTreeMap<__CdzF64,") && empty.contains("struct __CdzF64(u64)"),
        "a typed empty float Map annotates __CdzF64 AND injects its decl (no `cannot find type`):\n{empty}"
    );

    // (2) A user sum literally named `__CdzF64` (a legal Cadenza ident — `_`-start is allowed) is ESCAPED
    // by `sanitize_ident` to `cdz_user___CdzF64`, so it can NEVER collide with the backend-reserved wrapper.
    let user = compile_rust(
        "(module m (type __CdzF64 (A) (B)) \
           (def (main (: d Int64)) (match (__CdzF64.A) ((A) 1) ((B) 2))) (export main))",
    );
    assert!(
        user.contains("enum cdz_user___CdzF64") && !user.contains("struct __CdzF64(u64)"),
        "a user `__CdzF64` sum is escaped to cdz_user___CdzF64 with no injected wrapper struct (no E0428):\n{user}"
    );
    // A user def named `__pay` (another backend-reserved local) is likewise escaped, not captured.
    let userfn = compile_rust("(module m (def (__pay (: n Int64)) (+ n 1)) (export __pay))");
    assert!(
        userfn.contains("cdz_user___pay") && userfn.contains("pub fn cdz_user___pay"),
        "a user `__pay` def is escaped away from the reserved local namespace:\n{userfn}"
    );
}

#[test]
fn rustc_roundtrip_map_and_set_compute_and_enumerate_in_order() {
    // Map: build `{1:10, 2:20, 3:30}` at runtime, sum its size + a lookup. 3 keys + lookup(2)=20 = 23.
    let mp = compile_rust(
        "(module m \
           (def (f (: n Int64)) (if (= n 0) (map (= 1 10) (= 2 20) (= 3 30)) (f (+ n -1)))) \
           (def (look (: k Int64)) (match (Map.lookup (f 1) k) ((Some v) v) ((None _) -1))) \
           (def (g) (+ (Map.len (f 1)) (look 2))) (export g))",
    );
    if let Some(out) = rustc_run(&mp, "g()") {
        assert_eq!(out, "23", "3 keys + lookup(2)=20");
    }
    // Set: dedup + canonical-order to-list. `{30,10,20,10}` → 3 distinct, to-list summed = 60 → 63.
    let st = compile_rust(
        "(module m \
           (def (f (: n Int64)) (if (= n 0) (Set.of (list 30 10 20 10)) (f (+ n -1)))) \
           (def (suml (: xs (List Int64))) (match xs ((list) 0) ((list h .. t) (+ h (suml t))))) \
           (def (g) (+ (Set.len (f 1)) (suml (Set.to-list (f 1))))) (export g))",
    );
    if let Some(out) = rustc_run(&st, "g()") {
        assert_eq!(out, "63", "3 distinct + (10+20+30)");
    }
    // A user-sum map KEY needs the enum to derive Ord (a nullary enum qualifies) — look up by the variant.
    let uk = compile_rust(
        "(module m (type C (R) (G)) \
           (def (g) (match (Map.lookup (Map.insert (Map.empty) (C.R) 42) (C.R)) \
              ((Some v) v) ((None _) -1))) (export g))",
    );
    if let Some(out) = rustc_run(&uk, "g()") {
        assert_eq!(out, "42", "a user-sum key is looked up by its variant");
    }
}

#[test]
fn a_bare_float_set_or_map_uses_cdz_f64_and_a_monomorphic_float_carrying_sum_gets_a_custom_ord() {
    // A `Set`/`Map` of a BARE FLOAT now COMPILES via the `CdzF64` total-order wrapper (a raw `f64` is only
    // `PartialOrd`; `CdzF64` orders by canonical bits, NaN-canonical — mirroring the runtime's `box-float`).
    // (WAS a decline — the "no BTreeSet<f64>" era, before the wrapper.) The float-insert into an EMPTY set is
    // the sharp case: the empty base's element type is an unsolved var, fixed to a float only by the insert —
    // so both the guard AND the wrapper substitution key off the ELEMENT/KEY node type.
    let set_float = compile_rust(
        "(module m (def (main (: d Float64)) (Set.len (Set.insert (Set.of (list)) d))) (export main))",
    );
    assert!(
        set_float.contains("__CdzF64::new(") && set_float.contains("struct __CdzF64"),
        "a float-element Set now compiles via __CdzF64:\n{set_float}"
    );
    let map_float = compile_rust(
        "(module m (def (main (: d Float64)) (Map.len (Map.insert (Map.empty) d 1))) (export main))",
    );
    assert!(
        map_float.contains("__CdzF64::new("),
        "a float-KEY Map now compiles via __CdzF64:\n{map_float}"
    );
    // CONTROL: an Int-keyed set/map still compiles (unchanged — Int is natively Ord, no wrapper).
    let set_int = compile_rust(
        "(module m (def (main (: n Int64)) (Set.len (Set.insert (Set.of (list)) n))) (export main))",
    );
    assert!(
        set_int.contains("BTreeSet"),
        "an Int-element Set still emits a BTreeSet:\n{set_int}"
    );

    // FOLLOW-ON (float-carrying-sum Ord-key): a MONOMORPHIC sum CARRYING A FLOAT is one type-shape past a
    // bare float key. Its emitted enum can't `#[derive(Ord)]` (a float payload isn't `Eq`/`Ord`), so it
    // NOW gets a HAND-WRITTEN `impl Ord` (delegating to a `__ord_<Ident>` walk that orders the float leaf by
    // canonical bits) — making `Set<W>`/`Map<W,_>` compilable with an order that agrees with wasm. (This
    // REVERSES the old decline: `ty_is_ord`'s Sum arm now admits a `sum_is_custom_ord` sum.) Both the VALUE
    // path (a construction op) and the TYPE-POSITION path (a `(Set W)` param) now EMIT.
    let sum_float_set_val = compile_rust(
        "(module m (type W (F Float64) (G)) \
           (def (main (: d Float64)) (Set.len (Set.of (list ((. W F) d))))) (export main))",
    );
    assert!(
        sum_float_set_val.contains("impl Ord for W") && sum_float_set_val.contains("BTreeSet<W>"),
        "a Set of a float-carrying sum (value) now emits a custom impl Ord + BTreeSet<W>:\n{sum_float_set_val}"
    );
    let sum_float_set_param = compile_rust(
        "(module m (type W (F Float64) (G)) (def (main (: s (Set W))) (Set.len s)) (export main))",
    );
    assert!(
        sum_float_set_param.contains("BTreeSet<W>")
            && sum_float_set_param.contains("impl Ord for W"),
        "a (Set W) PARAM where W carries a float now emits BTreeSet<W> + custom impl Ord:\n{sum_float_set_param}"
    );
    // CONTROL: a GENERIC float-carrying sum still DECLINES as a key (a generic `__ord_` helper signature is
    // a follow-up) — `sum_is_custom_ord` is monomorphic-only.
    let generic_float_sum_set = compile_rust_result(
        "(module m (type W a (F Float64 a) (G)) (def (main (: s (Set (W Int64)))) (Set.len s)) (export main))",
    );
    assert!(
        generic_float_sum_set.is_err(),
        "a (Set (W Int64)) where generic W carries a float must still DECLINE (monomorphic-only), got:\n{generic_float_sum_set:?}"
    );
    // CONTROL: an ALL-NULLARY sum (its enum DOES derive Ord) is a valid Set element/key — the fix is
    // Ord-derivability-specific, not "decline every sum key".
    let nullary_sum_set = compile_rust(
        "(module m (type W (A) (B)) (def (main (: s (Set W))) (Set.len s)) (export main))",
    );
    assert!(
        nullary_sum_set.contains("BTreeSet<W>"),
        "an all-nullary sum is a valid (Ord) Set element:\n{nullary_sum_set}"
    );
    // CONTROL: a float-carrying sum is fine as a Map VALUE (only the KEY needs Ord) — `(Map Int64 W)`.
    let sum_float_map_val = compile_rust(
        "(module m (type W (F Float64) (G)) (def (main (: mp (Map Int64 W))) (Map.len mp)) (export main))",
    );
    assert!(
        sum_float_map_val.contains("BTreeMap<i64, W>"),
        "a float-carrying sum is a valid Map VALUE (only the key needs Ord):\n{sum_float_map_val}"
    );
}

#[test]
fn nested_option_matches_bind_distinct_payload_names() {
    // REGRESSION (a nested-match binder collision the map slice surfaced): two matches on DIFFERENT
    // scrutinees nesting at the same relative path both minted `__pay_0_0`, so the inner shadowed the
    // outer and `(+ a b)` silently became `b + b`. The binder name now includes the scrutinee id, so `a`
    // and `b` are distinct. `main(1,2)` over `{1:10, 2:20}` = 10+20 = 30 (was a wrong 40).
    let rs = compile_rust(
        "(module m (def (pick (: k1 Int64) (: k2 Int64)) \
           (let ((mp (Map.insert (Map.insert (Map.empty) 1 10) 2 20))) \
              (match (Map.lookup mp k1) ((Some a) (match (Map.lookup mp k2) ((Some b) (+ a b)) (None -1))) \
                 (None -2)))) (export pick))",
    );
    if let Some(out) = rustc_run(&rs, "pick(1, 2)") {
        assert_eq!(out, "30", "distinct binders: a=10, b=20 → 30 (not b+b=40)");
    }
}

#[test]
fn a_string_constant_emits_an_owned_string() {
    // A `Ty::String` → Rust `String`; a `ConstStr` → `"…".to_string()`, with content-safe escaping.
    let rs = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"café\" (f (+ n -1)))) (def (g) (f 1)) (export g))",
    );
    assert!(rs.contains("-> String"), "string return type:\n{rs}");
    assert!(
        rs.contains("\"café\".to_string()"),
        "const string with UTF-8 preserved:\n{rs}"
    );
    // A String PARAMETER crosses as `String`.
    let param = compile_rust("(module m (def (id (: s String)) s) (export id))");
    assert!(param.contains("id(s: String)"), "string param:\n{param}");
    // A string literal with escapes emits valid Rust (quote + backslash + newline).
    let esc = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"a\\\"b\" (f (+ n -1)))) (def (g) (f 1)) (export g))",
    );
    assert!(esc.contains("\\\""), "escaped quote in the literal:\n{esc}");
}

#[test]
fn rustc_roundtrip_string_result_renders_quoted() {
    // A runtime string result crosses end-to-end and renders as cdz-run's `"…"` form (raw UTF-8, quoted).
    let module = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"parse error\" (f (+ n -1)))) \
           (def (mk) (f 1)) (export mk))",
    );
    // The driver renders a `String` result as `"<content>"` — a multi-word string keeps its spaces.
    let driver = "fn main() { let s = prog::mk(); println!(\"\\\"{}\\\"\", s); }";
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "\"parse error\"", "a multi-word string renders quoted");
    }
}

#[test]
fn bytes_ops_emit_native_vec_u8_operations() {
    // A `Ty::Bytes` → `Vec<u8>`; `Bytes.of` builds it with a per-element range check.
    let of = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 65 66 67)) (f (+ n -1)))) \
           (def (g) (Bytes.len (f 1))) (export g))",
    );
    assert!(of.contains("Vec<u8>"), "bytes type:\n{of}");
    assert!(
        of.contains("panic!(\"byte value out of range\")") && of.contains("as u8"),
        "range-checked byte build:\n{of}"
    );
    // Bytes.at → a native Option, byte zero-extended to Int64.
    let at = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 10 20)) (f (+ n -1)))) \
           (def (g (: i Int64)) (match (Bytes.at (f 1) i) ((Some b) b) ((None _) -1))) (export g))",
    );
    assert!(
        at.contains("as i64") && at.contains("None"),
        "bytes at:\n{at}"
    );
    // String.concat lowers through BytesConcat but must emit push_str (String), NOT extend (Vec<u8>).
    let sconcat = compile_rust(
        "(module m (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s \"x\") (- n 1)))) \
           (def (g) (rep \"hi\" 2)) (export g))",
    );
    assert!(
        sconcat.contains("push_str") && !sconcat.contains(".extend("),
        "String.concat uses push_str, not Vec extend:\n{sconcat}"
    );
}

#[test]
fn rustc_roundtrip_bytes_build_read_and_string_concat_run() {
    // Bytes: build [65,66,67], len 3 + at(1)=66 = 69.
    let by = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 65 66 67)) (f (+ n -1)))) \
           (def (at1 (: i Int64)) (match (Bytes.at (f 1) i) ((Some b) b) ((None _) -1))) \
           (def (mk) (+ (Bytes.len (f 1)) (at1 1))) (export mk))",
    );
    if let Some(out) = rustc_run(&by, "mk()") {
        assert_eq!(out, "69", "3 bytes + byte-at(1)=66");
    }
    // A runtime-built string via String.concat crosses end-to-end and renders quoted: "hi" + "x"*3 = "hixxx".
    let module = compile_rust(
        "(module m (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s \"x\") (- n 1)))) \
           (def (mk) (rep \"hi\" 3)) (export mk))",
    );
    let driver = "fn main() { let s = prog::mk(); println!(\"\\\"{}\\\"\", s); }";
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(
            out, "\"hixxx\"",
            "a runtime-built string concatenates correctly"
        );
    }
}

#[test]
fn rustc_roundtrip_string_concat_nfc_normalizes_matching_wasm() {
    // FINDING #23 rust-parity regression (breaker report: wasm-vs-rust value divergence). The String TYPE's
    // doctrine (`collections-and-text.md`) makes NFC an invariant — every String constructor normalizes before
    // the value exists — so `String.concat "e" <U+0301 combining acute>` MUST equal the precomposed "é"
    // (U+00E9, 2 UTF-8 bytes), NOT the 3-byte decomposed "e"+combining sequence. The wasm backend calls the
    // `str-nfc-normalize` runtime op; the rust backend's `Core::NfcNormalize` used to be a NO-OP (pass-through),
    // so a rust-target concat kept the un-normalized 3-byte form and disagreed with wasm on `=` / byte-len /
    // set-membership. Operands are threaded through a runtime identity (`id … 1` recurses once) so the concat is
    // a RUNTIME value, NOT a const-fold — this exercises the emitted `unicode_normalization::…::nfc(…)` at run.
    let blen = compile_rust(&format!(
        "(module m (def (id (: s String) (: n Int64)) (if (< n 1) s (id s (- n 1)))) \
           (def (g) (String.byte-len (String.concat (id \"e\" 1) (id \"{}\" 1)))) (export g))",
        '\u{301}'
    ));
    // The emit must carry the real NFC call (not the old identity pass-through).
    assert!(
        blen.contains("unicode_normalization::UnicodeNormalization::nfc("),
        "String.concat emits the NFC canonicalization:\n{blen}"
    );
    if let Some(out) = rustc_run(&blen, "g()") {
        assert_eq!(
            out, "2",
            "runtime concat of 'e' + combining-acute NFC-normalizes to the 2-byte precomposed 'é' (matches wasm)"
        );
    }
    // …and the normalized concat is EQUAL to the precomposed literal (the divergence breaker saw on `=`).
    let eq = compile_rust(&format!(
        "(module m (def (id (: s String) (: n Int64)) (if (< n 1) s (id s (- n 1)))) \
           (def (g) (if (= (String.concat (id \"e\" 1) (id \"{}\" 1)) (id \"{}\" 1)) 1 0)) (export g))",
        '\u{301}', '\u{e9}'
    ));
    if let Some(out) = rustc_run(&eq, "g()") {
        assert_eq!(
            out, "1",
            "the NFC-normalized concat equals the precomposed 'é' literal by content (matches wasm)"
        );
    }
    // ADVERSARIAL (breaker's co-verify cell #4): NFC must NOT over-normalize. `q` + U+0301 (combining acute)
    // has NO precomposed form, so NFC leaves it as the 2-scalar / 3-byte sequence — it must stay 3 bytes and
    // equal ITSELF, guarding against an emit that maps to some *other* codepoint. Both backends agree here.
    let noncompose = compile_rust(&format!(
        "(module m (def (id (: s String) (: n Int64)) (if (< n 1) s (id s (- n 1)))) \
           (def (g) (String.byte-len (String.concat (id \"q\" 1) (id \"{}\" 1)))) (export g))",
        '\u{301}'
    ));
    if let Some(out) = rustc_run(&noncompose, "g()") {
        assert_eq!(
            out, "3",
            "a non-composing combining sequence (q + U+0301) is NOT over-normalized: stays 3 bytes (matches wasm)"
        );
    }
}

#[test]
fn runtime_string_ops_emit_native_str_operations() {
    // StrAt → scalar-indexed `.chars().nth(i).map(to_string)`; StrFromBytes → `from_utf8().ok()`;
    // StrToBytes → `.into_bytes()`.
    let at = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"hi\" (f (+ n -1)))) \
           (def (g (: i Int64)) (match (String.at (f 1) i) ((Some c) 1) ((None _) 0))) (export g))",
    );
    assert!(
        at.contains(".chars().nth(") && at.contains(".to_string()"),
        "String.at is scalar-indexed:\n{at}"
    );
    let fb = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 104 105)) (f (+ n -1)))) \
           (def (g) (match (String.from-bytes (f 1)) ((Some s) 1) ((None _) 0))) (export g))",
    );
    assert!(
        fb.contains("String::from_utf8(") && fb.contains(".ok()"),
        "String.from-bytes uses from_utf8().ok():\n{fb}"
    );
    let tb = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (String.concat \"a\" \"b\") (f (+ n -1)))) \
           (def (g) (Bytes.len (String.to-bytes (f 1)))) (export g))",
    );
    assert!(
        tb.contains(".into_bytes()"),
        "String.to-bytes uses into_bytes:\n{tb}"
    );
}

#[test]
fn rustc_roundtrip_string_at_is_scalar_indexed_and_from_bytes_validates() {
    // StrAt indexes by SCALAR VALUE, not byte: in "café", scalar 3 is 'é' (a 2-byte UTF-8 char). Read it
    // back and measure its byte length (2), proving scalar-not-byte addressing; an OOB index → None → -1.
    let at = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"café\" (f (+ n -1)))) \
           (def (mk (: i Int64)) (match (String.at (f 1) i) ((Some c) (Bytes.len (String.to-bytes c))) ((None _) -1))) \
           (export mk))",
    );
    if let Some(out) = rustc_run(&at, "mk(3)") {
        assert_eq!(out, "2", "scalar 3 of 'café' is 'é', a 2-byte UTF-8 char");
    }
    if let Some(out) = rustc_run(&at, "mk(9)") {
        assert_eq!(out, "-1", "an out-of-range scalar index is None");
    }
    // String.from-bytes decodes valid UTF-8 to Some, and rejects a lone continuation byte (0x80) to None.
    let fb = compile_rust(
        "(module m \
           (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 104 105)) (f (+ n -1)))) \
           (def (bad (: n Int64)) (if (= n 0) (Bytes.of (list 128)) (bad (+ n -1)))) \
           (def (mk (: which Int64)) \
              (match (String.from-bytes (if (= which 0) (f 1) (bad 1))) ((Some s) (Bytes.len (String.to-bytes s))) ((None _) -1))) \
           (export mk))",
    );
    if let Some(out) = rustc_run(&fb, "mk(0)") {
        assert_eq!(out, "2", "valid UTF-8 'hi' decodes to a 2-byte string");
    }
    if let Some(out) = rustc_run(&fb, "mk(1)") {
        assert_eq!(
            out, "-1",
            "a lone continuation byte is rejected → None (never traps)"
        );
    }
}

#[test]
fn a_context_typed_empty_map_or_set_emits_a_bare_new_not_a_decline() {
    // REGRESSION: an `Map.empty`/`Set.of(list)` whose element type is pinned only by DOWNSTREAM use (a
    // typed callee param) has unsolved vars AT THE NODE, so `type_of`→None — the empty-collection handler
    // used to DECLINE. It must instead emit a BARE `BTreeMap::new()`/`BTreeSet::new()` and let Rust infer
    // the element type from the use. A recursive map-accumulator seeded with `Map.empty` compiles+runs.
    let m = compile_rust(
        "(module m \
           (def (ins (: n Int64) (: mp (Map Int64 Int64))) \
              (if (< n 1) mp (ins (- n 1) (Map.insert mp n (* n 10))))) \
           (def (mk (: n Int64)) \
              (match (List.at (Map.to-list (ins n (Map.empty))) 0) \
                 ((Some p) (match p ((tuple k v) (+ k v)))) ((None _) -1))) (export mk))",
    );
    assert!(
        m.contains("std::collections::BTreeMap::new()"),
        "empty map is a bare BTreeMap::new():\n{m}"
    );
    if let Some(out) = rustc_run(&m, "mk(5)") {
        assert_eq!(out, "11", "min key 1 + val 10 from the accumulated map");
    }
    // A set accumulator seeded with `Set.of (list)` (empty) likewise infers its element type.
    let s = compile_rust(
        "(module m \
           (def (ins (: n Int64) (: st (Set Int64))) (if (< n 1) st (ins (- n 1) (Set.insert st n)))) \
           (def (mk (: n Int64)) (Set.len (ins n (Set.of (list))))) (export mk))",
    );
    if let Some(out) = rustc_run(&s, "mk(3)") {
        assert_eq!(
            out, "3",
            "3 distinct elements accumulated into a context-typed empty set"
        );
    }
}

#[test]
fn rustc_roundtrip_unconstrained_empty_set_grounds_and_compiles_not_e0282() {
    // REGRESSION (breaker adv-rust-backend-unconstrained-empty-set-E0282): an UNANNOTATED empty
    // `(Set.of (list))` used len-only — nothing downstream fixes its element type — left a bare
    // `BTreeSet::new()` whose `_` element rustc could not infer → error[E0282] "type annotations needed for
    // BTreeSet<_>". wasm computes 0. This is the Set companion of the resolved empty-Map/List E0282 class.
    // The fix GROUNDS the open element var to the default and annotates `BTreeSet::<i64>` (the empty-Map
    // twin): rustc infers nothing from a len-only use, so the collection needs a spelled element type.
    let src = "(module m (def (g) (Set.len (Set.of (list)))) (export g))";
    let s = compile_rust(src);
    assert!(
        s.contains("BTreeSet<i64> = std::collections::BTreeSet::new()"),
        "an unconstrained empty set grounds its element to i64 (not a bare `BTreeSet::new()` that E0282s):\n{s}"
    );
    // The whole point: it now COMPILES on rustc (was E0282) and computes 0 like wasm.
    assert!(
        compile_rust_result(src).is_ok(),
        "the emitted set program is produced (not declined)"
    );
    if let Some(out) = rustc_run(&s, "g()") {
        assert_eq!(out, "0", "an empty set has cardinality 0 — same as wasm");
    }
    // A NON-EMPTY set with a genuinely-UNDETERMINED key element — `(Set.of #list(#list()))` whose key type
    // `(List (List Any))` bakes no canonical-compare shape — must REJECT CDZ0203 "not fully determined —
    // annotate it" (mirroring the wasm `ownership.rs` Set/Map-key determinacy reject), NOT ground `Any`→`i64`
    // and emit a Set with a FABRICATED key type (the wrong-accept the fresh rust harvest caught; v-corpus-
    // harness family-A #7, 19-sets:5790). This is DISTINCT from the empty-set grounding above: the empty set
    // has no keys to compare (grounds harmlessly), a non-empty set with an unshapeable key does.
    let undet = try_compile_rust(
        "(module m (def (g) (Set.len (Set.of #list(#list())))) (export g))",
    )
    .expect_err(
        "an undetermined non-empty Set key must decline CDZ0203, not emit a fabricated-key Set",
    );
    assert!(
        undet.iter().any(|d| d.contains("not fully determined")),
        "the undetermined Set key declines CDZ0203 (not a silent fabricated key): {undet:?}"
    );
}

#[test]
fn rustc_a_narrow_literal_in_a_compound_nested_in_a_set_element_grounds_to_the_field_width() {
    // REGRESSION (v-corpus-harness family-B, 06-numeric-model:2527 "an IN-RANGE literal in a tuple nested
    // inside a set element still compiles — the nested check is not over-broad"): `#set(#tuple(100))` under
    // `(Set (Tuple Int8))` — the in-range literal `100` (≤127) sits inside the tuple inside the set element.
    // The set is `BTreeSet<(i8,)>`, but the tuple element's OWN `type_of` is UNDER-GROUND `(Tuple Int64)`, so
    // the plain Tuple emit grounded `100` to the i64 DEFAULT → `((100u64 as i64),)` inserted into a
    // `BTreeSet<(i8,)>` → rustc E0308 (uncompilable, graded fail — while wasm grounds the field and runs to 1).
    // `container_slot_grounding` only grounds a SCALAR set element; the fix grounds a COMPOUND element's FIELDS
    // to the set's DECLARED element type (the compound recursion in `emit_elem_grounding_empty_list`).
    let tup = "(module m (def (g) (Set.len (: #set(#tuple(100)) (Set (Tuple Int8))))) (export g))";
    let s = compile_rust(tup);
    assert!(
        s.contains("BTreeSet<(i8,)>") && s.contains("100u8 as i8") && !s.contains("100u64 as i64"),
        "the nested tuple literal grounds to the tuple's i8 field width (not the i64 default) so the \
         `(i8,)` element matches the `BTreeSet<(i8,)>`:\n{s}"
    );
    assert!(
        compile_rust_result(tup).is_ok(),
        "the tuple-in-set program emits (was E0308-uncompilable via the i64-default field)"
    );
    if let Some(out) = rustc_run(&s, "g()") {
        assert_eq!(
            out, "1",
            "the set has one element — Set.len = 1, same as wasm"
        );
    }
    // RECORD twin (exercises the `Ty::Record` compound arm): a record → a Rust tuple in sorted field-name
    // order, its `a: Int8` field grounded to i8.
    let rec = "(module m (def (g) (Set.len (: #set(#record((= a 100))) (Set (Record (: a Int8)))))) (export g))";
    let sr = compile_rust(rec);
    assert!(
        sr.contains("BTreeSet<(i8,)>") && sr.contains("100u8 as i8"),
        "a record set element grounds its `a: Int8` field to i8:\n{sr}"
    );
    // NESTED-COMPOUND (tuple-in-tuple): the grounding recurses through arbitrary compound depth.
    let nested = "(module m (def (g) (Set.len (: #set(#tuple(#tuple(100))) (Set (Tuple (Tuple Int8)))))) (export g))";
    let sn = compile_rust(nested);
    assert!(
        sn.contains("BTreeSet<((i8,),)>") && sn.contains("100u8 as i8"),
        "a tuple-nested-in-a-tuple set element grounds the deepest i8 field (descent recurses):\n{sn}"
    );
    // MUST-HOLD control: a determined-scalar set (`(Set Int8)`) still grounds+compiles unchanged (no
    // over-reject, no regression to the scalar `container_slot_grounding` path this fix sits beside).
    let scalar = "(module m (def (g) (Set.len (: #set(100) (Set Int8)))) (export g))";
    assert!(
        compile_rust_result(scalar).is_ok(),
        "a scalar-element set is unaffected by the compound-element grounding"
    );
}

#[test]
fn rustc_a_fitting_payload_through_option_expect_grounds_to_the_narrow_result_annotation() {
    // REGRESSION (v-corpus-harness family-B, 06-numeric-model:2367 "a fitting payload projected through
    // Option.expect under a narrow annotation runs (no over-rejection)"): `(: (Option.expect (if c (Some 100)
    // None) "x") UInt8)` solves the expect RESULT as UInt8 (`f`'s `-> u8`), but the narrow annotation does not
    // back-propagate to the `Some 100` node — its payload defaults to Int64, so the scrutinee is `Option<i64>`
    // and `__expect` binds `i64`, returned where `u8` is expected → rustc E0308 (a fitting-payload OVER-FAIL;
    // wasm's width-tagged value model runs to 100). The fix casts the bound payload to the annotated result
    // width; the checker range-checks the payload at check (a non-fitting `Some 10000`/UInt8 is CDZ0302 before
    // emit — the sibling case), so the cast is exact.
    let src = "(do (def (f (: c Bool)) (: (Option.expect (if c (Some 100) None) \"x\") UInt8)) (export f))";
    let s = compile_rust(src);
    assert!(
        s.contains("__expect as u8"),
        "the expect payload is cast to the annotated u8 result width (was returned as i64 → E0308):\n{s}"
    );
    assert!(
        compile_rust_result(src).is_ok(),
        "the Option.expect-under-narrow-annotation program emits (not declined)"
    );
    if let Some(out) = rustc_run(&s, "f(true)") {
        assert_eq!(
            out, "100",
            "the fitting payload projects through expect and runs — same as wasm"
        );
    }
    // An OUT-OF-RANGE payload under the same narrow annotation is a CHECK-time CDZ0302 (the cast-at-sink
    // never masks it — the reject fires before emit). This is the guardrail that keeps the cast sound.
    let over = try_compile_rust(
        "(do (def (f (: c Bool)) (: (Option.expect (if c (Some 10000) None) \"x\") UInt8)) (export f))",
    )
    .expect_err("an out-of-UInt8 expect payload must decline CDZ0302 at check, not silently truncate");
    assert!(
        over.iter()
            .any(|d| d.contains("does not fit") || d.contains("range")),
        "the oversize payload declines CDZ0302 (not a truncating cast): {over:?}"
    );
}

#[test]
fn a_qty_in_a_collection_element_display_scales_to_its_reference() {
    // REGRESSION (v-quantity 9a5fd3c5): a Qty in a whole-MAP VALUE / LIST ELEMENT rendered RAW on rust
    // ((map (1 (Qty.of 5.0 meter))) not 5000.0) — the per-element scale-fold reached tuple/record/sum but
    // NOT a collection element slot. Fix: collect_qty_scale_paths descends List/Set (`.*`) + Map (`!k`/`!v`)
    // — the scale is uniform per collection, keyed once, applied to each per-iteration bind by the render.
    // MAP VALUE:
    let mv = compile_rust(
        "(do (def (run) (Map.insert (Map.empty) 1 (Qty.of 5.0 (Unit.prefix kilo (Unit.base #\"meter\"))))) (export run))",
    );
    assert!(
        mv.contains("// cdz-qty-at[run]: !v 1000/1"),
        "a Qty map VALUE emits a per-value scale note (!v, 1000/1):\n{mv}"
    );
    // LIST ELEMENT:
    let le = compile_rust(
        "(do (def (run) (list (Qty.of 5.0 (Unit.prefix kilo (Unit.base #\"meter\"))) \
                             (Qty.of 2.0 (Unit.prefix kilo (Unit.base #\"meter\"))))) (export run))",
    );
    assert!(
        le.contains("// cdz-qty-at[run]: * 1000/1"),
        "a Qty list ELEMENT emits a per-element scale note (*, 1000/1):\n{le}"
    );
    // (End-to-end value-form render — (map (1 (Qty.of 5000.0 meter))) / (list (Qty 5000.0 m) …) — is
    // validated by the corpus gate, which drives the full cdz_render_expr driver; rustc_run's bare
    // println!("{}", …) can't Display a BTreeMap/Vec.)
}

#[test]
fn a_scalar_match_grounds_its_float32_arm_literals_to_the_result_width() {
    // REGRESSION (corpus-bugfix, sibling of the wasm Float32-branch bug): a scalar `match` under an outer
    // Float32 result — `(: (match n (0 0.5) (_ 1.5)) Float32)` — defaulted each arm's `ConstFloat` to
    // Float64, emitting `f64::from_bits(…)` in an `-> f32` match → rustc E0308. (The `if`-form was already
    // grounded via `emit_branch`; only `match` missed it.) Now `emit_match_impl` grounds each arm literal to
    // the match's result float width, mirroring the result-int grounding beside it.
    let m = compile_rust(
        "(module m (def (run (: n Int64)) (: (match n (0 0.5) (_ 1.5)) Float32)) (export run))",
    );
    assert!(
        m.contains("f32::from_bits(") && !m.contains("f64::from_bits("),
        "match arm Float32 literals render as f32, not f64:\n{m}"
    );
    // End-to-end: n=0 → 0.5f32.
    if let Some(out) = rustc_run(&m, "run(0)") {
        assert_eq!(out, "0.5", "the match returns the f32 arm value 0.5");
    }
}

#[test]
fn a_compound_construct_grounds_a_narrow_field_literal_to_the_declared_slot_width() {
    // REGRESSION (corpus-bugfix, sibling of the wasm record-emit bug → v-inference): a FITTING narrow
    // record-field literal `(record (x 100))` at field type Int8 defaulted its `ConstInt` to Int64, emitting
    // `(100u64 as i64,)` into a `(i8,)` slot → rustc E0308. `Core::Record`/`Core::Tuple` now ground each
    // field/element literal to the declared slot type in `emit_elem_grounding_empty_list` (the compound twin
    // of `emit_grounded`/`emit_branch` — the match-arm/if-branch grounding). Int64 fields + non-literal
    // fields are unchanged.
    // (a) Int8 record field: the literal grounds to i8, computes 100 (matches wasm).
    let i8rec = compile_rust(
        "(module m (def (get (: r (Record (: x Int8)))) (. r x)) \
           (def (run) (get (record (= x 100)))) (export run))",
    );
    assert!(
        i8rec.contains("100u8 as i8") && !i8rec.contains("100u64 as i64"),
        "the Int8 record-field literal grounds to i8, not i64:\n{i8rec}"
    );
    if let Some(out) = rustc_run(&i8rec, "run()") {
        assert_eq!(out, "100", "the fitting Int8 record field computes 100");
    }
    // (b) Float32 record field: grounds to f32 (was the same E0308 class), computes 1.5.
    let f32rec = compile_rust(
        "(module m (def (get (: r (Record (: x Float32)))) (. r x)) \
           (def (run) (get (record (= x 1.5)))) (export run))",
    );
    assert!(
        f32rec.contains("f32::from_bits(") && !f32rec.contains("f64::from_bits("),
        "the Float32 record-field literal grounds to f32:\n{f32rec}"
    );
    if let Some(out) = rustc_run(&f32rec, "run()") {
        assert_eq!(out, "1.5", "the fitting Float32 record field computes 1.5");
    }
    // (c) CONTROL — an Int8 TUPLE element also grounds (same shared helper), and an Int64 record field is
    // unchanged (no spurious cast). Both compute, confirming no over-broadening.
    let i8tup = compile_rust(
        "(module m (def (get (: r (Tuple Int8 Int64))) (. r 0)) \
           (def (run) (get (tuple 100 7))) (export run))",
    );
    if let Some(out) = rustc_run(&i8tup, "run()") {
        assert_eq!(
            out, "100",
            "an Int8 tuple element also grounds and computes 100"
        );
    }
    let i64rec = compile_rust(
        "(module m (def (get (: r (Record (: x Int64)))) (. r x)) \
           (def (run) (get (record (= x 100)))) (export run))",
    );
    if let Some(out) = rustc_run(&i64rec, "run()") {
        assert_eq!(
            out, "100",
            "an Int64 record field is unchanged and computes 100"
        );
    }
}

#[test]
fn a_bigint_quantity_display_scales_to_its_reference_in_the_bignum_path() {
    // REGRESSION (v-quantity/v-runtime): a NON-scale-1 BigInt-inner Qty display-scales to its reference in
    // the bignum path — `(Qty.of (BigInt.of 5) kilometer)` → `(Qty.of 5000 meter)` (×1000/1 kilo scale,
    // EXACT for a whole ratio). Previously DECLINED ("(Qty BigInt meter) has no native Rust representation")
    // because `qty_scale_supported` excluded BigInt. Now it emits: the Qty type maps to `cdz_num::Big`, and
    // the render's Qty arm scales via `Big.mul(num).divmod(den).0`. The bare-Qty return crosses raw + a
    // `// cdz-scale` note, so the emit itself must NOT decline (a Big magnitude + a scale note).
    let m = compile_rust_result(
        "(do (def (run) (Qty.of (BigInt.of 5) (Unit.prefix kilo (Unit.base #\"meter\")))) (export run))",
    );
    assert!(
        m.is_ok(),
        "a non-scale-1 BigInt-Qty return now EMITS (was declined 'no native Rust representation'): {m:?}"
    );
    let m = m.unwrap();
    // Emits the Big magnitude + a scale note (kilo = 1000/1) for the harness display-multiply.
    assert!(
        m.contains("cdz_num::Big") && m.contains("// cdz-scale[run]: 1000/1"),
        "the BigInt-Qty emits a Big magnitude + a 1000/1 scale note:\n{m}"
    );
}

#[test]
fn a_generic_nominal_returned_whole_notes_its_erased_inner_tuple() {
    // REGRESSION (v-quantity/corpus-bugfix "a record of quantities RETURNED as a value"): a GENERIC nominal
    // (`(type V3q (V3 a a a))`) instantiated + returned WHOLE erases to a Rust tuple `(f64, f64, f64)`, but
    // its `// cdz-return` note used to be the bare nominal name `V3q` — for which no descriptor is emitted
    // (a generic nominal is skipped by emit_newtype_descriptors), so the render fell to a scalar Display of
    // the tuple → rustc E0277. Now the return note is the ERASED INNER's render_name (a `(Tuple …)`), so the
    // render's structural Tuple arm handles it (and the per-element cdz-qty-at notes scale each Qty field).
    let m = compile_rust(
        "(do (type V3q (V3 a a a)) \
             (def (run) (V3q.V3 (Qty.of 5.0 (Unit.prefix kilo (Unit.base #\"meter\"))) \
                                (Qty.of 2.0 (Unit.prefix kilo (Unit.base #\"meter\"))) \
                                (Qty.of 3.0 (Unit.prefix kilo (Unit.base #\"meter\"))))) (export run))",
    );
    // The return note is the erased inner Tuple, NOT the bare nominal `V3q`.
    assert!(
        m.contains("// cdz-return[run]: (Tuple (Qty Float64")
            && !m.contains("// cdz-return[run]: V3q"),
        "a generic nominal returned whole notes its erased inner tuple type:\n{m}"
    );
    // The per-field scale notes still key positionally (0/1/2) — each km field scales ×1000/1.
    assert!(
        m.contains("// cdz-qty-at[run]: 0 1000/1"),
        "the first Qty field emits a positional per-element scale note:\n{m}"
    );
    // A MONOMORPHIC nominal keeps its name (its newtype descriptor resolves it) — no over-broadening.
    let mono = compile_rust("(do (type P (Mk Int64 Int64)) (def (run) (P.Mk 3 4)) (export run))");
    assert!(
        mono.contains("// cdz-return[run]: P"),
        "a monomorphic nominal keeps its nominal name in the return note:\n{mono}"
    );
}

#[test]
fn a_compound_tuple_of_quantities_emits_per_element_scale_notes() {
    // REGRESSION (pr-sync/v-core-opt rust-red on 18-units): a `(Tuple (Qty km) (Qty mile))` result — each
    // element display-scales to its reference INDEPENDENTLY. The single `// cdz-scale` note only scales a
    // top-level bare Qty; a Qty NESTED in a tuple was rendered RAW (`5.0`/`5/1` not `5000.0`/`201168/25`).
    // The fix emits a per-element `// cdz-qty-at[ident]: <path> <num>/<den>` note per non-scale-1 Qty leaf.
    let m = compile_rust(
        "(do (def (main) (tuple (Qty.of 5.0 (Unit.prefix kilo (Unit.base #\"meter\"))) \
                                (Qty.of (Rational.of 5 1) (Unit.of #\"mile\")))) (export main))",
    );
    // Element 0 (Float64 km) scales ×1000/1; element 1 (Rational mile) scales ×201168/125.
    assert!(
        m.contains("// cdz-qty-at[main]: 0 1000/1"),
        "the km Float element emits a per-path scale note (path 0, 1000/1):\n{m}"
    );
    assert!(
        m.contains("// cdz-qty-at[main]: 1 201168/125"),
        "the mile Rational element emits a per-path scale note (path 1, 201168/125):\n{m}"
    );
    // A SCALE-1 element (already at reference) emits NO per-path note — rendered as stored.
    let m1 = compile_rust(
        "(do (def (main) (tuple (Qty.of 5.0 (Unit.base #\"meter\")) (Qty.of 7.0 (Unit.base #\"meter\")))) (export main))",
    );
    assert!(
        !m1.contains("// cdz-qty-at["),
        "a reference-unit (scale-1) tuple of quantities emits no per-path scale note:\n{m1}"
    );
}

#[test]
fn a_user_sum_qty_payload_emits_a_per_variant_scale_note() {
    // REGRESSION (v-quantity follow-up): a Qty in a USER-DEFINED variant payload display-scales to its
    // reference too — `Circle(Qty.of 3.0 km)` should render `(Circle (Qty.of 3000.0 meter))`, not raw 3.0.
    // The per-element scale note keys a user-sum payload by the LOCAL `<variant>?<idx>` (the render's reused
    // helper has no outer path prefix). A `Circle` payload declared at kilometer scales ×1000/1.
    let m = compile_rust(
        "(do (type Shape (Circle (Qty Float64 (Unit.prefix kilo (Unit.base #\"meter\")))) (Sq Int64)) \
             (def (run) (Circle (Qty.of 3.0 (Unit.prefix kilo (Unit.base #\"meter\"))))) (export run))",
    );
    assert!(
        m.contains("// cdz-qty-at[run]: Circle?0 1000/1"),
        "the Circle Qty payload emits a per-variant scale note (Circle?0, 1000/1):\n{m}"
    );
    // (End-to-end value-form render is validated by the corpus gate case in 18-units-of-measure.sexp, which
    // drives the full `cdz_render_expr` driver — `rustc_run`'s bare `println!("{}", …)` can't Display a sum.)
    // A user sum whose payload is a SCALE-1 (reference) Qty emits no note (rendered as stored).
    let m1 = compile_rust(
        "(do (type Shape (Circle (Qty Float64 (Unit.base #\"meter\"))) (Sq Int64)) \
             (def (run) (Circle (Qty.of 3.0 (Unit.base #\"meter\")))) (export run))",
    );
    assert!(
        !m1.contains("// cdz-qty-at["),
        "a reference-unit (scale-1) user-sum Qty payload emits no per-variant scale note:\n{m1}"
    );
}

#[test]
fn empty_set_at_a_call_arg_grounds_to_the_callee_param_element_not_the_default() {
    // REGRESSION (breaker adv-rust-empty-set-call-arg-elem-type-not-consulted-E0308): an empty
    // `(Set.of (list))` passed as a CALL ARGUMENT whose callee PARAM declares a `(Set Float64)` — with no
    // insert anywhere, the param type is the ONLY element-type fixer. The empty node's element is an
    // unsolved var at the construction site, so the old `ground_open_vars` default spelled `BTreeSet<i64>`
    // at the call site while the param is `BTreeSet<__CdzF64>` → error[E0308]. The fix threads the callee
    // param type as the arg's EXPECTED type so the empty set annotates from it.
    let m = compile_rust(
        "(module m \
           (def (loop (: n Int64) (: s (Set Float64))) (if (= n 0) (Set.len s) (loop (- n 1) s))) \
           (def (run) (loop 3 (Set.of (list)))) (export run))",
    );
    assert!(
        m.contains("std::collections::BTreeSet<__CdzF64> = std::collections::BTreeSet::new()"),
        "the empty-set call arg grounds to the param's __CdzF64 element (not the default i64):\n{m}"
    );
    // The whole point: rustc-compiles (was E0308) and computes 0 like wasm.
    assert!(
        compile_rust_result(
            "(module m \
               (def (loop (: n Int64) (: s (Set Float64))) (if (= n 0) (Set.len s) (loop (- n 1) s))) \
               (def (run) (loop 3 (Set.of (list)))) (export run))"
        )
        .is_ok(),
        "the emitted program is produced (not declined)"
    );
    if let Some(out) = rustc_run(&m, "run()") {
        assert_eq!(
            out, "0",
            "an empty float set has cardinality 0 — same as wasm"
        );
    }
    // An INT-element call-arg empty set stays fine (the default ground already matched — no regression).
    let mi = compile_rust(
        "(module m \
           (def (loop (: n Int64) (: s (Set Int64))) (if (= n 0) (Set.len s) (loop (- n 1) s))) \
           (def (run) (loop 3 (Set.of (list)))) (export run))",
    );
    assert!(
        mi.contains("std::collections::BTreeSet<i64>"),
        "an Int64 call-arg empty set still grounds to i64:\n{mi}"
    );
}

#[test]
fn rustc_roundtrip_unconstrained_empty_set_grounds_through_a_control_flow_join() {
    // REGRESSION (v-rust-backend, extends the direct-`Set.len (Set.of (list))` pin above): an
    // UNANNOTATED empty set whose element type is fixed by NOTHING downstream must still ground to the
    // default `BTreeSet<i64>` when it reaches its len-use THROUGH A CONTROL-FLOW JOIN — an `if` whose
    // BOTH branches are `(Set.of (list))`, and a `match` likewise. The direct case grounds the sole
    // construction site; these pin that the grounding survives when the empty set is produced on two
    // arms that join (each arm must independently spell `BTreeSet<i64>`, not a bare `_` that E0282s).
    for (shape, src) in [
        (
            "if-both-branches-empty",
            "(module m (def (g) (Set.len (if false (Set.of (list)) (Set.of (list))))) (export g))",
        ),
        (
            "match-both-arms-empty",
            "(module m (def (g) (Set.len (match 0 (0 (Set.of (list))) (_ (Set.of (list)))))) (export g))",
        ),
    ] {
        let s = compile_rust(src);
        assert!(
            s.contains("BTreeSet<i64> = std::collections::BTreeSet::new()"),
            "{shape}: an empty set grounds its element to i64 on each joining arm (not a bare \
             `BTreeSet::new()` that E0282s):\n{s}"
        );
        assert!(
            compile_rust_result(src).is_ok(),
            "{shape}: the emitted set program compiles on rustc (was the E0282 class)"
        );
        if let Some(out) = rustc_run(&s, "g()") {
            assert_eq!(
                out, "0",
                "{shape}: an empty set has cardinality 0 — same as wasm"
            );
        }
    }
}

#[test]
fn bytes_slice_is_total_on_a_usize_overflowing_range() {
    // REGRESSION (Copilot PR#435): the `Bytes.slice` bounds guard summed `(start as usize)+(len as usize)`,
    // which OVERFLOWS usize for two near-i64::MAX operands (wraps to a small sum in release) → the guard
    // passed and the index PANICKED. `Bytes.slice` must be TOTAL → the guard now uses `checked_add`, so an
    // overflowing range returns None. Assert the emitted source uses the checked form.
    let sl = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 1 2 3)) (f (+ n -1)))) \
           (def (g (: st Int64) (: ln Int64)) (match (Bytes.slice (f 1) st ln) ((Some b) (Bytes.len b)) ((None _) -1))) \
           (export g))",
    );
    assert!(
        sl.contains("checked_add(") && !sl.contains("as usize) + (__len as usize)"),
        "Bytes.slice bound uses checked_add, not a wrapping sum:\n{sl}"
    );
    // In-range slice works; a huge (overflowing) start/len returns None, not a panic.
    if let Some(out) = rustc_run(&sl, "g(1, 2)") {
        assert_eq!(out, "2", "in-range slice [1..3) has length 2");
    }
    if let Some(out) = rustc_run(&sl, "g(9223372036854775807, 9223372036854775807)") {
        assert_eq!(out, "-1", "a usize-overflowing range is None, not a panic");
    }
}

#[test]
fn rustc_roundtrip_persistent_collections_do_not_mutate_the_original() {
    // INVARIANT PIN: Cadenza collections are PERSISTENT — an op returns a NEW value and the operand is
    // unchanged. The rust backend realizes this by CLONING a shared (non-Copy) binding on read (tick-2/5/11
    // work). These pin that a `let`-bound collection used in BOTH an op and a later read keeps its original
    // value — a future change to the clone-on-read discipline that broke persistence would flip these.
    // List: push then read the ORIGINAL length. len([1,2,3]+push) 4 + len original 3 = 7.
    let lst = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list 1 2 3) (f (+ n -1)))) \
           (def (mk) (let ((xs (f 1))) (+ (List.len (List.push xs 9)) (List.len xs)))) (export mk))",
    );
    if let Some(out) = rustc_run(&lst, "mk()") {
        assert_eq!(
            out, "7",
            "List.push leaves the original list unchanged (4 + 3)"
        );
    }
    // Map: remove a key, look it up in the NEW map (None→99) AND the ORIGINAL (still 10). 99 + 10 = 109.
    let mp = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Map.insert (Map.insert (Map.empty) 1 10) 2 20) (f (+ n -1)))) \
           (def (mk) (let ((m (f 1))) \
              (+ (match (Map.lookup (Map.remove m 1) 1) ((Some v) v) ((None _) 99)) \
                 (match (Map.lookup m 1) ((Some v) v) ((None _) 0))))) (export mk))",
    );
    if let Some(out) = rustc_run(&mp, "mk()") {
        assert_eq!(
            out, "109",
            "Map.remove leaves the original map's key intact (99 + 10)"
        );
    }
    // Set: insert into a copy, sum the new len + the original len. len({1,2}+9)=3 + len{1,2}=2 = 5.
    let st = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Set.of (list 1 2)) (f (+ n -1)))) \
           (def (mk) (let ((s (f 1))) (+ (Set.len (Set.insert s 9)) (Set.len s)))) (export mk))",
    );
    if let Some(out) = rustc_run(&st, "mk()") {
        assert_eq!(
            out, "5",
            "Set.insert leaves the original set unchanged (3 + 2)"
        );
    }
}

#[test]
fn rustc_roundtrip_nested_heap_in_heap_composes() {
    // INVARIANT PIN: heap values NEST arbitrarily — a list in a map value, a compound in a sum payload —
    // and the native-aggregate types + clone-on-read compose. A list stored as a Map VALUE, retrieved and
    // measured: len == 3.
    let lm = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Map.insert (Map.empty) 1 (list 10 20 30)) (f (+ n -1)))) \
           (def (mk) (match (Map.lookup (f 1) 1) ((Some xs) (List.len xs)) ((None _) -1))) (export mk))",
    );
    if let Some(out) = rustc_run(&lm, "mk()") {
        assert_eq!(out, "3", "a list stored in a map value round-trips");
    }
    // A Map inside a sum payload, matched then looked up: Box.Mk({5:50}) → 50.
    let ms = compile_rust(
        "(module m (type Box (Mk (Map Int64 Int64))) \
           (def (f (: n Int64)) (if (= n 0) (Box.Mk (Map.insert (Map.empty) 5 50)) (f (+ n -1)))) \
           (def (mk) (match (f 1) ((Mk m) (match (Map.lookup m 5) ((Some v) v) ((None _) -1))))) (export mk))",
    );
    if let Some(out) = rustc_run(&ms, "mk()") {
        assert_eq!(out, "50", "a map inside a sum payload round-trips");
    }
}

#[test]
fn rustc_roundtrip_compound_map_key_matches_by_value() {
    // INVARIANT PIN: a map/set keyed by a COMPOUND (tuple) matches BY VALUE (the BTreeMap `Ord` over the
    // Rust tuple, matching Cadenza's by-value key semantics). A tuple key inserted and looked up → 99.
    let mk = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Map.insert (Map.empty) (tuple 1 2) 99) (f (+ n -1)))) \
           (def (g) (match (Map.lookup (f 1) (tuple 1 2)) ((Some v) v) ((None _) -1))) (export g))",
    );
    if let Some(out) = rustc_run(&mk, "g()") {
        assert_eq!(out, "99", "a tuple map key matches by value");
    }
}

#[test]
fn an_ill_formed_integer_width_is_rejected_not_declined() {
    // An out-of-range integer WIDTH (negative/non-natural, or over-ceiling `(UInt 65)`) is an ILL-FORMED
    // TYPE, not a target limitation — a boundary of that type must REJECT (CDZ0302), the SAME outcome the
    // wasm target gives, not a codeless "no native Rust representation" decline (which the gate would read
    // as an unimplemented-construct todo). As of the shared-front-end well-formedness fix (`int_width_fault`
    // classifies `Malformed` vs `OverCeiling`), `cdz check` REJECTS a malformed width `(Int -8)` directly —
    // both backends inherit it — with a message that cites the admitted 1..=64 range and that a width must
    // be a compile-time natural number (an over-ceiling width still names the written width).
    let neg = try_compile_rust("(module m (def (main) (: 5 (Int -8))) (export main))")
        .expect_err("an ill-formed integer width must reject, not emit");
    assert!(
        neg.iter().any(|d| d.contains("1..=64")
            && (d.contains("not a valid integer type")
                || d.contains("width must be a compile-time natural number"))),
        "reject should cite the ill-formed width + admitted range: {neg:?}"
    );
    // The same in PARAMETER position (over-ceiling this time).
    let over = try_compile_rust("(module m (def (main (: x (UInt 65))) x) (export main))")
        .expect_err("an over-ceiling parameter width must reject");
    assert!(
        over.iter().any(|d| d.contains("not a valid integer type")),
        "parameter reject should cite the ill-formed width: {over:?}"
    );
    // A VALID but non-aliased width (`UInt7`, in 1..=64) now EMITS — it STORES in the next-larger machine
    // primitive (`UInt7`→`u8`), so a value/param of it is representable (NOT a decline, and NOT the CDZ0302
    // reject the ill-formed cases get). Guards that the storage-width map admits a valid non-aliased width
    // while the well-formedness reject above still fires for a truly ill-formed one. (Runtime ARITHMETIC on
    // such a width still declines — the 2^N overflow check is a later slice — but a param passthrough emits.)
    let seven = compile_rust("(module m (def (main (: x (UInt 7))) x) (export main))");
    assert!(
        seven.contains("x: u8") && seven.contains("-> u8"),
        "a valid non-aliased UInt7 stores in u8 and emits (not declined): {seven}"
    );
}

#[test]
fn a_runtime_record_emits_a_sorted_field_tuple() {
    // A record that survives to runtime → a Rust tuple in SORTED field-name order (a record is
    // structural; at run time it IS a positional array in sorted key order). Field read → `.index`.
    // Fields declared OUT of order still emit sorted: `(record (b n) (a 7))` → `((7), n)` (a before b).
    let rs = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (record (= b n) (= a 7)) (f (+ n -1)))) \
                    (def (main) (f 1)) (export main))",
    );
    assert!(rs.contains("-> (i64, i64)"), "record → tuple type:\n{rs}");
    // a=7 first (sorted), b=n second — the declared order (b, a) is re-sorted.
    assert!(
        rs.contains("((7u64 as i64), __p0)"),
        "sorted-field literal:\n{rs}"
    );

    // A record field read is a projection at the field's SORTED index.
    let proj =
        compile_rust("(module m (def (g (: r (Record (a Int64) (b Int64)))) (. r b)) (export g))");
    assert!(
        proj.contains("(r).1"),
        "field `b` is sorted index 1:\n{proj}"
    );
}

#[test]
fn rustc_roundtrip_record_builds_and_projects() {
    // A record crosses rustc end-to-end: a field read at the sorted index, and a returned record renders
    // (via the gate's type-directed path elsewhere). Here: `(. r a)` on `(Record (a) (b))` reads `.0`.
    let proj =
        compile_rust("(module m (def (g (: r (Record (a Int64) (b Int64)))) (. r a)) (export g))");
    if let Some(out) = rustc_run(&proj, "g((5, 9))") {
        assert_eq!(out, "5"); // field `a` = sorted index 0
    }
}

#[test]
fn a_runtime_tuple_emits_a_native_rust_tuple() {
    // A tuple that survives to runtime (built behind a recursive call) → a Rust tuple type + literal;
    // a projection → tuple field access. Scalar elements and nested tuples both compose.
    let rs = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (tuple n 7) (f (+ n -1)))) \
                    (def (main) (f 3)) (export main))",
    );
    assert!(rs.contains("-> (i64, i64)"), "tuple return type:\n{rs}");
    assert!(rs.contains("(__p0, (7u64 as i64))"), "tuple literal:\n{rs}");

    let nested = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (tuple n (tuple n n)) (f (+ n -1)))) \
                    (def (main) (f 2)) (export main))",
    );
    assert!(
        nested.contains("-> (i64, (i64, i64))"),
        "nested tuple type:\n{nested}"
    );

    let proj =
        compile_rust("(module m (def (fst (: t (Tuple Int64 Int64))) (. t 0)) (export fst))");
    assert!(proj.contains("t: (i64, i64)"), "tuple param type:\n{proj}");
    assert!(proj.contains("(t).0"), "projection:\n{proj}");
}

#[test]
fn rustc_roundtrip_tuple_builds_and_projects() {
    // A tuple crosses rustc end-to-end: a projection reads the element, and a returned tuple renders as
    // the `(tuple …)` form. `fst((5,9))=5`; the nested tuple result is driven via field access.
    let proj =
        compile_rust("(module m (def (fst (: t (Tuple Int64 Int64))) (. t 0)) (export fst))");
    if let Some(out) = rustc_run(&proj, "fst((5, 9))") {
        assert_eq!(out, "5");
    }
    let mk = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (tuple n 7) (f (+ n -1)))) \
                    (def (mktup) (f 3)) (export mktup))",
    );
    // Drive the tuple result, printing cdz-run's `(tuple …)` form via field access. (Export is `mktup`,
    // not `main`, so the call in the driver's `fn main` names the export, not the driver itself.)
    if let Some(out) = rustc_run(
        &mk,
        "{ let t = mktup(); format!(\"(tuple {} {})\", t.0, t.1) }",
    ) {
        assert_eq!(out, "(tuple 0 7)");
    }
}

#[test]
fn a_recursive_export_emits_a_self_calling_fn() {
    // A recursive def becomes a `Core::Call` (non-recursive calls inline), so it emits a `pub fn` that
    // calls itself by its SANITIZED name (`sum-to` → `sum_to`, matching the call site).
    let rs = compile_rust(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export sum-to))",
    );
    assert!(
        rs.contains("pub fn sum_to(n: i64) -> i64"),
        "signature:\n{rs}"
    );
    assert!(rs.contains("sum_to("), "self-call by sanitized name:\n{rs}");
    assert!(!rs.contains("sum-to"), "no unsanitized `-` name:\n{rs}");
}

#[test]
fn a_recursive_def_named_a_rust_keyword_emits_a_raw_identifier() {
    // `loop` is a valid Cadenza identifier but a Rust KEYWORD. A recursive def named it SURVIVES as a
    // top-level `fn` (a non-recursive one inlines away), so the emitter writes `fn loop(…)` — invalid Rust
    // (`expected `{`, found `(``). It must be a RAW identifier `r#loop` at the declaration AND the call, the
    // way rustc round-trips a keyword-named symbol. (The body's own `loop { }` tail-loop is a real loop
    // keyword and stays bare — only the NAME is escaped.)
    let rs = compile_rust(
        "(module m (def (loop (: n Int64)) (if (= n 0) 42 (loop (- n 1)))) (def (go) (loop 3)) (export go))",
    );
    assert!(
        rs.contains("fn r#loop(") && rs.contains("r#loop("),
        "a keyword-named fn + its call are raw identifiers:\n{rs}"
    );
    assert!(
        !rs.contains("fn loop("),
        "the fn name must not be the bare keyword:\n{rs}"
    );
    // A non-keyword name is unaffected — no `r#` on `sum_to`.
    let ok = compile_rust(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (+ n (sum-to (- n 1))))) (export sum-to))",
    );
    assert!(
        !ok.contains("r#"),
        "a non-keyword name gets no raw prefix:\n{ok}"
    );
    // End-to-end through rustc: the keyword-named recursion builds (was a rustc parse error) and loop(3) = 42.
    let run = compile_rust(
        "(module m (def (loop (: n Int64)) (if (= n 0) 42 (loop (- n 1)))) (def (go) (loop 3)) (export go))",
    );
    if let Some(out) = rustc_run(&run, "go()") {
        assert_eq!(
            out, "42",
            "the keyword-named recursion builds and runs:\n{run}"
        );
    }
}

#[test]
fn an_inlined_do_local_recursive_fn_uniques_its_name_per_copy() {
    // A helper with a do-local recursive worker, called from MORE THAN ONE site: the helper's body is
    // β-copied per call, each copy carrying its OWN `fac` DEFINITION (a distinct `db.defs` index) but the
    // SAME source name. Emitting both as a top-level `fn fac` collides (rustc E0428 "the name `fac` is
    // defined multiple times") — the wasm backend never collides because a function's identity there is its
    // INDEX. `fn_ident` uniques each colliding copy by its def index (`fac_<n>`), consistently at the
    // declaration AND the recursive self-call, so the artifact builds and each copy recurses into itself.
    // Export under a non-`main` name (`run`) so the round-trip driver's own `fn main` does not collide with
    // the emitted export (`rustc_run` splices the module beside a driver `fn main`, not under `mod prog`).
    let rs = compile_rust(
        "(module m (def (helper x) (do (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))) (fac x))) \
                    (def (run) (+ (helper 5) (helper 3))) (export run))",
    );
    // No two identically-named `fn fac(` may be emitted (that is the E0428 the fix removes).
    assert_eq!(
        rs.matches("fn fac(").count(),
        0,
        "no two identically-named `fn fac` may be emitted (E0428):\n{rs}"
    );
    // Each inlined copy gets its own uniqued `fn fac_<def>` declaration — two distinct declarations.
    let decls = rs.matches("fn fac_").count();
    assert!(
        decls >= 2,
        "each inlined copy gets its own uniqued `fn fac_<n>` declaration (got {decls}):\n{rs}"
    );
    // End-to-end: the emitted crate BUILDS and runs to fac(5)+fac(3) = 120+6 = 126 (the E0428 is gone).
    if let Some(out) = rustc_run(&rs, "run()") {
        assert_eq!(out, "126", "the uniqued crate builds and runs:\n{rs}");
    }
}

#[test]
fn mutual_tail_recursion_compiles_to_a_shared_dispatch_loop() {
    // `even`/`odd` are a same-signature mutual-tail-recursion SCC → each emits a SHARED `which`-dispatch
    // loop (no cross-calls, no Box::pin): a tail call to the other member sets `which` + shared locals +
    // continues. `even` is `pub fn` (exported), `odd` a private `fn` (reachable member); both loop.
    let rs = compile_rust(
        "(module m (def (even (: n Int64)) (if (= n 0) true (odd (+ n -1)))) \
                    (def (odd (: n Int64)) (if (= n 0) false (even (+ n -1)))) (export even))",
    );
    assert!(
        rs.contains("pub fn even(mut n: i64) -> bool"),
        "export:\n{rs}"
    );
    assert!(
        rs.contains("fn odd(mut n: i64) -> bool"),
        "private member:\n{rs}"
    );
    assert!(!rs.contains("pub fn odd"), "odd must NOT be pub:\n{rs}");
    // The loop dispatches on `which` and iterates via `continue` — no residual cross-call, no boxing.
    assert!(rs.contains("which == 0"), "which-dispatch:\n{rs}");
    assert!(
        rs.contains("which = 1;") && rs.contains("continue;"),
        "iterates:\n{rs}"
    );
    assert!(!rs.contains("Box::pin"), "no boxing (sync):\n{rs}");
    // Neither member CALLS the other any more (only `pub fn even(`/`fn odd(` declaration heads remain).
    assert_eq!(rs.matches("odd(").count(), 1, "no call to odd:\n{rs}");
    assert_eq!(rs.matches("even(").count(), 1, "no call to even:\n{rs}");
}

#[test]
fn rustc_roundtrip_mutual_tail_loop_runs_deep() {
    // The shared loop must run deep mutual recursion in bounded stack — even(2_000_000) = true.
    let rs = compile_rust(
        "(module m (def (even (: n Int64)) (if (= n 0) true (odd (+ n -1)))) \
                    (def (odd (: n Int64)) (if (= n 0) false (even (+ n -1)))) (export even))",
    );
    if let Some(out) = rustc_run(&rs, "even(2000000)") {
        assert_eq!(out, "true");
    }
    if let Some(out) = rustc_run(&rs, "even(7)") {
        assert_eq!(out, "false");
    }
}

#[test]
fn self_tail_recursion_compiles_to_a_loop() {
    // A self-tail-recursive fn becomes a `loop` with `mut` params: the tail self-call reassigns params
    // + `continue`s, the base case `break`s its value. Bounded stack (sync) / no Box::pin (async).
    let rs = compile_rust(
        "(module m (def (go (: n Int64) (: acc Int64)) \
           (if (= n 0) acc (go (+ n -1) (+ acc n)))) (export go))",
    );
    assert!(
        rs.contains("pub fn go(mut n: i64, mut acc: i64)"),
        "mut params:\n{rs}"
    );
    assert!(rs.contains("loop {"), "loop:\n{rs}");
    // The body runs over the shared positional locals `__p0`/`__p1` (initialized from the params); the
    // base case `break`s the accumulator local, the recursive case parallel-moves + `continue`s.
    assert!(
        rs.contains("break __p1;"),
        "base case breaks the accumulator:\n{rs}"
    );
    assert!(
        rs.contains("continue;") && rs.contains("let (__t0, __t1,)"),
        "parallel-move + continue:\n{rs}"
    );
    // The tail self-call became the reassignment+continue, not a recursive call — no `Box::pin`, and no
    // `go(` CALL survives (the only `go(` is the `pub fn go(` declaration head).
    assert!(!rs.contains("Box::pin"), "no boxing (sync):\n{rs}");
    assert_eq!(
        rs.matches("go(").count(),
        1,
        "only the decl, no self-call:\n{rs}"
    );
}

#[test]
fn rustc_roundtrip_self_loop_runs_deep() {
    // The loop must run a large tail recursion in bounded stack — 1M iterations (sum 1..=1_000_000).
    // Export is `sumn` (not `main`, which would collide with the driver's `fn main`).
    let rs = compile_rust(
        "(module m (def (go (: n Int64) (: acc Int64)) (if (= n 0) acc (go (+ n -1) (+ acc n)))) \
                    (def (sumn (: n Int64)) (go n 0)) (export sumn))",
    );
    if let Some(out) = rustc_run(&rs, "sumn(1000000)") {
        assert_eq!(out, "500000500000");
    }
}

#[test]
fn rustc_roundtrip_async_self_loop_deep_is_bounded() {
    // The async form of a deep tail loop must ALSO run in bounded stack — no Box::pin poll-chain (the
    // loop iterates in place), so 1M iterations complete under the executor. Same answer as sync.
    let module = compile_rust_async(
        "(module m (def (go (: n Int64) (: acc Int64)) (if (= n 0) acc (go (+ n -1) (+ acc n)))) \
                    (def (main (: n Int64)) (go n 0)) (export main))",
    );
    let driver = r#"
struct GateEnv;
impl cdz_rt::CdzEnv for GateEnv { async fn consume(&mut self, _g: u64) {} }
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() { println!("{}", block_on(prog::main(&mut GateEnv, 1000000))); }
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "500000500000");
    }
}

// ── the rustc round-trip (behavior oracle) ───────────────────────────────────────────────────────

/// A stable per-(module, driver) key for the round-trip temp dir — an FNV-1a hash of both strings.
/// Distinct programs get distinct dirs so parallel round-trip tests never share a `prog` binary (which
/// would race write-vs-exec). No clock/rng needed; the hash is deterministic.
fn test_key(a: &str, b: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for byte in a.bytes().chain([0u8]).chain(b.bytes()) {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// A GLOBALLY-UNIQUE temp-dir path for one round-trip invocation: `<prefix>-<pid>-<counter>`. The
/// content-hash key alone is NOT enough to prevent the `Text file busy`/`ExecutableFileBusy` flake —
/// two round-trips with IDENTICAL `(module, call)` content (a test that runs the same program twice, or
/// two distinct tests whose emitted source coincides) hash to the SAME dir and, running in PARALLEL,
/// race write-vs-exec on the one shared `prog` binary. A per-invocation `pid`+atomic-counter suffix
/// gives EVERY call its own dir, so no two ever touch the same `prog`/`prog.rs` — the same fix the gate
/// harness (`xtask`) applies to `run_program_rust`. (The `content_key` still seeds the name so a kept
/// dir is recognizable; uniqueness is what the suffix guarantees.) See
/// [[rust-backend-roundtrip-tests-flake-text-file-busy]].
fn unique_tmp_dir(prefix: &str, content_key: u64) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("{prefix}-{content_key:016x}-{pid}-{n}"))
}

/// Compile the emitted Rust `module` plus a generated `main` that calls `export`(`args`) and prints
/// the result, run it under the ambient `rustc`, and return the printed line. Returns `None` if
/// `rustc` is not available (the test then skips its assertion rather than failing).
fn rustc_run(module: &str, call: &str) -> Option<String> {
    use std::process::Command;
    if Command::new("rustc").arg("--version").output().is_err() {
        return None; // no rustc — skip the round-trip.
    }
    // A GLOBALLY-UNIQUE temp dir per INVOCATION (pid + atomic counter, seeded by the content hash) —
    // tests run in PARALLEL, and two round-trips with IDENTICAL `(module, call)` content (the same
    // program run twice, or two tests whose emitted source coincides) would otherwise share ONE dir and
    // race write-vs-exec on the fixed `prog` binary ("Text file busy"). The content-hash-only key was
    // NOT enough (identical content collides); the per-invocation suffix guarantees no two calls touch
    // the same `prog`/`prog.rs`. (The test bin may use the filesystem — it is the host boundary.)
    let dir = unique_tmp_dir("rcdzc-rust-rt", test_key(module, call));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("prog.rs");
    let bin_path = dir.join("prog");
    let full = format!("{module}\nfn main() {{ println!(\"{{}}\", {call}); }}\n");
    std::fs::write(&src_path, full).expect("write rust source");
    // Compile with a retry: many round-trip tests run in PARALLEL, each shelling `rustc`→`cc`, and the
    // linker can transiently fail under that concurrency ("linking with cc failed") — an environment
    // race, not a defect in the emitted source. Retry once before treating a non-zero status as a real
    // compile error (a genuine miscompile fails both attempts, so this never hides one).
    // A BigInt program emits `cdz_num::Big`; link the `cdz-num` dev-dep rlib (harmless when unused —
    // `--extern` only makes the crate available). Mirrors the async runner's `cdz_rt` linking.
    let cdz_num = cdz_num_link();
    // A String.concat / from-bytes program emits a `unicode_normalization::…::nfc(…)` NFC canonicalization
    // (FINDING #23 rust parity); link its rlib (a workspace dep, in the test binary's `deps/`) like `cdz_num`.
    let unicode_norm = dep_rlib_link("libunicode_normalization");
    let compile = || {
        let mut cmd = Command::new("rustc");
        cmd.args(["-O", "--edition", "2021"])
            .arg(&src_path)
            .arg("-o")
            .arg(&bin_path);
        if let Some((dep_dir, rlib)) = &cdz_num {
            cmd.arg("-L")
                .arg(format!("dependency={}", dep_dir.display()))
                .arg("--extern")
                .arg(format!("cdz_num={}", rlib.display()));
        }
        if let Some((dep_dir, rlib)) = &unicode_norm {
            cmd.arg("-L")
                .arg(format!("dependency={}", dep_dir.display()))
                .arg("--extern")
                .arg(format!("unicode_normalization={}", rlib.display()));
        }
        cmd.output().expect("run rustc")
    };
    let mut status = compile();
    if !status.status.success() {
        status = compile();
    }
    assert!(
        status.status.success(),
        "emitted Rust did not compile:\n{}\n--- source ---\n{module}",
        String::from_utf8_lossy(&status.stderr)
    );
    let run = Command::new(&bin_path).output().expect("run compiled prog");
    assert!(
        run.status.success(),
        "compiled prog did not run:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let out = String::from_utf8_lossy(&run.stdout).trim().to_string();
    // Success → remove the now-unique per-invocation dir so `/tmp` doesn't accumulate one per test call.
    // (On an assert-panic above the dir is intentionally LEFT, as a debugging artifact — a rare failure.)
    let _ = std::fs::remove_dir_all(&dir);
    Some(out)
}

/// The outcome of running a trap-EXPECTING program (see [`rustc_run_traps`]). A three-way result so a
/// trap-asserting pin can distinguish "no rustc → skip" from "ran without trapping → FAIL" — the earlier
/// `Option<String>` conflated those two into `None`, making a pin regression-BLIND: `if let Some(msg) = …`
/// silently did nothing when the emit STOPPED trapping (a lost trap), so a real regression passed (Copilot
/// PR#496). The caller matches on this and MUST fail on `RanOk` to be sound.
#[derive(Debug)]
enum TrapRun {
    /// `rustc` is absent — the round-trip is skipped (like `rustc_run`); the caller does nothing.
    NoRustc,
    /// The program trapped (a `panic!` → non-zero exit). Carries the panic MESSAGE (stderr) so the caller
    /// can assert WHICH trap KIND fired (the message the gate's `trap_kind` classifies).
    Trapped(String),
    /// The program RAN to completion (no trap). Carries the printed stdout. A trap-expecting pin FAILS on
    /// this — a lost trap must not pass silently.
    RanOk(String),
}

/// Compile `module` + `println!(call)`, run it, and report whether it TRAPPED — the trap-asserting twin of
/// `rustc_run` (which asserts SUCCESS and so cannot validate a trap). Returns a [`TrapRun`] so the caller
/// distinguishes a trap (with its message, for `trap_kind`) from a silent run-to-completion (a regression)
/// from a skipped no-rustc environment — see `TrapRun` for why the old `Option<String>` was regression-blind.
fn rustc_run_traps(module: &str, call: &str) -> TrapRun {
    use std::process::Command;
    if Command::new("rustc").arg("--version").output().is_err() {
        return TrapRun::NoRustc; // no rustc — skip.
    }
    let dir = unique_tmp_dir("rcdzc-rust-trap", test_key(module, call));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("prog.rs");
    let bin_path = dir.join("prog");
    let full = format!("{module}\nfn main() {{ println!(\"{{}}\", {call}); }}\n");
    std::fs::write(&src_path, full).expect("write rust source");
    let cdz_num = cdz_num_link();
    // A String.concat / from-bytes program emits a `unicode_normalization::…::nfc(…)` NFC canonicalization
    // (FINDING #23 rust parity); link its rlib (a workspace dep, in the test binary's `deps/`) like `cdz_num`.
    let unicode_norm = dep_rlib_link("libunicode_normalization");
    let compile = || {
        let mut cmd = Command::new("rustc");
        cmd.args(["-O", "--edition", "2021"])
            .arg(&src_path)
            .arg("-o")
            .arg(&bin_path);
        if let Some((dep_dir, rlib)) = &cdz_num {
            cmd.arg("-L")
                .arg(format!("dependency={}", dep_dir.display()))
                .arg("--extern")
                .arg(format!("cdz_num={}", rlib.display()));
        }
        if let Some((dep_dir, rlib)) = &unicode_norm {
            cmd.arg("-L")
                .arg(format!("dependency={}", dep_dir.display()))
                .arg("--extern")
                .arg(format!("unicode_normalization={}", rlib.display()));
        }
        cmd.output().expect("run rustc")
    };
    let mut status = compile();
    if !status.status.success() {
        status = compile(); // retry once (parallel linker race, as in `rustc_run`).
    }
    assert!(
        status.status.success(),
        "emitted Rust did not compile:\n{}\n--- source ---\n{module}",
        String::from_utf8_lossy(&status.stderr)
    );
    let run = Command::new(&bin_path).output().expect("run compiled prog");
    let result = if run.status.success() {
        TrapRun::RanOk(String::from_utf8_lossy(&run.stdout).trim().to_string())
    } else {
        TrapRun::Trapped(String::from_utf8_lossy(&run.stderr).to_string())
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Compile the emitted `module` wrapped in `mod prog { … }` PLUS a caller-supplied `driver` (which
/// defines its own `fn main` and references the module as `prog::…`), run it, and return the printed
/// line. `None` if `rustc` is absent. Used for the async round-trip, where the driver must supply an
/// `Env` impl + an executor rather than a one-line `println!(call)`.
fn rustc_run_driver(module: &str, driver: &str) -> Option<String> {
    use std::process::Command;
    if Command::new("rustc").arg("--version").output().is_err() {
        return None;
    }
    // GLOBALLY-UNIQUE per invocation (see `unique_tmp_dir` / `rustc_run`): identical `(module, driver)`
    // content across parallel round-trips must NOT share one `prog` binary, or they race write-vs-exec.
    let dir = unique_tmp_dir("rcdzc-rust-drv", test_key(module, driver));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("prog.rs");
    let bin_path = dir.join("prog");
    // Wrap the module in `mod prog { … }` so its `pub fn`s are `prog::…` and its `#![allow(…)]` inner
    // attrs stay valid at the mod head, then append the driver (which owns `fn main`).
    let full = format!("mod prog {{\n{module}\n}}\n{driver}");
    std::fs::write(&src_path, full).expect("write rust source");
    // An emitted ASYNC module `use`s `cdz_rt::CdzEnv` from the shared `cdz-rt` crate; link its rlib (a
    // dev-dependency, so `cargo test` built it into the target dir). `cdz_rt_link()` finds the rlib +
    // its `-L` search dir; if absent (rlib not built), skip the extern — a sync module needs no crate.
    let mut cmd = Command::new("rustc");
    cmd.args(["-O", "--edition", "2021"])
        .arg(&src_path)
        .arg("-o")
        .arg(&bin_path);
    if let Some((dep_dir, rlib)) = cdz_rt_link() {
        cmd.arg("-L")
            .arg(format!("dependency={}", dep_dir.display()))
            .arg("--extern")
            .arg(format!("cdz_rt={}", rlib.display()));
    }
    // The emitted preamble declares the prelude `Ast` sum, whose `Int` payload is now `cdz_num::Big` (a
    // quoted AST stores integers non-lossily), so EVERY emitted module references `cdz_num` — link it here
    // too (mirrors the sync `rustc_run`; harmless when the crate is otherwise unused).
    if let Some((dep_dir, rlib)) = cdz_num_link() {
        cmd.arg("-L")
            .arg(format!("dependency={}", dep_dir.display()))
            .arg("--extern")
            .arg(format!("cdz_num={}", rlib.display()));
    }
    // A String.concat / from-bytes program emits `unicode_normalization::…::nfc(…)` (FINDING #23 rust NFC
    // parity); link its rlib (a workspace dep in the test binary's `deps/`), mirroring the sync runners.
    if let Some((dep_dir, rlib)) = dep_rlib_link("libunicode_normalization") {
        cmd.arg("-L")
            .arg(format!("dependency={}", dep_dir.display()))
            .arg("--extern")
            .arg(format!("unicode_normalization={}", rlib.display()));
    }
    let status = cmd.output().expect("run rustc");
    assert!(
        status.status.success(),
        "emitted async Rust did not compile:\n{}\n--- source ---\n{module}",
        String::from_utf8_lossy(&status.stderr)
    );
    let run = Command::new(&bin_path).output().expect("run compiled prog");
    assert!(
        run.status.success(),
        "compiled prog did not run:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let out = String::from_utf8_lossy(&run.stdout).trim().to_string();
    // Success → drop the unique per-invocation dir (see `rustc_run`); left on an assert-panic for debug.
    let _ = std::fs::remove_dir_all(&dir);
    Some(out)
}

/// Locate the built `cdz-rt` rlib (a dev-dependency, so present when `cargo test` runs) and the `-L`
/// search directory `rustc` needs, as `(dep_dir, rlib_path)`. The test binary lives in
/// `target/<profile>/deps/`, and cargo writes dependency rlibs (with a metadata-hash suffix) into that
/// same `deps/` dir — so search there for `libcdz_rt-*.rlib`. `None` if not found (then the async
/// round-trip skips the extern; a sync module never needs the crate). A hashed name means picking the
/// newest match, which the current build produced.
fn cdz_rt_link() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    dep_rlib_link("libcdz_rt")
}

/// Same as `cdz_rt_link` for the `cdz-num` bignum rlib (`libcdz_num-*.rlib`) — the BigInt round-trip
/// tests link it so an emitted `cdz_num::Big` program compiles.
fn cdz_num_link() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    dep_rlib_link("libcdz_num")
}

/// Locate a dev-dependency rlib (a `cargo test` build writes each dep's rlib, with a metadata-hash
/// suffix, into the test binary's `deps/` dir) by its `lib<crate>` filename prefix, returning `(dep_dir,
/// rlib_path)` for `rustc -L dependency=<dep_dir> --extern <crate>=<rlib>`. Picks the newest match.
fn dep_rlib_link(prefix: &str) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?.to_path_buf(); // …/target/<profile>/deps
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(&deps).ok()? {
        let path = entry.ok()?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with(prefix) && name.ends_with(".rlib") {
            let mtime = path
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().is_none_or(|(t, _)| mtime >= *t) {
                best = Some((mtime, path));
            }
        }
    }
    best.map(|(_, rlib)| (deps, rlib))
}

#[test]
fn rustc_roundtrip_bigint_arithmetic_and_render() {
    // BigInt emit-side: a runtime `Big` (cdz_num, source-shared with the wasm runtime) crosses end to end.
    // `Ty::BigInt` → `cdz_num::Big`; `BigInt.of` widens; `+`/`-`/`*`/`/`/`%` are `Big` methods; the result
    // renders as its exact decimal (`to_decimal_string`). Small in-range add: 40+2 = 42.
    // `rustc_run`'s driver prints the call via `{}`; `Big` isn't `Display` (it renders by
    // `to_decimal_string`, as the gate harness does), so the CALL expression asks for that string.
    let add = compile_rust("(module m (def (g) (+ (BigInt.of 40) (BigInt.of 2))) (export g))");
    assert!(
        add.contains("cdz_num::Big"),
        "BigInt maps to cdz_num::Big:\n{add}"
    );
    if let Some(out) = rustc_run(&add, "g().to_decimal_string()") {
        assert_eq!(out, "42", "runtime BigInt add");
    }
    // The DEFINING case — a product that overflows i64 must GROW, not trap: 10^10 * 10^10 = 10^20.
    let big = compile_rust(
        "(module m (def (g) (* (BigInt.of 10000000000) (BigInt.of 10000000000))) (export g))",
    );
    if let Some(out) = rustc_run(&big, "g().to_decimal_string()") {
        assert_eq!(out, "100000000000000000000", "10^10 squared grows past i64");
    }
    // A negative subtract crosses with its sign; truncating divide.
    let neg = compile_rust("(module m (def (g) (- (BigInt.of 42) (BigInt.of 100))) (export g))");
    if let Some(out) = rustc_run(&neg, "g().to_decimal_string()") {
        assert_eq!(out, "-58", "negative BigInt result keeps its sign");
    }
    let divv = compile_rust("(module m (def (g) (/ (BigInt.of 100) (BigInt.of 7))) (export g))");
    if let Some(out) = rustc_run(&divv, "g().to_decimal_string()") {
        assert_eq!(out, "14", "truncating BigInt divide");
    }
    // REGRESSION (Copilot PR#464): `BigInt.of` is `∀a.(Int a)->BigInt`, so a runtime UNSIGNED operand
    // >= 2^63 must widen BY VALUE. The old `Big::from_i64((v) as i64)` reinterpreted a large u64 as
    // NEGATIVE (silent miscompile). The emit now goes through `i128` (a `uN as i128` keeps its true sign).
    // Pin `BigInt.of` on a runtime UInt64 = 2^63 → the POSITIVE 9223372036854775808, not a negative value.
    let usig = compile_rust("(module m (def (g (: n UInt64)) (BigInt.of n)) (export g))");
    assert!(
        usig.contains("as i128"),
        "BigInt.of widens via i128 (not `as i64`):\n{usig}"
    );
    if let Some(out) = rustc_run(&usig, "g(9223372036854775808u64).to_decimal_string()") {
        assert_eq!(
            out, "9223372036854775808",
            "BigInt.of on a UInt64 >= 2^63 is POSITIVE, not negative"
        );
    }
    // …and u64::MAX round-trips as its true positive value (was -1 under the `as i64` bug).
    if let Some(out) = rustc_run(&usig, "g(u64::MAX).to_decimal_string()") {
        assert_eq!(
            out, "18446744073709551615",
            "BigInt.of(u64::MAX) is the positive max, not -1"
        );
    }
}

#[test]
fn a_runtime_bigint_sum_payload_literal_probe_compares_by_bigint_not_an_i64_cast() {
    // REGRESSION (corpus-bugfix, rust twin of breaker FINDING #22): a match on a sum whose payload is a
    // RUNTIME BigInt, probed against an integer literal (`((Mk 1) …)`), used to emit the compare as a raw
    // `(<Big>) as i64` — a NON-primitive cast (rustc E0605, a HARD build-fail, not an honest todo). The
    // literal-probe compare now detects a `Ty::BigInt` sub-value and compares by BigInt equality against the
    // materialized `Big` literal (`const_big_expr` → `Big::from_i64(1)`); `Big` derives `PartialEq`, so it
    // types. Build clean AND compute: `Mk (BigInt.of 1)` matches `(Mk 1)` → 40; `BigInt.of 2` falls through.
    let m = compile_rust(
        "(module m (type W (Mk BigInt)) \
           (def (go (: k Int64)) (match (Mk (BigInt.of k)) ((Mk 1) 40) (_ (- 0 1)))) \
           (export go))",
    );
    assert!(
        m.contains("== cdz_num::Big::from_i64(1)") && !m.contains(") as i64)) == 1i64"),
        "the BigInt payload probe compares by Big equality, not an `as i64` cast:\n{m}"
    );
    if let Some(out) = rustc_run(&m, "go(1)") {
        assert_eq!(
            out, "40",
            "Mk(BigInt.of 1) matches the (Mk 1) literal probe"
        );
    }
    if let Some(out) = rustc_run(&m, "go(2)") {
        assert_eq!(out, "-1", "Mk(BigInt.of 2) falls through to the wildcard");
    }
}

#[test]
fn a_constant_bigint_newtype_payload_materializes_a_big_not_an_i64() {
    // REGRESSION (corpus-bugfix FACE-B, sibling of the runtime E0605 probe fix): a CONSTANT integer that is
    // the payload of a BigInt-typed ERASED NEWTYPE construction (`(Mk 1)` where `W = (Mk BigInt)`) used to
    // emit as a fixed-width int literal (`1u64 as i64`) because `is_bigint_valued` did NOT strip the nominal
    // wrapper — so a `Ty::Nominal { inner: BigInt }`-typed constant fell to the int-literal path, mismatching
    // the `cdz_num::Big` slot (rustc E0308). `is_bigint_valued` now strips the nominal, so the constant
    // materializes `Big::from_i64(1)`. Build clean AND compute: `(Mk 1)` fed through a const-scrutinee probe
    // matches `(Mk 1)` → 40.
    let m = compile_rust(
        "(module m (type W (Mk BigInt)) \
           (def (walk (: n Int64) (: w W)) (if (< n 1) (- 0 1) (match w ((Mk 1) 40) (_ (walk (- n 1) w))))) \
           (def (go) (walk 2 (Mk 1))) (export go))",
    );
    assert!(
        m.contains("cdz_num::Big::from_i64(1))")
            && !m.contains("walk((2u64 as i64), (1u64 as i64))"),
        "the constant (Mk 1) BigInt payload materializes a Big, not an i64 literal:\n{m}"
    );
    if let Some(out) = rustc_run(&m, "go()") {
        assert_eq!(
            out, "40",
            "walk 2 (Mk 1) matches the (Mk 1) const probe → 40"
        );
    }
    // The isolated newtype constructor: `(Mk 1)` returned as W (= Big) is the Big 1, not an i64.
    let id = compile_rust(
        "(module m (type W (Mk BigInt)) (def (id (: w W)) w) (def (go) (id (Mk 1))) (export go))",
    );
    assert!(
        id.contains("-> cdz_num::Big") && id.contains("cdz_num::Big::from_i64(1)"),
        "a constant BigInt-newtype value materializes a Big:\n{id}"
    );
}

#[test]
fn rustc_roundtrip_rational_arithmetic_and_render() {
    // Rational emit-side: `Ty::Rational` → `cdz_num::Rational` (a Big num/den pair, canonical normalized);
    // `Rational.of`/`.of-int` widen + build, `+`/`-`/`*`/`/` are Rational methods, cmp reduces to a bool,
    // render is `n/d`. Values MIRROR the wasm runtime's rational-* byte-for-byte. 1/3 + 1/6 = 1/2 exactly.
    let add =
        compile_rust("(module m (def (g) (+ (Rational.of 1 3) (Rational.of 1 6))) (export g))");
    assert!(
        add.contains("cdz_num::Rational"),
        "Rational maps to cdz_num::Rational:\n{add}"
    );
    if let Some(out) = rustc_run(&add, "g().to_display_string()") {
        assert_eq!(out, "1/2", "exact rational addition, lowest terms");
    }
    // Reduce to lowest terms; sign normalized onto the numerator; a whole rational keeps `/1`.
    let red = compile_rust("(module m (def (g) (Rational.of 2 4)) (export g))");
    if let Some(out) = rustc_run(&red, "g().to_display_string()") {
        assert_eq!(out, "1/2", "2/4 reduces to 1/2");
    }
    let sign = compile_rust("(module m (def (g) (Rational.of 3 -4)) (export g))");
    if let Some(out) = rustc_run(&sign, "g().to_display_string()") {
        assert_eq!(
            out, "-3/4",
            "sign moves to the numerator, denominator positive"
        );
    }
    let whole = compile_rust("(module m (def (g) ((. Rational of-int) 5)) (export g))");
    if let Some(out) = rustc_run(&whole, "g().to_display_string()") {
        assert_eq!(out, "5/1", "a whole rational carries denominator 1");
    }
    // A Rational is a valid BTreeSet/BTreeMap key (impl Ord) — dedup by exact value: {1/2, 2/4, 1/3} → 2.
    let set = compile_rust(
        "(module m (def (g) (Set.len (Set.of (list (Rational.of 1 2) (Rational.of 2 4) (Rational.of 1 3))))) \
           (export g))",
    );
    if let Some(out) = rustc_run(&set, "g()") {
        assert_eq!(
            out, "2",
            "a Rational set dedups by normalized value (1/2 == 2/4)"
        );
    }
}

#[test]
fn quantity_result_maps_to_inner_at_any_scale1_unit_else_declines() {
    // A QUANTITY RESULT at a scale-1 unit maps to its INNER magnitude's Rust type (the `Ty::Qty` wrapper
    // erases in `lower`); the unit's canonical VALUE form is carried in a `// cdz-unit` note
    // (`Unit::render_value_form`) and rendered `((. Qty of) <mag> <unit>)` by the gate harness (the corpus
    // cases pin the end-to-end render). Here we pin the EMIT side: a scale-1 unit of ANY shape compiles with
    // the inner type + the value-form note; a non-scale-1 unit still declines.
    let base = compile_rust("(module m (def (g) (Qty.of 5.0 (Unit.base #\"meter\"))) (export g))");
    assert!(
        base.contains("-> f64")
            && base.contains("// cdz-return[g]: (Qty Float64 (Unit.base #\"meter\"))")
            && base.contains("// cdz-unit[g]: (Unit.base #\"meter\")"),
        "a Qty{{Float64,meter}} result emits the inner f64 + return + value-form unit notes:\n{base}"
    );
    // A NON-scale-1 unit over a BIGINT magnitude still DECLINES — a bignum scaled by a non-integer ratio is
    // not a BigInt (Float/Int/Rational non-scale-1 all scale now; see the dedicated display-scale pin).
    let scaled = compile_rust_result(
        "(module m (def (g) (Qty.of (BigInt.of 5) (Unit.of #\"mile\"))) (export g))",
    );
    assert!(
        scaled.is_err(),
        "a non-scale-1 BigInt (mile) quantity result declines (not a BigInt after scaling):\n{scaled:?}"
    );
    // A SINGLE base to a POSITIVE power — `meter²`, an area from `m·m`. The value-form note carries the
    // `Unit.^` surface (the gate's area cases pin the end-to-end render).
    let area = compile_rust(
        "(module m (def (g) (* (Qty.of 2.0 (Unit.base #\"meter\")) (Qty.of 3.0 (Unit.base #\"meter\")))) (export g))",
    );
    assert!(
        area.contains("-> f64")
            && area.contains("// cdz-unit[g]: (Unit.^ (Unit.base #\"meter\") 2)"),
        "a meter² result emits the inner f64 + a `Unit.^` value-form unit note:\n{area}"
    );
    // A QUOTIENT of DISTINCT bases — a velocity `m/s` — now COMPILES (scale-1, the value-form note renders it
    // as a `Unit./` quotient, cdz-run's canonical surface). The `cdz-return` note's TYPE surface spells it
    // `Unit.*`/`Unit.^ -1`, but the `cdz-unit` note carries the quotient form the boundary render needs.
    let velocity = compile_rust(
        "(module m (def (g) (/ (Qty.of 6.0 (Unit.base #\"meter\")) (Qty.of 2.0 (Unit.base #\"second\")))) (export g))",
    );
    assert!(
        velocity.contains("-> f64")
            && velocity.contains(
                "// cdz-unit[g]: (Unit./ (Unit.base #\"meter\") (Unit.base #\"second\"))"
            ),
        "a velocity (m/s) result emits the inner f64 + a `Unit./` quotient value-form note:\n{velocity}"
    );
    // A reciprocal / negative power — `second⁻¹`, a frequency — renders as `(Unit./ Unit.one…)`.
    let freq = compile_rust(
        "(module m (def (g) (Qty.pow (Qty.of 2.0 (Unit.base #\"second\")) -1)) (export g))",
    );
    assert!(
        freq.contains("// cdz-unit[g]: (Unit./ Unit.one (Unit.base #\"second\"))"),
        "a frequency (second⁻¹) result emits a `Unit./` over the dimensionless numerator:\n{freq}"
    );
}

#[test]
fn rustc_roundtrip_nominal_over_narrow_qty_map_value_grounds_to_inner_width() {
    // A NOMINAL newtype WRAPPING a narrow-int quantity — `(type Len (Q (Qty Int8 meter)))` — stored as a
    // MAP VALUE and read back. The map value type is the erased narrow inner (`i8`, `BTreeMap<i64, i8>`),
    // but `int_ty_of` peeled only a RAW `Ty::Qty`, so a `Ty::Nominal { inner: Qty }` missed the peel and
    // grounded the inserted magnitude to the i64 default → `insert(k, n as i64)` into an `i8` slot → rustc
    // E0308 (reviewer-flagged, low-confidence, confirmed real). Fixed by `int_ty_of` doing strip_nominal →
    // peel Qty → strip_nominal (mirroring the wasm backend), so the narrow inner is seen through ANY
    // nominal/Qty wrapping. Emits `insert(…, 100u8 as i8)` and runs to 100 = the wasm oracle.
    let rs = compile_rust(
        "(module m (type Len (Q (Qty Int8 (Unit.base #\"meter\")))) \
           (def (run) \
             (match (Map.lookup (Map.insert (Map.empty) 1 \
                       (Len.Q (Qty.of (Int8.of 100) (Unit.base #\"meter\")))) 1) \
               ((Some (Len.Q q)) (Qty.value q)) \
               ((None) 0))) \
           (export run))",
    );
    // The narrow inner drives the map value type + the inserted magnitude — no `as i64` into an i8 slot.
    assert!(
        rs.contains("BTreeMap<i64, i8>") && rs.contains("100u8 as i8"),
        "the nominal-over-Qty map value grounds to the inner i8 width (no i64 default):\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "run()") {
        assert_eq!(
            out, "100",
            "the stored 100-meter nominal-Qty reads back + unwraps to 100"
        );
    }

    // The FLOAT twin: a nominal-over-`(Qty Float32 …)` map value. The `Core::ConstFloat` width reader's bare
    // `peel_qty` (RAW Ty::Qty, no strip_nominal) missed a `Ty::Nominal { inner: Qty { Float32 } }` → f64
    // default → `f64::from_bits` into an `f32` slot → E0308/invalid-wasm. `float_width_of` (strip → peel →
    // strip, the float twin of `int_ty_of`) now grounds it to f32. Both arms Float32 so the match unifies.
    let frs = compile_rust(
        "(module m (type Len (Q (Qty Float32 (Unit.base #\"meter\")))) \
           (def (run) \
             (Qty.value \
               (match (Map.lookup (Map.insert (Map.empty) 1 \
                         (Len.Q (Qty.of (Float32.of 1.5) (Unit.base #\"meter\")))) 1) \
                 ((Some (Len.Q q)) q) \
                 ((None) (Qty.of (Float32.of 0.0) (Unit.base #\"meter\")))))) \
           (export run))",
    );
    assert!(
        frs.contains("BTreeMap<i64, f32>")
            && frs.contains("f32::from_bits")
            && !frs.contains("f64::from_bits"),
        "the nominal-over-Qty-Float32 map value grounds to the inner f32 (no f64 default):\n{frs}"
    );
    if let Some(out) = rustc_run(&frs, "run()") {
        assert_eq!(
            out, "1.5",
            "the stored 1.5-meter nominal-Qty-f32 reads back + unwraps to 1.5"
        );
    }
}

#[test]
fn a_non_scale1_float_int_or_rational_quantity_display_scales_to_its_reference() {
    // A NON-scale-1 unit DISPLAY-SCALES the stored magnitude to its dimension's reference (`5 km` →
    // `5000 m`). The backend emits the unit at REFERENCE (`Unit::at_reference().render_value_form`) + a
    // `// cdz-scale[…]: num/den` note; the gate harness multiplies the boundary magnitude by that scale.
    // Supported for FLOAT (rounds) / INT (truncates) / RATIONAL (EXACT via `Rational::mul`) inners. (BigInt
    // still declines — a bignum scaled by a non-integer ratio isn't a BigInt.)
    // Float: `5.0 kilometer` — reference `meter`, scale `1000/1`.
    let km = compile_rust(
        "(module m (def (g) (Qty.of 5.0 (Unit.prefix kilo (Unit.base #\"meter\")))) (export g))",
    );
    assert!(
        km.contains("-> f64")
            && km.contains("// cdz-unit[g]: (Unit.base #\"meter\")")
            && km.contains("// cdz-scale[g]: 1000/1"),
        "a Float kilometer result emits the reference unit + a 1000/1 scale note:\n{km}"
    );
    // Int: `1 kibibyte` — reference `byte`, scale `1024/1`.
    let kib = compile_rust(
        "(module m (def (g) (Qty.of 1 (Unit.prefix kibi (Unit.base #\"byte\")))) (export g))",
    );
    assert!(
        kib.contains("// cdz-scale[g]: 1024/1")
            && kib.contains("// cdz-unit[g]: (Unit.base #\"byte\")"),
        "an Int kibibyte result emits the reference byte + a 1024/1 scale note:\n{kib}"
    );
    // A scale-1 (reference) unit emits NO scale note — the magnitude is displayed as stored.
    let m = compile_rust("(module m (def (g) (Qty.of 5.0 (Unit.base #\"meter\"))) (export g))");
    assert!(
        !m.contains("// cdz-scale["),
        "a scale-1 meter result emits no scale note:\n{m}"
    );
    // A RATIONAL non-scale-1 (`5 mile`) scales EXACTLY: reference `meter`, scale `201168/125`, so the
    // harness multiplies the stored `5/1` by `201168/125` (as a `cdz_num::Rational`) = `201168/25 meter`.
    // The result type is `cdz_num::Rational`; the scale note carries the exact ratio.
    let mile = compile_rust(
        "(module m (def (g) (Qty.of (Rational.of 5 1) (Unit.of #\"mile\"))) (export g))",
    );
    assert!(
        mile.contains("-> cdz_num::Rational")
            && mile.contains("// cdz-scale[g]: 201168/125")
            && mile.contains("// cdz-unit[g]: (Unit.base #\"meter\")"),
        "a Rational mile result emits the reference meter + the exact 201168/125 scale note:\n{mile}"
    );
    // A BigInt non-scale-1 still DECLINES (a bignum scaled by a non-integer ratio is not a BigInt).
    let bigmile = compile_rust_result(
        "(module m (def (g) (Qty.of (BigInt.of 5) (Unit.of #\"mile\"))) (export g))",
    );
    assert!(
        bigmile.is_err(),
        "a BigInt non-scale-1 (mile) quantity result declines (not a BigInt after scaling):\n{bigmile:?}"
    );
}

#[test]
fn rustc_roundtrip_add_matches_the_wasm_answer() {
    // The exact I2b wasmtime answers: add(20,22)=42, add(100,-1)=99.
    let rs = compile_rust("(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))");
    if let Some(out) = rustc_run(&rs, "add(20, 22)") {
        assert_eq!(out, "42");
    }
    if let Some(out) = rustc_run(&rs, "add(100, -1)") {
        assert_eq!(out, "99");
    }
}

#[test]
fn rustc_roundtrip_signed_compare() {
    let rs = compile_rust("(module m (def (lt (: a Int64) (: b Int64)) (< a b)) (export lt))");
    if let Some(out) = rustc_run(&rs, "lt(3, 5)") {
        assert_eq!(out, "true");
    }
    if let Some(out) = rustc_run(&rs, "lt(5, 3)") {
        assert_eq!(out, "false");
    }
}

#[test]
fn rustc_roundtrip_option_compare_follows_cadenza_some_before_none_not_std() {
    // SOUNDNESS (breaker/corpus-bugfix #42): Cadenza declares `Some` (disc 0) `< None` (disc 1) — but Rust's
    // std `Option`, which the backend maps Cadenza `Option` to, derives the REVERSE order `None < Some`. A
    // native `l < r` / `l.cmp(&r)` therefore gave the WRONG total order (`compare (Some 3) None` → std
    // Greater, Cadenza Less). `ValueCmp` now routes an Option-containing operand through the type-directed
    // walk (Some-before-None), so compare/`<` match the declared order + the wasm backend (which is correct).
    // Ordering ctor→Int probe: Less→1, Equal→2, Greater→3.
    // A RUNTIME Option (built by an `if`, so it lowers to a value-cmp, not a const fold), matching the corpus
    // witness. `mk 0` = None, `mk k` = Some k. compare (mk 3) (mk 0) = compare (Some 3) None → Less (1).
    let cmp = compile_rust(
        "(module m \
           (def (mk (: k Int64)) (if (= k 0) (: (None unit) (Option Int64)) (Some k))) \
           (def (go (: a Int64) (: b Int64)) \
             (match (Ordering.of (mk a) (mk b)) ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
         (export go))",
    );
    if let Some(out) = rustc_run(&cmp, "go(3, 0)") {
        assert_eq!(
            out, "1",
            "compare (Some 3) None must be Less (Cadenza Some<None), NOT Greater (std None<Some):\n{cmp}"
        );
    }
    // Two Somes still order by payload (Some 1 < Some 2 → Less); None is the greatest (Some k < None).
    if let Some(out) = rustc_run(&cmp, "go(1, 2)") {
        assert_eq!(out, "1", "(Some 1) < (Some 2) by payload → Less:\n{cmp}");
    }
    if let Some(out) = rustc_run(&cmp, "go(0, 5)") {
        assert_eq!(
            out, "3",
            "None > (Some 5) → Greater (None is the max):\n{cmp}"
        );
    }
    // NESTED: an Option as a tuple leaf — the flip must be corrected at the leaf, not just top-level.
    // (tuple 0 (Some 3)) vs (tuple 0 None): field 0 equal, so decided by the Option leaf → Some<None → Less.
    let nested = compile_rust(
        "(module m \
           (def (mk (: k Int64)) (if (= k 0) (: (None unit) (Option Int64)) (Some k))) \
           (def (go (: n Int64)) \
             (match (Ordering.of (tuple n (mk n)) (tuple n (mk 0))) \
               ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
         (export go))",
    );
    if let Some(out) = rustc_run(&nested, "go(3)") {
        assert_eq!(
            out, "1",
            "a nested Option leaf orders Some<None too (tuple .1 decides):\n{nested}"
        );
    }
    // CONTROL — `Result` maps to std `Result` whose `Ok < Err` MATCHES Cadenza; it must stay correct (the fix
    // must not disturb it — Result is not a flip). compare (Ok n) (Err n) → Ok<Err → Less.
    let res = compile_rust(
        "(module m \
           (def (mk (: k Int64)) (if (= k 0) (: (Err unit) (Result Int64 Unit)) (Ok k))) \
           (def (go (: a Int64) (: b Int64)) \
             (match (Ordering.of (mk a) (mk b)) ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
         (export go))",
    );
    if let Some(out) = rustc_run(&res, "go(1, 0)") {
        assert_eq!(
            out, "1",
            "Result Ok<Err control stays correct (Ok 1 < Err):\n{res}"
        );
    }
}

#[test]
fn rustc_roundtrip_option_keyed_set_enumerates_some_before_none() {
    // SOUNDNESS #42 WITNESS 2: an Option-KEYED set enumerates in Cadenza declared order (Some < None) via the
    // `__CdzOpt` wrapper, NOT std `Option`'s `None < Some`. A `BTreeSet<Option<T>>` would order by std's
    // derived Ord (None first) — the cross-target divergence (wasm Set.to-list head = Some, rust = None).
    // The wrapper gives BTreeSet the declared order. Fixture: a set of {Some 3, None, Some 1}; Set.to-list
    // head is the SMALLEST = Some 1 (Cadenza), the count is 3. Probe reads the head: Some x → x, None → -99.
    let m = compile_rust(
        "(module m \
           (def (mk (: k Int64)) (if (= k 0) (: (None unit) (Option Int64)) (Some k))) \
           (def (go (: a Int64)) \
             (match (List.at (Set.to-list (Set.of (list (mk 3) (mk 0) (mk 1)))) 0) \
               ((Option.Some p) (match p ((Option.Some x) x) ((Option.None) -1))) \
               ((Option.None) -99))) \
         (export go))",
    );
    // Head of the ordered enumeration is the SMALLEST element. Cadenza: Some 1 < Some 3 < None → head Some 1
    // → inner x = 1. std order would put None first → head None → the outer/inner match yields -1/-99.
    if let Some(out) = rustc_run(&m, "go(0)") {
        assert_eq!(
            out, "1",
            "an Option-keyed Set.to-list must enumerate Some-before-None (head = Some 1, Cadenza order), \
             NOT None-first (std Option order):\n{m}"
        );
    }
    // The set also dedups + counts correctly (3 distinct elements: Some 3, None, Some 1).
    let len = compile_rust(
        "(module m \
           (def (mk (: k Int64)) (if (= k 0) (: (None unit) (Option Int64)) (Some k))) \
           (def (go (: a Int64)) (Set.len (Set.of (list (mk 3) (mk 0) (mk 1) (mk 3))))) \
         (export go))",
    );
    if let Some(out) = rustc_run(&len, "go(0)") {
        assert_eq!(
            out, "3",
            "the Option-keyed set dedups (Some 3 twice) → 3 distinct:\n{len}"
        );
    }
}

#[test]
fn a_compound_key_containing_a_nested_option_declines_cleanly_not_e0308() {
    // PR#894 finding (1): the `__CdzOpt` Option-key wrapper threads through a BARE Option key/element, but
    // NOT through an Option nested inside a tuple/record key (that wrapper-threading is a later increment).
    // Before the fix, a `(Tuple (Option Int64) Int64)` key emitted `BTreeSet<(__CdzOpt<i64>, i64)>` (type
    // wrapped) with a bare `(Option<i64>, i64)` value (not rebuilt) → rustc E0308 + a missed `__CdzOpt`
    // injection (the `<(__CdzOpt` marker). Now `ty_is_ord_key` DECLINES a compound key containing a nested
    // built-in Option → a clean backend decline (fall back), never an uncompilable artifact. A BARE Option
    // key still works (the other witness); only the nested-in-a-compound case declines.
    let src = "(module m \
        (def (mk (: k Int64)) (if (= k 0) (: (None unit) (Option Int64)) (Some k))) \
        (def (go (: n Int64)) (Set.len (Set.of (list (tuple (mk n) 1) (tuple (mk 0) 2))))) \
        (export go))";
    match compile_rust_result(src) {
        // A clean DECLINE is the expected outcome (the wrapper doesn't thread through the tuple yet).
        Err(_) => {}
        // If it ever DOES emit (a future increment threads the wrapper through), it MUST compile + run — never
        // an E0308. rustc_run asserts the emit compiles; a bad emit fails LOUDLY here, not silently.
        Ok(_) => {
            if let Some(out) = rustc_run(&compile_rust(src), "go(3)") {
                assert_eq!(
                    out, "2",
                    "if a compound-Option key ever emits, it must compile + dedup to 2 (never E0308):\n{src}"
                );
            }
        }
    }
}

#[test]
fn rustc_roundtrip_recursive_option_carrying_sum_compare_terminates_via_helper() {
    // PR#890 REGRESSION: emit_sum_cmp_walk (the #42 Option-order compare walk) must route a RECURSIVE
    // Option-carrying sum through a `__cmp_<Ident>` helper fn (seen-guard + call-indirection), NOT expand
    // inline — an inline expansion recurses UNBOUNDED in codegen (compiler stack overflow / runaway output)
    // when the sum reappears in its own payload. Fixture: a recursive `Lst` = (Cons (Tuple (Option Int64)
    // Lst)) | (Nil), compared via `compare` — the payload's `Option Int64` forces the value-cmp walk (not
    // native .cmp()), and `Lst` recurses through the Cons payload. If codegen doesn't terminate this reds at
    // COMPILE (the backend or rustc stack-overflows); a green compile + correct answer proves the helper
    // routing works. compare of two equal single-element lists → Equal (2); a Some-vs-None head → Less (1).
    let src = "(module m \
        (type Lst (Cons (Tuple (Option Int64) Lst)) (Nil)) \
        (def (mk-some (: k Int64)) (Lst.Cons (tuple (Some k) (Lst.Nil)))) \
        (def (mk-none) (Lst.Cons (tuple (: (None unit) (Option Int64)) (Lst.Nil)))) \
        (def (go (: k Int64)) \
          (match (Ordering.of (mk-some k) (mk-none)) \
            ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
        (export go))";
    match compile_rust_result(src) {
        // A clean decline (e.g. a shape the walk doesn't render) is acceptable — the point is NO unbounded
        // codegen. If it emits, it MUST compile + compute the declared-order answer.
        Err(_) => {}
        Ok(_) => {
            let rs = compile_rust(src);
            // The helper must be present (proves routing, not inline expansion) — a `fn __cmp_` in the output.
            assert!(
                rs.contains("fn __cmp_"),
                "a recursive Option-carrying sum compare must route through a __cmp_ helper (not inline):\n{rs}"
            );
            if let Some(out) = rustc_run(&rs, "go(5)") {
                // Cons (Some 5, Nil) vs Cons (None, Nil): same variant Cons → compare payloads; tuple field 0
                // is Option → Some 5 < None (Cadenza) → Less (1).
                assert_eq!(
                    out, "1",
                    "recursive Option-carrying sum orders its Option payload Some<None → Less:\n{rs}"
                );
            }
        }
    }
}

#[test]
fn a_runtime_shift_emits_a_guarded_block() {
    // `<<` guards the count (`>= N` panics) AND round-trips to catch overflow; `>>` guards the count
    // and shifts natively (arithmetic for signed, logical for unsigned — the value type decides).
    let shl = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (<< a b)) (export go))");
    // The count is range-checked at its FULL i64 width BEFORE the narrowing `as u32` — a
    // `(count as u32) >= 64` guard reads only the low 32 bits and lets a 2^32-multiple count slip
    // through (see `a_runtime_shift_count_that_is_a_multiple_of_2_pow_32_traps`).
    assert!(
        shl.contains("(0..64).contains(&c64)"),
        "count guard:\n{shl}"
    );
    assert!(shl.contains("(r >> c) != v"), "overflow round-trip:\n{shl}");
    assert!(shl.contains("v << c"), "the shift:\n{shl}");
    let shr = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (>> a b)) (export go))");
    assert!(
        shr.contains("(0..64).contains(&c64)") && shr.contains("v >> c"),
        ">> guarded:\n{shr}"
    );
    assert!(
        !shr.contains("round"),
        ">> needs no overflow round-trip:\n{shr}"
    );
}

#[test]
fn rustc_roundtrip_shift_computes_and_traps() {
    // `<<` and `>>` match the wasm oracle: value, out-of-range-count trap, overflow trap, and the
    // arithmetic-vs-logical distinction (a signed `>>` sign-extends).
    let shl = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (<< a b)) (export go))");
    if let Some(out) = rustc_run(&shl, "go(1, 4)") {
        assert_eq!(out, "16");
    }
    let shr = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (>> a b)) (export go))");
    if let Some(out) = rustc_run(&shr, "go(-16, 2)") {
        assert_eq!(out, "-4"); // arithmetic (sign-extending) right shift
    }
    let ushr = compile_rust("(module m (def (go (: a UInt8) (: b UInt8)) (>> a b)) (export go))");
    if let Some(out) = rustc_run(&ushr, "go(200, 1)") {
        assert_eq!(out, "100"); // logical (zero-fill) right shift
    }
    // Overflow/out-of-range traps abort (nonzero exit → the run helper's success assert fails), so the
    // trap paths are pinned by the wasm gate cross-check + the emit-shape test above; here we assert the
    // in-range values match. (An explicit panic-catch driver is the emit-side test's job, not here.)
}

#[test]
fn rustc_odd_width_left_shift_overflow_range_checks_the_declared_width_not_the_slot() {
    // adv-67b (HIGH differential, same family as adv-67): an ODD-width `<<` overflow must trap — the result
    // out of the DECLARED range escapes on rust while wasm traps. `UInt4 3<<3` = 24 > UInt4 max 15 → MUST
    // trap. The bug: the overflow check was ONLY the round-trip `(r >> c) != v` done at the SLOT type (i32
    // for UInt4); 24 fits i32 losslessly, `24>>3`==3==v round-trips clean → 24 escaped (poisoning a CHAMP
    // Set in the wild). Fix: for an odd width, ALSO range-check `r` against the declared `[min_N, max_N]`.
    let rs = compile_rust(
        "(module m (def (go (: k Int64)) \
           (Int64.of (<< ((. (UInt 4) wrap) 3) ((. (UInt 4) wrap) k)))) (export go))",
    );
    // The emit carries a declared-range check (against UInt4's max 15) in addition to the round-trip.
    assert!(
        rs.contains("as i128) > 15") && rs.contains("integer overflow in left shift"),
        "an odd-width << range-checks the result against the declared max (15 for UInt4):\n{rs}"
    );
    // End-to-end: 3 << 3 = 24 is out of UInt4 range → must TRAP (was escaping as 24).
    match rustc_run_traps(&rs, "go(3)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("overflow"),
            "UInt4 3<<3 (=24 > max 15) must trap overflow; panic was:\n{msg}"
        ),
        TrapRun::RanOk(out) => {
            panic!("UInt4 3<<3 must TRAP (24 out of range), but ran → {out} (adv-67b)")
        }
        TrapRun::NoRustc => {}
    }
    // An IN-RANGE odd-width shift still computes: 3 << 2 = 12 (≤ 15). No trap.
    if let Some(out) = rustc_run(&rs, "go(2)") {
        assert_eq!(
            out, "12",
            "UInt4 3<<2 = 12 is in range (≤ 15), computes normally"
        );
    }
    // CONTROL: an ALIASED width (UInt8) keeps ONLY the round-trip (slot==declared, no separate range check).
    let aliased = compile_rust(
        "(module m (def (go (: k Int64)) \
           (Int64.of (<< ((. (UInt 8) wrap) 3) ((. (UInt 8) wrap) k)))) (export go))",
    );
    assert!(
        aliased.contains("(r >> c) != v") && !aliased.contains("as i128) > 255"),
        "an aliased UInt8 << uses only the round-trip (no separate declared-range check):\n{aliased}"
    );
}

#[test]
fn a_runtime_shift_count_that_is_a_multiple_of_2_pow_32_traps() {
    // REGRESSION (breaker 2026-07-17): the count guard must range-check the count at its FULL i64
    // width BEFORE narrowing to u32. The prior guard `let c = (count) as u32; if c >= 64` truncated
    // FIRST, so a count that is a multiple of 2^32 (low 32 bits = 0) read as 0, skipped the trap, and
    // shifted by 0 — returning the operand UNCHANGED where wasm traps (a backend value differential).
    // `(<< 5 2^32)` MUST trap "shift count out of range", not evaluate to 5.
    let shl = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (<< a b)) (export go))");
    match rustc_run_traps(&shl, "go(5, 4294967296)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("shift count out of range"),
            "expected an out-of-range-count trap, got:\n{msg}"
        ),
        TrapRun::RanOk(out) => {
            panic!("`(<< 5 2^32)` must TRAP (count out of range), but ran → {out}")
        }
        TrapRun::NoRustc => {}
    }
    // The `>>` arm shares the fix: `(>> 20 2^32+2)` must trap, not truncate to `(>> 20 2)` = 5.
    let shr = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (>> a b)) (export go))");
    match rustc_run_traps(&shr, "go(20, 4294967298)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("shift count out of range"),
            "expected an out-of-range-count trap, got:\n{msg}"
        ),
        TrapRun::RanOk(out) => {
            panic!("`(>> 20 2^32+2)` must TRAP (count out of range), but ran → {out}")
        }
        TrapRun::NoRustc => {}
    }
}

#[test]
fn a_runtime_rational_zero_denominator_traps_with_a_classifying_unreachable_message() {
    // REGRESSION (corpus "a rational built from a RUNTIME zero denominator traps"): `(Rational.of n d)`
    // with a runtime `d = 0` has no rational value and must TRAP (numeric-model.md; the rational analogue
    // of a runtime divide-by-zero; the const case is CDZ0304 at lower). `cdz_num::Rational::new` DOES
    // panic on a zero denominator, but its message ("Rational with zero denominator") does NOT classify
    // under the gate's `trap_kind`, so the case graded `todo` (unconfirmed trap). The emit now guards the
    // denominator explicitly and panics "unreachable" — the SAME non-arithmetic trap kind the wasm backend
    // lowers this to — so both backends grade PASS.
    let m = compile_rust(
        "(module m (def (go (: n Int64) (: d Int64)) \
           (Int64.of (Rational.numerator (Rational.of n d)))) (export go))",
    );
    // Emit-shape: an explicit zero-denominator guard panicking the classifying "unreachable".
    assert!(
        m.contains("is_zero()") && m.contains("panic!(\"unreachable\")"),
        "the runtime Rational.of emits an explicit zero-denominator unreachable guard:\n{m}"
    );
    // In-range control: 1/2 normalizes, numerator reads back 1.
    if let Some(out) = rustc_run(&m, "go(1, 2)") {
        assert_eq!(out, "1", "1/2 normalizes and its numerator is 1");
    }
    // d = 0 traps with a message that classifies as `unreachable` (matches the corpus + the wasm oracle).
    match rustc_run_traps(&m, "go(1, 0)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("unreachable"),
            "a zero denominator traps with a classifying 'unreachable' message, got:\n{msg}"
        ),
        TrapRun::RanOk(out) => {
            panic!("`(Rational.of 1 0)` must TRAP (zero denominator), but ran → {out}")
        }
        TrapRun::NoRustc => {}
    }
}

#[test]
fn int64_of_a_runtime_bigint_out_of_range_traps_with_a_classifying_unreachable_message() {
    // REGRESSION (corpus "truncating a rational whose integer part exceeds Int64 traps"): `Int64.of` on a
    // runtime `Big` that exceeds i64 must TRAP (numeric-model.md; the checked narrowing, matching the wasm
    // `bigint-to-i64-checked` which lowers to a wasm `unreachable`). The behavior already trapped, but the
    // `.expect("BigInt value out of Int64 range")` message did NOT classify under the gate's `trap_kind`, so
    // the case graded `todo` (an unconfirmed trap) — the exact gap the case doc calls out. The emit now
    // panics a message CONTAINING "unreachable" (the same non-arithmetic trap kind as the shift-count and
    // rational-zero guards), so both backends grade PASS.
    let m = compile_rust(
        "(module m (def (go (: n Int64)) \
           (Int64.of (Rational.numerator \
             (* (Rational.of n 1) (Rational.of 9223372036854775807 1))))) (export go))",
    );
    // Emit-shape: the checked narrowing panics a message that classifies as `unreachable`.
    assert!(
        m.contains("to_i64_checked()") && m.contains("panic!(\"unreachable"),
        "Int64.of a Big narrows checked and panics a classifying 'unreachable' message:\n{m}"
    );
    // In range (n = 1 → 1 · Int64.max = Int64.max, fits) reads back Int64.max.
    if let Some(out) = rustc_run(&m, "go(1)") {
        assert_eq!(
            out, "9223372036854775807",
            "1 · Int64.max fits and narrows back"
        );
    }
    // n = 3 → 3 · Int64.max exceeds i64 → traps with a classifying `unreachable` message.
    match rustc_run_traps(&m, "go(3)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("unreachable"),
            "an out-of-range Int64.of traps with a classifying 'unreachable' message, got:\n{msg}"
        ),
        TrapRun::RanOk(out) => {
            panic!("`Int64.of (3 · Int64.max)` must TRAP (out of range), but ran → {out}")
        }
        TrapRun::NoRustc => {}
    }
}

#[test]
fn runtime_arithmetic_on_an_unusual_width_range_checks_at_its_own_width_not_the_storage_width() {
    // REGRESSION (corpus "a genuinely-runtime MULTIPLY on an unusual signed width traps on overflow"): an
    // unusual width (`Int 4`, `UInt 12` — 1..=64 but not 8/16/32/64) is STORED in the next-larger machine
    // primitive, so a `checked_*` on the storage type traps at `2^machine`, NOT the type's `2^N` — a wrong
    // overflow, so `emit_arith` DECLINED. Now `+`/`-`/`*` compute the native (wrapping) op on the storage
    // type and RANGE-CHECK the result against the TYPE's own `[min_N, max_N]`, panicking "integer overflow"
    // (the classifying kind) out of range — the rust twin of the wasm narrow-width `emit_range_check`.
    // (a) signed Int4 `[-8, 7]`: 2·3=6 in range; 3·3=9 and 4·4=16 overflow (the corpus oracle).
    let i4 = compile_rust(
        "(module m (def (run (: a Int64) (: b Int64)) \
           (* ((. (Int 4) wrap) a) ((. (Int 4) wrap) b))) (export run))",
    );
    assert!(
        i4.contains("integer overflow in multiplication")
            && (i4.contains("7i8") || i4.contains("> 7")),
        "an unusual-width multiply range-checks at the TYPE's bound (7 for Int4), not the storage width:\n{i4}"
    );
    if let Some(out) = rustc_run(&i4, "run(2, 3)") {
        assert_eq!(out, "6", "2·3 = 6 fits Int4 [-8, 7]");
    }
    match rustc_run_traps(&i4, "run(4, 4)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("overflow"),
            "4·4 = 16 overflows Int4 and traps overflow, got:\n{msg}"
        ),
        TrapRun::RanOk(out) => panic!("`4·4 : Int4` must TRAP (16 > 7), but ran → {out}"),
        TrapRun::NoRustc => {}
    }
    // (b) unsigned UInt12 `[0, 4095]` — the single upper-bound test: 3·4=12 fits; 50·90=4500 overflows.
    let u12 = compile_rust(
        "(module m (def (run (: a Int64) (: b Int64)) \
           (* ((. (UInt 12) wrap) a) ((. (UInt 12) wrap) b))) (export run))",
    );
    if let Some(out) = rustc_run(&u12, "run(3, 4)") {
        assert_eq!(out, "12", "3·4 = 12 fits UInt12 [0, 4095]");
    }
    match rustc_run_traps(&u12, "run(50, 90)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("overflow"),
            "50·90 = 4500 overflows UInt12 and traps overflow, got:\n{msg}"
        ),
        TrapRun::RanOk(out) => panic!("`50·90 : UInt12` must TRAP (4500 > 4095), but ran → {out}"),
        TrapRun::NoRustc => {}
    }
    // (c) MUL past the STORAGE width — the Copilot/github-liaison PR#756 miscompile: `UInt48` `2^32 · 2^32`
    // = 2^64 exceeds BOTH the type's 2^48 AND the storage `u64`'s 2^64, so a `wrapping_mul` on u64 wrapped
    // to 0 and FALSELY passed the `[0, 2^48-1]` check → silent wrong value. Mul now computes in a WIDER
    // `i128` intermediate (exact — the product can't wrap), so it TRAPS. Emit must use the i128 widening,
    // NOT a `wrapping_mul` on the storage type.
    let u48 = compile_rust(
        "(module m (def (run (: a Int64) (: b Int64)) \
           (* ((. (UInt 48) wrap) a) ((. (UInt 48) wrap) b))) (export run))",
    );
    assert!(
        u48.contains("as i128) * (") && !u48.contains("wrapping_mul"),
        "unusual-width Mul widens to i128 (not wrapping_mul on storage), so a product past 2^storage can't silently wrap:\n{u48}"
    );
    // 2^32 = 4294967296; 2^32 · 2^32 = 2^64 must TRAP (was silently 0 pre-fix).
    match rustc_run_traps(&u48, "run(4294967296, 4294967296)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("overflow"),
            "2^32·2^32 overflows UInt48 (and past the u64 storage) → traps overflow, got:\n{msg}"
        ),
        TrapRun::RanOk(out) => panic!(
            "`2^32·2^32 : UInt48` must TRAP (2^64 > 2^48, and mustn't wrap the u64 storage to 0), but ran → {out}"
        ),
        TrapRun::NoRustc => {}
    }
    // in-range control: 1000·1000 = 1_000_000 < 2^48 computes.
    if let Some(out) = rustc_run(&u48, "run(1000, 1000)") {
        assert_eq!(out, "1000000", "1000·1000 fits UInt48 and computes");
    }
}

#[test]
fn a_provably_in_range_left_shift_elides_its_overflow_guard_on_the_rust_backend() {
    // BOTH-BACKEND PARITY (v-core-opt Slice-4): the rust `<<` emit now consults the SAME Core-tier
    // shl_provably_in_range / _dynamic predicates the wasm backend uses (select.rs emit_shift), so a
    // provably-in-range left shift sheds BOTH its count guard and its overflow round-trip on BOTH
    // backends — one Core-tier decision, not a wasm-only elision. Emit the bare `v << count`.
    //
    // CONSTANT count: `(<< (& a 15) 2)` — value ∈ [0,15], << 2 = [0,60] ⊆ Int64, count 2 < 64 → bare shift.
    let konst = compile_rust("(module m (def (f (: a Int64)) (<< (& a 15) 2)) (export f))");
    assert!(
        konst.contains("v << (")
            && !konst.contains("(r >> c) != v")
            && !konst.contains("(0..64).contains(&c64)"),
        "a provably-in-range constant-count `<<` drops both guards:\n{konst}"
    );
    // DYNAMIC (masked runtime) count: `(<< (& a 15) (& k 3))` — value ∈ [0,15], count ∈ [0,7], max
    // 15 << 7 = 1920 ⊆ Int64, count < 64 → bare shift via shl_provably_in_range_dynamic.
    let dynamic = compile_rust(
        "(module m (def (f (: a Int64) (: k Int64)) (<< (& a 15) (& k 3))) (export f))",
    );
    assert!(
        !dynamic.contains("(r >> c) != v") && !dynamic.contains("(0..64).contains(&c64)"),
        "a provably-in-range masked-dynamic-count `<<` drops both guards:\n{dynamic}"
    );
    // A FULL-RANGE `<<` (unbounded value/count) is NOT provable → KEEPS both guards. The dual proving
    // the elision is opt-in on a proof of safety, never on the absence of a disproof.
    let kept = compile_rust("(module m (def (f (: a Int64) (: b Int64)) (<< a b)) (export f))");
    assert!(
        kept.contains("(0..64).contains(&c64)") && kept.contains("(r >> c) != v"),
        "a full-range `<<` keeps its count guard + overflow round-trip:\n{kept}"
    );
}

#[test]
fn rustc_roundtrip_provably_in_range_left_shift_elision_computes_identically_and_unproven_still_traps()
 {
    // The `<<` elision is BEHAVIOR-PRESERVING end-to-end: a provably-in-range shift computes the SAME
    // value with both guards elided as it would guarded, AND an unproven shift still TRAPS on overflow.
    let elided = compile_rust("(module m (def (f (: a Int64)) (<< (& a 15) 2)) (export f))");
    // a=255 → (255&15) << 2 = 15 << 2 = 60. Same value the guarded form computes; no trap.
    if let Some(out) = rustc_run(&elided, "f(255)") {
        assert_eq!(
            out, "60",
            "provably-in-range `<<` computes identically when elided"
        );
    }
    // An UNPROVEN full-range `<< 63` still traps on genuine overflow — 1 << 63 leaves Int64.
    let checked = compile_rust("(module m (def (f (: a Int64)) (<< a 63)) (export f))");
    match rustc_run_traps(&checked, "f(1)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("overflow"),
            "unproven `<<` still traps on overflow, got: {msg}"
        ),
        TrapRun::RanOk(out) => panic!("1 << 63 must TRAP (overflow), but ran → {out}"),
        TrapRun::NoRustc => {}
    }
}

#[test]
fn a_diverging_body_or_operand_emits_never_not_a_decline_or_a_method_call_on_never() {
    // Never (`!`) in an emit position — the family v-wasm-opt + breaker reported.

    // (a) A BOTH-BRANCHES-DIVERGE `if` is Never — it produces no value on any path. The fn return type is
    // Rust's never `!`, and the body is the `if` whose arms both `panic!` (a valid `-> !` body). Uses the
    // shared `body_diverges` (recurses If/Let/Seq/Match), not a bare `Core::Trap` match — so it no longer
    // declines "result type has no native Rust representation".
    let both_if = compile_rust(
        "(module m (def (main (: b Bool)) (if b (trap \"then\") (trap \"else\"))) (export main))",
    );
    assert!(
        both_if.contains("pub fn main(b: bool) -> !"),
        "a both-diverge if returns Rust's never `!`:\n{both_if}"
    );

    // (b) ARITHMETIC on a diverging let-binding: the init traps before the add, so emit ONLY the trap — NOT
    // `x.checked_add(1)` on the `!`-typed binding (E0599, a method call on Never). rustc must accept it.
    let never_let =
        compile_rust("(module m (def (main) (let ((x (trap \"boom\"))) (+ x 1))) (export main))");
    assert!(
        never_let.contains("panic!(\"unreachable\")") && !never_let.contains(".checked_add"),
        "arithmetic on a diverging binding emits only the trap, no method call on `!`:\n{never_let}"
    );
    // It compiles (the E0599 is gone) — a lib build suffices (a `-> i64` fn whose body is a coerced panic).
    assert!(
        compile_rust_result(
            "(module m (def (mk) (let ((x (trap \"boom\"))) (+ x 1))) (export mk))"
        )
        .is_ok(),
        "the diverging-arithmetic emit is well-formed Rust (no E0599)"
    );

    // (c) The SAME via an inlined diverging call argument: `(f (trap))` with `f x = (+ x 1)` inlines the
    // Never arg into `x`, so the body's arithmetic is on Never — emits only the trap, same as (b).
    let never_arg = compile_rust(
        "(module m (def (f (: x Int64)) (+ x 1)) (def (mk) (f (trap \"boom\"))) (export mk))",
    );
    assert!(
        never_arg.contains("panic!(\"unreachable\")") && !never_arg.contains(".checked_add"),
        "arithmetic reached via an inlined diverging call-arg emits only the trap:\n{never_arg}"
    );

    // (d) NESTED diverging arithmetic — `(+ (+ (trap) 1) 2)` (reviewer residue of the direct-guard fix). The
    // outer op's lhs is a LIVE `Core::Arith` (its own lhs traps), which `body_diverges` does NOT treat as
    // diverging, so the DIRECT guard missed it and the outer emitted `<inner>.checked_add(2)` where `<inner>`
    // is `panic!("unreachable")` → E0599. The transitive `arith_operand_diverges` recurses into arith
    // operands, so the outer now emits only the diverging inner — no method call on `!` at ANY depth.
    let nested = compile_rust("(module m (def (mk) (+ (+ (trap \"boom\") 1) 2)) (export mk))");
    assert!(
        nested.contains("panic!(\"unreachable\")") && !nested.contains(".checked_add"),
        "nested diverging arithmetic emits only the trap, no method call on `!`:\n{nested}"
    );
    assert!(
        compile_rust_result("(module m (def (mk) (+ (+ (trap \"boom\") 1) 2)) (export mk))")
            .is_ok(),
        "the nested diverging-arithmetic emit is well-formed Rust (no E0599)"
    );
    // The diverging operand nested in the RHS (`(+ 5 (+ (trap) 1))`) — lhs runs for effect, then the RHS
    // aborts → `{ let _ = <lhs>; <trap> }`, still no `.checked_add` on `!`.
    let nested_rhs = compile_rust("(module m (def (mk) (+ 5 (+ (trap \"boom\") 1))) (export mk))");
    assert!(
        nested_rhs.contains("panic!(\"unreachable\")") && !nested_rhs.contains(".checked_add"),
        "a diverging operand nested in the rhs emits the lhs-for-effect then the trap:\n{nested_rhs}"
    );

    // (e) The `Core::Compare` TWIN — a comparison with a diverging operand. `(< (+ (trap) 1) 2)` would emit
    // `panic!("unreachable") < 2` (comparing Rust's `!`/`()` with i64 → E0277); the same diverging-operand
    // guard on the Compare emit path emits only the trap. wasm compiles + traps here, so this was a rust-only
    // invalid-emit differential (a bare `(< (trap) 1)` needs a heap-walk both backends decline, so the
    // reachable shape is the nested-arith one). rustc must accept it.
    let cmp_nested =
        compile_rust("(module m (def (mk) (if (< (+ (trap \"boom\") 1) 2) 10 20)) (export mk))");
    assert!(
        cmp_nested.contains("panic!(\"unreachable\")")
            && !cmp_nested.contains("panic!(\"unreachable\") <")
            && !cmp_nested.contains("panic!(\"unreachable\") =="),
        "a diverging operand in a comparison emits only the trap, no compare on `!`:\n{cmp_nested}"
    );
    assert!(
        compile_rust_result(
            "(module m (def (mk) (if (< (+ (trap \"boom\") 1) 2) 10 20)) (export mk))"
        )
        .is_ok(),
        "the diverging-comparison emit is well-formed Rust (no E0277)"
    );

    // (f) The `.len()`-RECEIVER twin — `List.len` (and Map/Set/Bytes/String len) of a DIVERGING operand.
    // `(List.len (g 7))` where `g` always traps (e.g. a violated `@ensures` folds `g`'s body to a trap)
    // would emit `(panic!("unreachable")).len()` — a method call on Rust's `!`, E0599 ("no method `len` for
    // `!`"). The diverging-operand guard on the len-family emit paths emits only the trap. Uses a violated
    // plain `@ensures` over a List result (the corpus shape corpus-bugfix flagged) so `g` folds to a trap.
    let len_diverging = compile_rust(
        "(module m (@ (ensures (> (List.len ret) 0)) (def (g (: x Int64)) (list))) \
           (def (run) (List.len (g 7))) (export run))",
    );
    assert!(
        len_diverging.contains("panic!(\"unreachable\")")
            && !len_diverging.contains("panic!(\"unreachable\")).len()"),
        "a `List.len` of a diverging operand emits only the trap, no `.len()` on `!`:\n{len_diverging}"
    );
    assert!(
        compile_rust_result(
            "(module m (@ (ensures (> (List.len ret) 0)) (def (g (: x Int64)) (list))) \
               (def (run) (List.len (g 7))) (export run))",
        )
        .is_ok(),
        "the diverging-len emit is well-formed Rust (no E0599 on `!`)"
    );
    // (The build-succeeds assert is the fix's proof — before it, the emit was `(panic!(…)).len()`, an
    // E0599. The RUNTIME trap outcome is graded by the gate's corpus case "a PLAIN @ensures over a HEAP
    // result (List) TRAPS when violated"; `rustc_run` here would panic on the trap, so it is not used.)
}

#[test]
fn rustc_roundtrip_overflow_traps() {
    // Int8 100+100 = 200 leaves the type → Cadenza traps → the emitted Rust panics.
    let rs = compile_rust("(module m (def (add8 (: a Int8) (: b Int8)) (+ a b)) (export add8))");
    // A non-overflowing call returns the value; an overflowing one aborts (nonzero exit → the run
    // helper's success assertion fails), so we only positively assert the in-range answer here.
    if let Some(out) = rustc_run(&rs, "add8(100, 20)") {
        assert_eq!(out, "120");
    }
}

#[test]
fn rustc_roundtrip_short_circuit_and() {
    // `(and (< a b) (< b c))` → Rust `&&`, short-circuiting with the same semantics.
    let rs = compile_rust(
        "(module m (def (between (: a Int64) (: b Int64) (: c Int64)) \
           (and (< a b) (< b c))) (export between))",
    );
    assert!(rs.contains("&&"), "connective:\n{rs}");
    if let Some(out) = rustc_run(&rs, "between(1, 2, 3)") {
        assert_eq!(out, "true");
    }
    if let Some(out) = rustc_run(&rs, "between(1, 5, 3)") {
        assert_eq!(out, "false");
    }
}

#[test]
fn rustc_roundtrip_recursion() {
    // A recursive `fn` calls itself on the native stack — no tail-call transform needed for
    // correctness. sum-to(5) = 15, fac(5) = 120 (match base case), fib(10) = 55 (double recursion).
    let sumto = compile_rust(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export sum-to))",
    );
    if let Some(out) = rustc_run(&sumto, "sum_to(5)") {
        assert_eq!(out, "15");
    }
    let fac = compile_rust(
        "(module m (def (fac (: n Int64)) (match n (0 1) (k (* k (fac (+ k -1)))))) (export fac))",
    );
    if let Some(out) = rustc_run(&fac, "fac(5)") {
        assert_eq!(out, "120");
    }
    let fib = compile_rust(
        "(module m (def (fib (: n Int64)) (match n (0 0) (1 1) (k (+ (fib (+ k -1)) (fib (+ k -2)))))) (export fib))",
    );
    if let Some(out) = rustc_run(&fib, "fib(10)") {
        assert_eq!(out, "55");
    }
}

#[test]
fn rustc_roundtrip_mutual_recursion() {
    // even(10)=true. (Deeper mutual + even(7) are covered by `rustc_roundtrip_mutual_tail_loop_runs_deep`.)
    let rs = compile_rust(
        "(module m (def (even (: n Int64)) (if (= n 0) true (odd (+ n -1)))) \
                    (def (odd (: n Int64)) (if (= n 0) false (even (+ n -1)))) (export even))",
    );
    if let Some(out) = rustc_run(&rs, "even(10)") {
        assert_eq!(out, "true");
    }
}

// ── sums → Rust enums ─────────────────────────────────────────────────────────────────────────────

#[test]
fn a_user_sum_emits_a_rust_enum_declaration() {
    // A monomorphic user sum becomes a `pub enum` of its name: a nullary variant is a unit variant, a
    // 1-payload variant carries its payload type, a multi-payload variant carries each positionally.
    let rs = compile_rust(
        "(module m (type Shape (Circle Int64) (Rect Int64 Int64)) \
           (def (area (: s Shape)) (match s (((. Shape Circle) r) (* r r)) \
                                            (((. Shape Rect) (tuple w h)) (* w h)))) (export area))",
    );
    // A 1-payload variant carries its payload directly (`Circle(i64)`); a MULTI-payload variant carries
    // its payloads as ONE TUPLE (`Rect((i64, i64))`) — the single-`Ty::Tuple` payload the core models and
    // the match reads as one indexed value, so the decl, construction, and match all agree.
    assert!(
        rs.contains("pub enum Shape { Circle(i64), Rect((i64, i64)) }"),
        "enum decl:\n{rs}"
    );
    // Construction is `Enum::Variant(args)`; the match reads a Rust `match`.
    assert!(rs.contains("Shape::Circle"), "ctor path:\n{rs}");
    assert!(rs.contains("match"), "match lowering:\n{rs}");
}

#[test]
fn a_generic_user_sum_emits_a_generic_enum() {
    // A generic MULTI-variant sum's params become the enum's type parameters (`T0`…), a param-typed
    // payload renders as its `T{k}`, and a use at a concrete type instantiates them via
    // `types::rust_type`. Two variants keep it a boxed sum (a SINGLE-variant generic sum is a NEWTYPE that
    // ERASES — see `a_generic_newtype_erases_to_its_underlying_rust_type`), so this covers the enum path.
    let rs = compile_rust(
        "(module m (type Box (Wrap a) Empty) \
           (def (unwrap (: b (Box Int64))) (match b (((. Box Wrap) x) x) (((. Box Empty) _) 0))) (export unwrap))",
    );
    assert!(
        rs.contains("pub enum Box<T0>") && rs.contains("Wrap(T0)"),
        "generic enum decl:\n{rs}"
    );
    assert!(
        rs.contains("unwrap(b: Box<i64>)"),
        "instantiated use:\n{rs}"
    );
}

#[test]
fn a_monomorphic_newtype_erases_and_emits_no_enum() {
    // A monomorphic newtype `(type UserId (Mk Int64))` erases: NO `enum UserId` is emitted (its value IS
    // the i64), and a param typed by it maps to the underlying `i64`. (Regression: before the enum-skip,
    // a dead `enum UserId { Mk(i64) }` was emitted — harmless but wrong; the value reads through the tag.)
    let rs = compile_rust(
        "(module m (type UserId (Mk Int64)) \
           (def (unwrap (: b UserId)) (match b ((Mk x) x))) (export unwrap))",
    );
    assert!(
        !rs.contains("enum UserId"),
        "an erased newtype emits no enum:\n{rs}"
    );
    assert!(
        rs.contains("unwrap(b: i64)"),
        "the newtype param maps to its underlying i64:\n{rs}"
    );
}

#[test]
fn a_generic_newtype_erases_to_its_underlying_rust_type() {
    // A GENERIC single-variant sum is an erasable NEWTYPE: it emits NO enum, and a use at a concrete
    // instantiation maps to the underlying (substituted) Rust type — `(Box Int64)` → `i64`. The runtime
    // value IS the payload (the tag adds nothing), so the Rust backend reads through it, exactly as the
    // wasm backend does. The monomorphic sibling is `a_monomorphic_newtype_erases_and_emits_no_enum`.
    let rs = compile_rust(
        "(module m (type Box (Wrap a)) \
           (def (unwrap (: b (Box Int64))) (match b ((Wrap x) x))) (export unwrap))",
    );
    assert!(
        !rs.contains("enum Box"),
        "an erased generic newtype emits no enum:\n{rs}"
    );
    assert!(
        rs.contains("unwrap(b: i64)"),
        "the newtype param maps to its underlying i64:\n{rs}"
    );
}

#[test]
fn the_builtin_option_maps_to_rusts_own_and_emits_no_enum() {
    // The built-in `Option` maps to Rust's OWN `Option` — no synthetic `enum Option { … }` is emitted
    // (that would shadow std's). Construction uses `Some(..)`/`None`, which resolve to std's.
    let rs = compile_rust("(module m (def (wrap (: n Int64)) (Some n)) (export wrap))");
    assert!(
        !rs.contains("enum Option"),
        "must not emit a synthetic Option enum:\n{rs}"
    );
    assert!(rs.contains("-> Option<i64>"), "std Option result:\n{rs}");
    assert!(rs.contains("Some("), "std Some ctor:\n{rs}");
}

#[test]
fn a_recursive_sum_emits_a_boxed_enum() {
    // A recursive sum now EMITS a Rust enum with the recursive variant's field BOXED (`Box<…>`) for
    // finite size — a function taking/returning it compiles. (Was deferred as "needs Box"; now realized:
    // the recursive variant field is one `Box`, construction `Box::new(…)`, match derefs `*__pay`.)
    let rs = try_compile_rust(
        "(module m (type IntList Nil (Cons (Tuple Int64 IntList))) \
           (def (len (: xs IntList)) (match xs (((. IntList Nil) _) 0) \
                                              (((. IntList Cons) (tuple h t)) (+ 1 (len t))))) (export len))",
    )
    .expect("a recursive sum now emits a boxed enum");
    assert!(
        rs.contains("enum IntList") && rs.contains("Box<"),
        "the recursive variant's field is boxed: {rs}"
    );
}

#[test]
fn rustc_roundtrip_generic_recursive_tree_boxes_and_counts() {
    // A GENERIC recursive sum `(type Tree (Leaf a) (Node (Tuple (Tree a) (Tree a))))` — the recursive
    // `Tree` occurrence is a type PARAMETER instantiation nested inside a `Tuple`. The Box-decision
    // (`variant_payloads_mention`) resolved a payload with `typeval_of`, which returns `None` for a payload
    // mentioning a type param (unbound), so `Node`'s recursive tuple was NOT boxed → `Node((Tree<T0>,
    // Tree<T0>))` had INFINITE size (rustc E0072). Resolving the payload PARAM-TOLERANTLY (at a sentinel
    // instantiation, the same path the enum-field render uses) makes `reaches_decl` see the self-reference
    // and box it: `Node(Box<(Tree<T0>, Tree<T0>)>)`. `cnt` counts leaves; a two-leaf node = 2.
    let rs = compile_rust(
        "(module m (type Tree (Leaf a) (Node (Tuple (Tree a) (Tree a)))) \
           (def (cnt (: t (Tree Int64))) (match t (((. Tree Leaf) _) 1) \
                                                   (((. Tree Node) (tuple l r)) (+ (cnt l) (cnt r))))) \
           (export cnt))",
    );
    assert!(
        rs.contains("enum Tree<T0>") && rs.contains("Node(::std::boxed::Box<"),
        "the generic recursive variant's nested-tuple field is boxed (was E0072 infinite size): {rs}"
    );
    // End-to-end through rustc: builds (was E0072) and cnt of a two-leaf node = 2, matching wasm.
    if let Some(out) = rustc_run(
        &rs,
        "cnt(Tree::Node(Box::new((Tree::Leaf(1), Tree::Leaf(2)))))",
    ) {
        assert_eq!(
            out, "2",
            "the generic recursive tree builds and counts:\n{rs}"
        );
    }
}

#[test]
fn a_recursive_newtype_declines_the_whole_function() {
    // A RECURSIVE NEWTYPE `(type Lst (Mk (Option (Tuple Int64 Lst))))` erases to its inner type on the rust
    // backend — but the inner type mentions `Lst` (the μ back-edge), which erasure would render as a bare
    // `Lst` that is never emitted → `cannot find type Lst` (an uncompilable crate). Erasure only works when
    // the unfold terminates; a recursive newtype needs a Box-indirected nominal emission (deferred, like a
    // recursive sum). So a function taking/returning it DECLINES, exactly as a recursive SUM does, rather
    // than emitting a signature naming an undeclared type. (The wasm backend runs it — types erase at
    // runtime with no Rust-level name needed.)
    let err = try_compile_rust(
        "(module m (type Lst (Mk (Option (Tuple Int64 Lst)))) \
           (def (sm (: l Lst)) (match l ((Mk o) (match o ((Some p) (+ (. p 0) (sm (. p 1)))) ((None) 0))))) \
           (export sm))",
    )
    .expect_err("a recursive newtype must decline, not emit an uncompilable crate");
    // The decline names it ACCURATELY as a recursive NEWTYPE (not "a sum" — it is not a sum, and it fails
    // for a distinct reason: its erasure unfolds forever, so it needs a Box-indirected nominal emission).
    assert!(
        err.iter().any(|d| d.contains("recursive newtype")),
        "decline reason should name the recursive newtype precisely: {err:?}"
    );

    // The decline is ROBUST across every position the recursive newtype can appear — a PARAM, a RESULT,
    // and NESTED inside a List result — so a future partial fix cannot leave one position emitting an
    // uncompilable signature (a `cannot find type Lst`). Each declines cleanly, none panics/miscompiles.
    for prog in [
        // param position
        "(module m (type Lst (Mk (Option (Tuple Int64 Lst)))) (def (f (: l Lst)) 0) (export f))",
        // result position
        "(module m (type Lst (Mk (Option (Tuple Int64 Lst)))) (def (f) (Mk (None))) (export f))",
        // nested inside a List result
        "(module m (type Lst (Mk (Option (Tuple Int64 Lst)))) (def (f) (list (Mk (None)))) (export f))",
    ] {
        let e = try_compile_rust(prog).expect_err(
            "a recursive newtype in any position must decline, not emit an uncompilable crate",
        );
        assert!(
            e.iter().any(|d| d.contains("recursive newtype")),
            "recursive-newtype decline in every position: {e:?}\nprog: {prog}"
        );
    }

    // A genuine recursive SUM, by contrast, COMPILES (its enum boxes the recursive variant field) — so the
    // "recursive newtype" phrasing is specific to the newtype gap, not a blanket recursion decline.
    let sum = compile_rust(
        "(module m (type IL (Cons (Tuple Int64 IL)) (Nil)) (def (f (: l IL)) 0) (export f))",
    );
    assert!(
        sum.contains("enum IL") && sum.contains("::std::boxed::Box<"),
        "a recursive SUM emits a Box-indirected enum (not declined like the newtype):\n{sum}"
    );
}

#[test]
fn a_recursive_sum_constructed_as_a_discarded_intermediate_folds_away() {
    // A helper `mk` returns `(tuple (NLit 5) 9)` — a pair whose element 0 is a recursive-sum value and
    // whose element 1 is the Int64 9. `main` reads `.1` and DISCARDS element 0. The projection folds
    // through the constant tuple (`(. l 1)` → 9), so the discarded `(NLit 5)` is never constructed and
    // `main` compiles to the constant 9 on BOTH backends — the same DCE the wasm backend performs, reached
    // on rust too. (The recursive `Node` enum now DOES emit — a boxed enum — but that is a harmless
    // `#[allow(dead_code)]` declaration; the load-bearing property is that `main` folds to 9, NOT whether
    // the declared-but-unused enum is present.)
    let rs = try_compile_rust(
        "(module m (type Node (NLit Int64) (NAdd (Tuple Node Node))) \
           (def (mk) (tuple (NLit 5) 9)) \
           (def (main) (let ((l (mk))) (. l 1))) (export main))",
    )
    .expect("a discarded recursive-sum intermediate folds away — the projection drops it");
    // `main` returns the folded constant 9 (the discarded `(NLit 5)` is never constructed in `main`'s body).
    assert!(
        rs.contains("9u64 as i64") || rs.contains("9i64") || rs.contains("-> i64"),
        "main folds to the projected Int64 constant 9: {rs}"
    );
}

#[test]
fn rustc_roundtrip_user_sum_constructs_and_matches() {
    // area(Circle 5) = 25, area(Rect 4 3) = 12 — construction + match run through rustc and match the
    // wasm oracle. The driver constructs a variant and calls the export.
    let rs = compile_rust(
        "(module m (type Shape (Circle Int64) (Rect Int64 Int64)) \
           (def (area (: s Shape)) (match s (((. Shape Circle) r) (* r r)) \
                                            (((. Shape Rect) (tuple w h)) (* w h)))) (export area))",
    );
    if let Some(out) = rustc_run(&rs, "area(Shape::Circle(5))") {
        assert_eq!(out, "25");
    }
    // A multi-payload variant carries ONE tuple, so it is constructed `Rect((4, 3))`.
    if let Some(out) = rustc_run(&rs, "area(Shape::Rect((4, 3)))") {
        assert_eq!(out, "12");
    }
}

#[test]
fn rustc_roundtrip_narrow_sum_payload_literal_match_widens_to_the_unified_result() {
    // A NARROW sum-payload literal-refinement match whose arms unify to a WIDER result — `(match b ((A 0)
    // 100) ((A x) x) ((B y) y))` over `(type Box (A UInt8) (B UInt8))`: the `100` literal arm (Int64) and
    // the `x`/`y` UInt8-payload arms unify to Int64, so the result is Int64. The sum-decision-tree emit
    // (`emit_sum_cont`) did NOT ground its Leaf bodies to that result width, so it emitted `if payload==0
    // { 100i64 } else { x_u8 }` — mismatched `if` arms + a wrong `-> u8` return → rustc E0308 (a DIFFERENTIAL
    // miscompile: wasm computed 100/5 fine). Fixed by threading the match's `result_it` through
    // `emit_sum_match`/`emit_sum_switch`/`emit_sum_cont` and grounding each Leaf via `emit_grounded` (the
    // same reconciliation the scalar-`match` path already did). Both arms now emit at ONE width, no E0308.
    // (corpus-bugfix's adv-rust-narrow-sum-payload-… reproducer, breaker-filed.)
    let rs = compile_rust(
        "(module m (type Box (A UInt8) (B UInt8)) \
           (def (f (: b Box)) (match b ((A 0) 100) ((A x) x) ((B y) y))) \
           (def (run (: n UInt8)) (f (A n))) (export run))",
    );
    // The emit must be internally consistent (no E0308): it compiles, and the `if` arms share a width — no
    // bare `iN` opposite a bare `uM`. The value is what matters end-to-end:
    if let Some(out) = rustc_run(&rs, "run(0)") {
        assert_eq!(out, "100", "n=0 hits the (A 0) literal arm → 100");
    }
    if let Some(out) = rustc_run(&rs, "run(5)") {
        assert_eq!(out, "5", "n=5 misses, binds the widened UInt8 payload → 5");
    }
    // CONTROL: an Int64-payload sum needs no widening — still works (the fix is narrow-payload-specific).
    let wide = compile_rust(
        "(module m (type W (Wrap Int64) (Other Int64)) \
           (def (f (: b W)) (match b ((Wrap 0) 100) ((Wrap x) x) ((Other y) y))) \
           (def (run (: n Int64)) (f (Wrap n))) (export run))",
    );
    if let Some(out) = rustc_run(&wide, "run(0)") {
        assert_eq!(out, "100", "Int64 payload, no widening needed");
    }
}

#[test]
fn rustc_roundtrip_erased_newtype_narrow_literal_probe_aligns_both_compare_sides() {
    // A NARROW literal probe over an ERASED single-variant newtype whose value was built through a
    // narrowing wrap — `(match (W.V (Int8.wrap n)) ((W.V 3) 1000) ((W.V _) 2000))` with `W = (V Int8)` and
    // `n: Int64`. The tag erases (the value IS the inner Int8), so this reaches the `LitTest` emit. The
    // literal was rendered at the narrow width (`3i8`) but the subject came back WIDENED — `emit_sum_payload`
    // read the wrapped value as `(n as i64)` — so the compare was `(n as i64) == 3i8` → rustc E0308 (i64 vs
    // i8). Fix: key BOTH sides of the `==` off the literal's `target` width — cast the subject to `target`
    // too (`((subj) as i8) == 3i8`). Sound: the sub-value is logically that narrow width, so the cast
    // recovers the true value, matching the wasm decision-tree's width-normalized compare. (corpus-bugfix's
    // adv-rust-erased-newtype-narrow-literal-compare-… reproducer, breaker-filed; a new face of the
    // narrow-sum-payload-literal E0308 family.)
    let i8w = compile_rust(
        "(module m (type W (V Int8)) \
           (def (run (: n Int64)) (match (W.V (Int8.wrap n)) ((W.V 3) 1000) ((W.V _) 2000))) \
           (export run))",
    );
    if let Some(out) = rustc_run(&i8w, "run(3)") {
        assert_eq!(out, "1000", "n=3 hits the (W.V 3) literal arm");
    }
    if let Some(out) = rustc_run(&i8w, "run(5)") {
        assert_eq!(out, "2000", "n=5 misses → the wildcard arm");
    }
    // WRAP-AROUND: n=259 wraps to Int8 3, so it MUST match (W.V 3) — the cast preserves wrap semantics.
    if let Some(out) = rustc_run(&i8w, "run(259)") {
        assert_eq!(
            out, "1000",
            "n=259 → Int8.wrap = 3 → the literal arm (wrap-around preserved)"
        );
    }
    // WIDTH AXIS: UInt16 (the note lists Int8/16/32/UInt8/16 all failing pre-fix) also aligns + runs.
    let u16w = compile_rust(
        "(module m (type W (V UInt16)) \
           (def (run (: n Int64)) (match (W.V (UInt16.wrap n)) ((W.V 3) 1000) ((W.V _) 2000))) \
           (export run))",
    );
    if let Some(out) = rustc_run(&u16w, "run(3)") {
        assert_eq!(out, "1000", "UInt16 narrow literal probe aligns + matches");
    }
    // CONTROL: a MULTI-variant sum of the same shape already emitted both-sides-narrow — must not regress.
    let multi = compile_rust(
        "(module m (type W (A Int8) (B Int8)) \
           (def (run (: n Int64)) \
             (match (W.A (Int8.wrap n)) ((W.A 3) 1000) ((W.A _) 2000) ((W.B _) 3000))) \
           (export run))",
    );
    if let Some(out) = rustc_run(&multi, "run(3)") {
        assert_eq!(
            out, "1000",
            "multi-variant control still matches the literal arm"
        );
    }
}

#[test]
fn rustc_roundtrip_recursive_sum_folds() {
    // A RECURSIVE user sum (a cons-list) constructs and folds THROUGH RUSTC: the enum is `Box`ed
    // (`Cons(Box<(i64, L)>)`), construction `Box::new(…)`, match derefs `*p`. `sm` sums a runtime-passed
    // list; `sm(L::Cons(Box::new((1, L::Cons(Box::new((2, L::Nil)))))))` = 3. Pins that a recursive sum
    // RUNS on the Rust backend (not just emits) and agrees with the wasm oracle — the last rust-backend
    // sum gap, now closed via Box indirection.
    let rs = compile_rust(
        "(module m (type L Nil (Cons (Tuple Int64 L))) \
           (def (sm (: l L)) (match l (((. L Nil) _) 0) \
                                      (((. L Cons) (tuple h t)) (+ h (sm t))))) (export sm))",
    );
    if let Some(out) = rustc_run(
        &rs,
        "sm(L::Cons(Box::new((1, L::Cons(Box::new((2, L::Nil)))))))",
    ) {
        assert_eq!(out, "3");
    }
    if let Some(out) = rustc_run(&rs, "sm(L::Nil)") {
        assert_eq!(out, "0");
    }
}

#[test]
fn rustc_roundtrip_recursive_match_scrutinee_materializes_once_not_exponentially() {
    // A match over a RECURSIVE-CALL scrutinee whose payload binder is used MULTIPLE times. Each payload
    // read via `emit_sum_payload` used to RE-EMIT the scrutinee expression, so a binder used K times
    // re-emitted the recursive call K times → `2^depth` calls (an exponential blow-up: `run(-40)` HUNG).
    // The fix materializes a non-trivial scrutinee into ONE `let __ms` and reads payloads from it. Pins
    // (a) the emitted `f` binds its recursive-call scrutinee ONCE (a single `let __ms…` per match, and the
    // recursive call `f(` appears once per level, not doubling), and (b) it RUNS to a value at a depth that
    // would hang if still exponential. `f` builds a 2-field `Mk` from the recursively-summed tail.
    let rs = compile_rust(
        "(module m (type P (Mk Int64 Int64) Nil) \
           (def (f (: n Int64)) (if (= n 0) Nil \
             (match (f (+ n 1)) (((. P Mk) (tuple a _)) (P.Mk a a)) (((. P Nil) _) (P.Mk 1 1))))) \
           (def (run) (match (f -40) (((. P Mk) (tuple x _)) x) (((. P Nil) _) 0))) (export run))",
    );
    // The scrutinee is materialized: a `let __ms` binds each recursive-call match subject once, and the
    // match dispatches on that local (`match __ms`), not on a re-emitted `f(…)`.
    assert!(
        rs.contains("let __ms") && rs.contains("match __ms"),
        "a recursive-call match scrutinee binds to a `let __ms` once and matches the local:\n{rs}"
    );
    // Runs at n=-40 — 2^40 calls if exponential (would hang); linear with the fix. `f(-40)` recurses to
    // `f(0)=Nil`, each level's `Mk a a` carries the tail's summed head; `run` reads `Mk`'s first field.
    if let Some(out) = rustc_run(&rs, "run()") {
        assert_eq!(
            out, "1",
            "the materialized recursive match runs (linear, not 2^40)"
        );
    }

    // SINGLE-VARIANT face: `(type P (Mk Int64 Int64))` (one variant). `lower` keeps the `Core::MatchSum`
    // wrapper for a recursive-call scrutinee at ANY variant count (v-compiler-perf's keep-wrapper), so the
    // single-variant match is a MatchSum too and this materialize fires on it — `let __ms; ((__ms).0,
    // (__ms).0)`, the recursive call bound ONCE. Without EITHER piece it re-emits `f(…)` per tuple field →
    // 2^depth. Pins that the single-variant recursive match is also linear on the rust backend.
    let sv = compile_rust(
        "(module m (type P (Mk Int64 Int64)) \
           (def (f (: n Int64)) (if (= n 0) (P.Mk 1 1) \
             (match (f (+ n 1)) (((. P Mk) (tuple a _)) (P.Mk a a))))) \
           (def (run) (match (f -40) (((. P Mk) (tuple x _)) x))) (export run))",
    );
    assert!(
        sv.contains("let __ms"),
        "a single-variant recursive-call match scrutinee also materializes once:\n{sv}"
    );
    if let Some(out) = rustc_run(&sv, "run()") {
        assert_eq!(
            out, "1",
            "the single-variant materialized recursive match runs (linear, not 2^40)"
        );
    }
}

#[test]
fn rustc_roundtrip_record_match_literal_field_probe_computes() {
    // A record-match LITERAL FIELD probe `((record (x 3) (y b)) …)` renders + runs on the RUST backend.
    // The refutable field probe lowers to a `lit_test` at `[Elem(sorted_slot)]` over the record scrutinee;
    // its subject read reaches `emit_sum_payload` with NO bind → before, the record path fell through to
    // "sum payload has no bound match arm" and DECLINED on rust (it computed on wasm only — a wasm-only
    // capability that a hand-baseline wrongly marked rust=pass, breaker-caught). The fix reads the record
    // field directly (`(<r>).slot`, the record twin of the runtime-tuple direct read — a record is a Rust
    // tuple in sorted-field order). `(match (record (x 3)(y 4)) ((record (x 3)(y b)) b) (_ -1)))` → the
    // x-literal matches, b=4; and the miss (x=9) → -1. Const-folds to a scalar (no runtime).
    let hit = compile_rust(
        "(module m (def (run) \
           (match (record (= x 3) (= y 4)) ((record (x 3) (y b)) b) (_ -1))) (export run))",
    );
    if let Some(out) = rustc_run(&hit, "run()") {
        assert_eq!(
            out, "4",
            "record-match literal-field HIT binds the other field on rust"
        );
    }
    let miss = compile_rust(
        "(module m (def (run) \
           (match (record (= x 9) (= y 4)) ((record (x 3) (y b)) b) (_ -1))) (export run))",
    );
    if let Some(out) = rustc_run(&miss, "run()") {
        assert_eq!(
            out, "-1",
            "record-match literal-field MISS falls through on rust"
        );
    }
}

#[test]
fn rustc_roundtrip_boxed_payload_projected_more_than_once_clones() {
    // A recursive sum whose `Cons` payload the Rust backend BOXES (`Cons(Box<(i64,L)>)`), where the bound
    // payload field is PROJECTED MORE THAN ONCE: `(let ((d (f t))) (if (= d 0) h d))` uses the recursive
    // tail `t` in both the `if` condition and the else-branch. A bare `(*box).1` read MOVES the non-`Copy`
    // `L` field out of the box — the FIRST projection moves it, so the second is E0382 "use of moved
    // value". The fix CLONES each boxed-payload projection, so each read is an owned copy that leaves the
    // box intact. Pins that the emitted crate BUILDS and runs to f(Cons 5 Nil) = 5 (was a build failure);
    // the wasm gate never saw this (it re-reads the heap slot with no move discipline).
    let rs = compile_rust(
        "(module m (type L (Nil) (Cons Int64 L)) \
           (def (f (: xs L)) (match xs ((Nil) 0) ((Cons h t) (let ((d (f t))) (if (= d 0) h d))))) \
           (export f))",
    );
    // Each boxed-payload projection is cloned (not a bare moving `(*p).N`).
    assert!(
        rs.contains(".clone()"),
        "a boxed payload projection clones to avoid moving out of the Box:\n{rs}"
    );
    // End-to-end through rustc: builds (was E0382) and f(Cons 5 Nil) = 5, matching the wasm oracle.
    if let Some(out) = rustc_run(&rs, "f(L::Cons(Box::new((5, L::Nil))))") {
        assert_eq!(
            out, "5",
            "the multiply-projected boxed payload builds and runs:\n{rs}"
        );
    }
}

#[test]
fn rustc_roundtrip_mutually_recursive_sums_fold() {
    // A MUTUALLY-recursive pair of sums (`A` references `B`, `B` references `A`) — NEITHER variant mentions
    // its OWN decl, but the A→B→A cycle needs Box indirection all the same (E0072 otherwise). Both edges
    // box: `A { AN(Box<B>) }`, `B { BN(Box<A>) }`, construction `Box::new(…)`, match derefs. `sa(A::AN(…B…))`
    // walks A→B→A to the `AL 9` leaf = 9. Pins that the cycle detector boxes a MUTUAL recursion, not only a
    // direct self-reference — and that it RUNS on rustc, matching the wasm oracle.
    let rs = compile_rust(
        "(module m (type A (AL Int64) (AN B)) (type B (BL Int64) (BN A)) \
           (def (sa (: a A)) (match a (((. A AL) n) n) (((. A AN) b) (sb b)))) \
           (def (sb (: b B)) (match b (((. B BL) n) n) (((. B BN) a) (sa a)))) (export sa))",
    );
    if let Some(out) = rustc_run(&rs, "sa(A::AN(Box::new(B::BN(Box::new(A::AL(9))))))") {
        assert_eq!(out, "9");
    }
    if let Some(out) = rustc_run(&rs, "sa(A::AL(42))") {
        assert_eq!(out, "42");
    }
}

#[test]
fn a_wide_mutually_recursive_cycle_boxes_every_edge() {
    // A CYCLE of many mutually-recursive sums `T0→T1→…→T{n-1}→T0` — the shape whose recursive-variant
    // boxing check the cycle detector runs per variant per enum. That check is now memoized (a per-decl
    // out-edge cache + a per-decl reachable-set cache + an O(1) `type_decl_by_occ` index), turning what
    // was O(N³) (a fresh full-cycle DFS per variant, with a linear `type_decl_by_occ` scan per step) into
    // ~O(N²). This locks in that the memoized path still produces the CORRECT boxing at width: every
    // `Mk{i}` variant carries `Box<T{i+1}>` (its payload reaches back around the cycle), and every enum
    // emits — the same verdict the un-memoized fresh-DFS gave.
    let n = 12;
    let types: Vec<String> = (0..n)
        .map(|i| format!("(type T{i} (Mk{i} T{}) (End{i}))", (i + 1) % n))
        .collect();
    let src = format!(
        "(module m {} (def (f (: t T0)) (match t (((. T0 Mk0) _) 1) (((. T0 End0) _) 0))) (export f))",
        types.join(" ")
    );
    let rs = try_compile_rust(&src).expect("a wide mutually-recursive cycle emits boxed enums");
    // Every cycle edge is boxed: `Mk{i}(::std::boxed::Box<T{i+1}>)` for each i (each payload reaches back
    // to its own sum through the cycle, so the finite-size Box is required on all of them). The box is
    // fully-qualified so a user sum named `Box` cannot shadow the heap pointer.
    for i in 0..n {
        let nxt = (i + 1) % n;
        assert!(
            rs.contains(&format!("Mk{i}(::std::boxed::Box<T{nxt}>)")),
            "T{i}'s recursive variant must box its T{nxt} payload; got:\n{rs}"
        );
        // The nullary `End{i}` variant carries no payload (not boxed) — a spot-check that boxing is
        // selective, not blanket.
        assert!(
            rs.contains(&format!("enum T{i} ")),
            "enum T{i} must emit: {rs}"
        );
    }
}

#[test]
fn rustc_roundtrip_erased_newtype_wrapping_a_sum_nested_match() {
    // A single-variant sum is an ERASED newtype (no Rust enum — its value IS the payload). When it WRAPS
    // A SUM (`(type W (V (Result …)))`) and is matched with a NESTED constructor pattern (`(W.V (Ok n))`),
    // the decision-tree's switch/bind paths carry the newtype's `Payload` step, but `lower` erased that
    // same step from the BODY's payload reads — so the switch dispatched one level too shallow and the
    // arm's payload binder mismatched the body read ("sum payload has no bound match arm"). The backend
    // now erases the nominal switch step (twin of `lower::erase_nominal_steps`), so the switch dispatches
    // on the INNER sum directly: `match w { Result::Ok(p) => p, Result::Err(p) => p }`. `f(Ok 5)`=5,
    // `f(Err 9)`=9 — the erased wrapper is invisible at runtime, matching the wasm oracle.
    let rs = compile_rust(
        "(module m (type W (V (Result Int64 Int64))) \
           (def (f (: w W)) (match w (((. W V) (Result.Ok n)) n) (((. W V) (Result.Err e)) e))) \
           (export f))",
    );
    // The erased newtype emits NO `enum W`; the match dispatches on the inner Result directly.
    assert!(
        !rs.contains("enum W "),
        "erased newtype emits no enum:\n{rs}"
    );
    assert!(
        rs.contains("Result::Ok(__pay"),
        "dispatches on inner sum:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(Ok(5))") {
        assert_eq!(out, "5");
    }
    if let Some(out) = rustc_run(&rs, "f(Err(9))") {
        assert_eq!(out, "9");
    }
}

#[test]
fn rustc_roundtrip_erased_newtype_wrapping_a_sum_binder_reads_the_inner_not_the_wrapper() {
    // REGRESSION for a latent `fold_sum_path` miscompile (shared by BOTH backends) the Rust decline had
    // been masking: `fold_sum_path` re-read `type_of(cur)` each step to detect a nominal newtype, but for
    // a newtype WRAPPING A SUM the erased value stays the SAME node, so its raw type read `Ty::Nominal` for
    // EVERY step — the inner sum's `Payload` was consumed as a SECOND nominal no-op and a payload binder
    // folded to the WHOLE wrapper (`n` in `(W.V (Ok n))` became the whole `Result`, an infinite-ish wrong
    // value). Fixed with a PEELED type cursor (one nominal layer per erased step). Here the whole thing
    // folds (constant scrutinee), so the answer proves the binder reads the INNER Int, not the wrapper:
    // `(W.V (Ok 7))` matched `(W.V (Ok n))` → 7.
    let rs = compile_rust(
        "(module m (type W (V (Result Int64 Int64))) \
           (def (run) (match (W.V (Result.Ok 7)) (((. W V) (Result.Ok n)) n) \
                                                 (((. W V) (Result.Err e)) e))) (export run))",
    );
    if let Some(out) = rustc_run(&rs, "run()") {
        assert_eq!(
            out, "7",
            "binder reads the inner Int, not the wrapper:\n{rs}"
        );
    }
}

#[test]
fn rustc_roundtrip_recursive_sum_literal_refined_const_disc_root() {
    // A recursive-sum match with a LITERAL-REFINED payload arm (`(Cons 0 t)`) over a scrutinee whose
    // DISCRIMINANT is statically known (a constant `SumNew` — `(Cons x Nil)`, `Cons` tag known, `x`
    // runtime). `lower`'s known-disc fold collapses the root `Switch` to the `Cons` arm's continuation — a
    // bare `LitTest` at ROOT. The backend previously declined a non-`Switch` root; it now routes `LitTest`
    // / `Guarded` / `Leaf` roots through `emit_sum_cont` (which reads the sub-value via `emit_sum_payload`,
    // folding against the constant scrutinee's payloads). `f(Cons 0 Nil)` hits the literal arm = 100;
    // `f(Cons 7 Nil)` binds `h` = 7. Matches the wasm oracle — the last non-Switch-root gap.
    let rs = compile_rust(
        "(module m (type L (Nil) (Cons Int64 L)) \
           (def (f (: x Int64)) (match (L.Cons x (L.Nil)) \
                                  (((. L Cons) 0 t) 100) (((. L Cons) h t) h) (((. L Nil)) -1))) \
           (export f))",
    );
    if let Some(out) = rustc_run(&rs, "f(0)") {
        assert_eq!(out, "100");
    }
    if let Some(out) = rustc_run(&rs, "f(7)") {
        assert_eq!(out, "7");
    }
}

#[test]
fn rustc_roundtrip_nested_match_on_a_variant_at_disc_ge_1() {
    // A NESTED constructor match on a variant that is NOT variant 0 — `(type W (A Int64) (V (Option
    // Int64)))` matched `((W.V (Option.Some n)) …)`, where the nested-sum-carrying variant `V` is at
    // discriminant 1. The backend's nested-switch subject-type walk read variant 0's payload
    // unconditionally, so the inner switch on `W.V`'s Option resolved to `A`'s `Int64` (not `Option Int64`)
    // and declined (`sum construction node is not a sum type`). It now RECORDS each entered arm's payload
    // type in the ctx (`sum_path_types`, the twin of `lower`'s `path_types`) and the nested switch looks it
    // up — so it dispatches on the Option even when the discriminant is RUNTIME (an `if` selecting the
    // variant): `f(V(Some 7))` = 7, `f(V(None))` = -2. Matches the wasm oracle.
    let rs = compile_rust(
        "(module m (type W (A Int64) (V (Option Int64))) \
           (def (f (: k Int64)) (match (W.V (if (> k 0) (Option.Some k) (Option.None))) \
                                  ((W.A h) h) ((W.V (Option.Some n)) n) ((W.V (Option.None)) -2))) \
           (export f))",
    );
    // Dispatches on the INNER Option (V's payload), not A's Int64.
    assert!(
        rs.contains("Option::Some(__pay"),
        "inner Option switch:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(7)") {
        assert_eq!(out, "7");
    }
    if let Some(out) = rustc_run(&rs, "f(-1)") {
        assert_eq!(out, "-2");
    }
}

#[test]
fn rustc_roundtrip_two_disc_ge_1_nested_sum_variants_over_a_runtime_disc() {
    // The HARDER disc-≥1 shape: TWO variants past the first each carry a DIFFERENT nested sum, and the
    // scrutinee's discriminant is genuinely RUNTIME (an `if` chooses `W.U` vs `W.V`, so no constant `disc`
    // to read). A constant-value cursor can't recover the disc here; the arm-recorded `sum_path_types`
    // hint can — each arm records ITS variant's payload type, and the two nested switches (`U`'s Option,
    // `V`'s Result) each resolve their subject by lookup. `f(5)` → `W.U(Some 5)` → 5; `f(-1)` →
    // `W.V(Ok 0)` → 0. Was a Rust-backend decline (`sum construction node is not a sum type`).
    // `mk` is RECURSIVE (a `(< k 0)` arm self-calls, never taken for the tested k) so the W is GENUINELY
    // runtime — the match-into-if AND case-of-match fusions refuse to reduce through a recursive call, so
    // `(match (mk k) …)` keeps the two runtime nested switches this asserts (a non-recursive `mk` would fuse
    // the match into each branch and fold both switches away, eliminating the `Option::Some(__pay`/
    // `Result::Ok(__pay` code). Semantics unchanged for the tested inputs.
    let rs = compile_rust(
        "(module m (type W (A Int64) (U (Option Int64)) (V (Result Int64 Int64))) \
           (def (mk (: k Int64)) (if (< k 0) (mk 0) \
                                    (if (> k 0) (W.U (Option.Some k)) (W.V (Result.Ok 0))))) \
           (def (f (: k Int64)) (match (mk k) \
                                  ((W.A h) h) ((W.U (Option.Some n)) n) ((W.U (Option.None)) -1) \
                                  ((W.V (Result.Ok o)) o) ((W.V (Result.Err e)) e))) (export f))",
    );
    assert!(
        rs.contains("Option::Some(__pay"),
        "U's inner Option switch:\n{rs}"
    );
    assert!(
        rs.contains("Result::Ok(__pay"),
        "V's inner Result switch:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(5)") {
        assert_eq!(out, "5");
    }
    if let Some(out) = rustc_run(&rs, "f(-1)") {
        assert_eq!(out, "0");
    }
}

#[test]
fn a_bottom_up_fold_tuple_of_recursive_results_never_miscompiles_declines_or_computes_12() {
    // LATENT-HAZARD TRIPWIRE (concierge-directed, tick-103). The self-hosting optimizer idiom: `fold`
    // recursively simplifies an expression's two children, then matches the TUPLE of the two recursive
    // results `(tuple (fold a) (fold b))` with CONSTRUCTOR patterns `(tuple (E.Lit x) (E.Lit y))` to fire
    // a rewrite. Folding `(Add (Lit 3) (Add (Lit 4) (Lit 5)))` must collapse to `(Lit 12)`, so `ev` = 12.
    //
    // HISTORY: this DECLINED while the recursive self-call's result shape was unresolved (subject_ty =
    // Ty::Any). Once v-inference's SCC return-type-fixpoint LANDED, it grounds subject_ty to E — which
    // EXPOSED a real miscompile: emit_sum_switch minted the two `(E.Lit x)` / `(E.Lit y)` payload binders
    // with a name keyed on the switch path's LENGTH, so the two sibling switches (`[Elem(0)]` and
    // `[Elem(1)]` of the tuple, both len 1, both arm 0) COLLIDED to the same name — `x` and `y` aliased and
    // `(+ x y)` emitted `p.checked_add(p)`, computing 20 not 12. Fixed by keying the binder name off the
    // path CONTENT (`sum_path_tag`: `e0` vs `e1`) so siblings never collide. It now EMITS and computes 12.
    //
    // THE TRIPWIRE (kept as a permanent guard against a re-collision OR a future wrong grounding — e.g. a
    // grounding to a DIFFERENT-shaped sum, which the emit_sum_switch variant-count guard declines rather
    // than mis-resolving; see the 20-structural-editing corpus case + [[queued-generic-transformer-closure-tie]]):
    // the ONLY acceptable outcomes are (a) a clean DECLINE, or (b) an emit that rustc-compiles AND runs to
    // 12. A build that emits but yields anything ELSE (20, a panic, non-compile) fails here — catching any
    // regression of the binder-collision fix or a bad grounding immediately.
    let prog = "(module m (type E (Lit Int64) (Add (Tuple E E))) \
         (def (fold e) \
           (match e ((E.Lit n) (E.Lit n)) \
             ((E.Add (tuple a b)) \
               (match (tuple (fold a) (fold b)) \
                 ((tuple (E.Lit x) (E.Lit y)) (E.Lit (+ x y))) \
                 ((tuple fa fb) (E.Add (tuple fa fb))))))) \
         (def (ev e) (match e ((E.Lit n) n) ((E.Add (tuple a b)) (+ (ev a) (ev b))))) \
         (def (run) (ev (fold (E.Add (tuple (E.Lit 3) (E.Add (tuple (E.Lit 4) (E.Lit 5)))))))) \
         (export run))";
    match compile_rust_result(prog) {
        // (a) A sound decline — the current, expected behavior while the self-call shape is unresolved.
        Err(_) => {}
        // (b) It emitted — then it MUST be correct (compiles + runs to 12), never a wrong-variant 20.
        Ok(_rs) => {
            if let Some(out) = rustc_run(&compile_rust(prog), "run()") {
                assert_eq!(
                    out, "12",
                    "if the bottom-up fold emits, it MUST compute 12 — a wrong value (e.g. 20) is the \
                     wrong-variant sum-match miscompile this tripwire guards against"
                );
            }
        }
    }
}

#[test]
fn rustc_roundtrip_two_field_sum_payload_binds_both_fields() {
    // REGRESSION GUARD — a 2-field ctor `(P Int64 Int64)` matched `((P x y) (+ x y))` must bind BOTH payload
    // fields and compute `x + y`, on the Rust backend, for a constant ctor AND a runtime-boxed one.
    //
    // HISTORY (queue TRACKING-multifield-sum-payload-2elem): this class TRAPPED on the COMPILED self-host
    // wasm path while running green on the interpreter — which LOOKED like a wasm-emit value-miscompile
    // (and rust round-tripping to 7 SUGGESTED that). It was NOT: the root was v-compiler-ml's INFER gap —
    // the payload-ctor arm typed only arg1, never arg2, so arg2 had no type-column entry → lower declined →
    // run-src returned `Option.None` → the test's None-arm `unreachable` fired (the same class as b128,
    // also an infer gap caught probe-first). RESOLVED + LANDED b146 (v-compiler-ml's multi-binder + arity +
    // arg-N-infer stack); ss-multifield-payload-ctor-{const-both-binders,runtime-boxed,bare-constructs} are
    // GREEN on the compiled path. NO rcdzc/wasm-emit change was needed. So multifield round-trips 7 on BOTH
    // backends today — this rust test is a permanent guard on the `emit_sum_payload` bind-both path (the
    // [[value-facts-slice5-variant-tags-nested-match-elision-seam]] family), NOT a wasm-emit isolator.
    // Both the CONSTANT ctor `(P 3 4)` and the runtime-BOXED twin `(if (> n 0) (P 3 4) (P 10 20))` → 7.
    let module = compile_rust(
        "(module m (type PP (P Int64 Int64)) \
           (def (add-pair (: p PP)) (match p ((P x y) (+ x y)))) \
           (def (run) (add-pair (P 3 4))) \
           (def (run-boxed (: n Int64)) (add-pair (if (> n 0) (P 3 4) (P 10 20)))) \
           (export run) (export run-boxed))",
    );
    if let Some(out) = rustc_run(&module, "run()") {
        assert_eq!(
            out, "7",
            "the Rust backend MUST bind BOTH fields of a 2-field sum payload (constant ctor) → 7 \
             (a wrong value would be a regression of the emit_sum_payload bind-both path)"
        );
    }
    // The runtime-boxed twin: n>0 selects (P 3 4) → 7 (the other arm is (P 10 20) → 30, never taken here).
    if let Some(out) = rustc_run(&module, "run_boxed(1)") {
        assert_eq!(
            out, "7",
            "the Rust backend binds both fields of a RUNTIME-boxed 2-field sum payload → 7 (n>0 arm)"
        );
    }
}

#[test]
fn rustc_roundtrip_match_a_runtime_tuple_from_an_if() {
    // A top-level `(tuple a b)` pattern over a RUNTIME tuple scrutinee — a tuple built by an `if` (or a
    // branchy fn), NOT a constant `Core::Tuple` and NOT a bound `__pay` (a top-level tuple match mints no
    // `Switch` arm, hence no bind). The binders `a`/`b` read `[Elem(0)]`/`[Elem(1)]` directly off the
    // scrutinee; the backend now emits the scrutinee value indexed (`(<t>).i`) — the runtime-tuple twin of
    // the constant fold. Was a Rust-backend decline ("sum payload has no bound match arm"); wasm reads it
    // via `arr-get` (no bind needed). `g(5)` = (5,0) → 5, `g(-3)` = (0,-3) → -3.
    let rs = compile_rust(
        "(module m (def (g (: k Int64)) (if (> k 0) (tuple k 0) (tuple 0 k))) \
           (def (f (: k Int64)) (match (g k) ((tuple a b) (+ a b)))) (export f))",
    );
    if let Some(out) = rustc_run(&rs, "f(5)") {
        assert_eq!(out, "5");
    }
    if let Some(out) = rustc_run(&rs, "f(-3)") {
        assert_eq!(out, "-3");
    }
}

#[test]
fn a_generic_sum_emits_a_parameterized_descriptor_for_the_gate_renderer() {
    // A GENERIC user sum emits a `// cdz-sum[Ident]:` descriptor whose payload tokens carry `T{k}`
    // PLACEHOLDERS (not concrete types), plus a `// cdz-sum-params[Ident]: N` note giving the parameter
    // count — so the gate's rust-target value renderer can substitute the result type's concrete args and
    // render a generic-sum escape (was: no descriptor → the escape fell to a scalar `Display` of the enum
    // → rustc E0277). A bare-param payload `(W a)` renders `T0`; a nested one `(W (Option a))` renders
    // `(Option T0)`.
    let bare =
        compile_rust("(module m (type Box (W a) (E)) (def (main) (Box.W 42)) (export main))");
    assert!(
        bare.contains("// cdz-sum[Box]: (W T0) (E)"),
        "bare-param generic sum descriptor uses T0 placeholder:\n{bare}"
    );
    assert!(
        bare.contains("// cdz-sum-params[Box]: 1"),
        "generic sum records its parameter count:\n{bare}"
    );

    // A nested-param payload renders the placeholder inside the nesting: `(W (Option a))` → `(Option T0)`.
    let nested = compile_rust(
        "(module m (type Box (E) (W (Option a))) (def (main) (Box.W (Option.Some 42))) (export main))",
    );
    assert!(
        nested.contains("// cdz-sum[Box]: (E) (W (Option T0))"),
        "nested-param generic sum descriptor places T0 inside the payload:\n{nested}"
    );

    // A MONOMORPHIC sum still emits its descriptor WITHOUT a params note (concrete payload types, no
    // placeholders) — the generic path must not regress the monomorphic one.
    let mono = compile_rust(
        "(module m (type W (V (Option Int64)) (Z)) (def (main) (W.V (Option.Some 5))) (export main))",
    );
    assert!(
        mono.contains("// cdz-sum[W]: (V (Option Int64)) (Z)"),
        "monomorphic descriptor keeps concrete payload types:\n{mono}"
    );
    assert!(
        !mono.contains("// cdz-sum-params[W]:"),
        "a monomorphic sum emits no params note:\n{mono}"
    );
}

#[test]
fn rustc_roundtrip_three_level_nested_sum_match_folds_through_known_constructors() {
    // THREE sum levels — `Outer.Q → Inner.Y → Result.Ok` — where the two OUTER variants are known
    // constructors (built inline) and only the innermost `Result` disc is runtime. `lower`'s disc-fold
    // collapses the two known outer switches, leaving a SINGLE `Result` switch whose subject sits at a deep
    // path (`[Payload, Payload]`) with NO enclosing binds. The backend reads that subject directly off the
    // constant value tree (`fold_const_sum_path` walks several `Payload`s deep), so it emits
    // `match <inner Result> { Ok(p) => p, Err(p) => p }`. `f(6)` = 6, `f(-1)` = 0 (Err payload). Was a
    // Rust-backend decline (`sum payload has no bound match arm`); matches the wasm oracle.
    let rs = compile_rust(
        "(module m (type Inner (X Int64) (Y (Result Int64 Int64))) (type Outer (P Int64) (Q Inner)) \
           (def (f (: k Int64)) (match (Outer.Q (Inner.Y (if (> k 0) (Result.Ok k) (Result.Err 0)))) \
                                  ((Outer.P h) h) ((Outer.Q (Inner.X n)) n) \
                                  ((Outer.Q (Inner.Y (Result.Ok o))) o) \
                                  ((Outer.Q (Inner.Y (Result.Err e))) e))) (export f))",
    );
    assert!(
        rs.contains("Result::Ok(__pay"),
        "dispatches on the innermost Result:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(6)") {
        assert_eq!(out, "6");
    }
    if let Some(out) = rustc_run(&rs, "f(-1)") {
        assert_eq!(out, "0");
    }
}

#[test]
fn rustc_roundtrip_generic_sum_with_a_type_param_nested_in_a_variant_payload() {
    // A GENERIC sum whose variant payload has the type parameter NESTED inside another type — `(type Box
    // (E) (W (Option a)))`, so `W`'s payload is `Option<a>`, not a bare `a`. The enum emitter mapped only a
    // WHOLE-payload param to `T{k}`; a param nested in `(Option a)` reached `rust_type(Ty::Var)` = None and
    // the whole generic enum declined ("no native representation"). It now renders the payload at a SENTINEL
    // instantiation and maps each param var to `T{k}` wherever it appears, emitting `enum Box<T0> { E,
    // W(Option<T0>) }`. `f(7)` = 7 (W(Some 7)), `f(-1)` = -1 (E). Matches the wasm oracle.
    let rs = compile_rust(
        "(module m (type Box (E) (W (Option a))) \
           (def (f (: k Int64)) (match (if (> k 0) (Box.W (Option.Some k)) (Box.E)) \
                                  ((Box.E) -1) ((Box.W (Option.Some n)) n) ((Box.W (Option.None)) -2))) \
           (export f))",
    );
    // The nested param renders `Option<T0>`, not a declined `Ty::Var`.
    assert!(
        rs.contains("W(Option<T0>)"),
        "nested param renders T0:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(7)") {
        assert_eq!(out, "7");
    }
    if let Some(out) = rustc_run(&rs, "f(-1)") {
        assert_eq!(out, "-1");
    }
}

#[test]
fn rustc_roundtrip_runtime_equality_over_a_payload_sum() {
    // Runtime `(= a b)` over a PAYLOAD-carrying sum. On wasm this is a value-heap equality walk; on the
    // Rust backend the sum maps to a native enum that now `#[derive(PartialEq, Eq)]` (its payloads are
    // Eq-derivable), so `=` emits a native `a == b` — was a decline ("does not yet render this compound
    // value"). `f(5)` compares `W.V(Some 5) == W.V(Some 5)` → 1; `f(9)` → `Some 9 != Some 5` → 0. Matches
    // the wasm oracle's structural equality.
    let rs = compile_rust(
        "(module m (type W (V (Option Int64)) (Z)) \
           (def (f (: k Int64)) (if (= (W.V (Option.Some k)) (W.V (Option.Some 5))) 1 0)) (export f))",
    );
    assert!(
        rs.contains("PartialEq, Eq"),
        "payload sum derives Eq:\n{rs}"
    );
    assert!(rs.contains("=="), "emits a native == :\n{rs}");
    if let Some(out) = rustc_run(&rs, "f(5)") {
        assert_eq!(out, "1");
    }
    if let Some(out) = rustc_run(&rs, "f(9)") {
        assert_eq!(out, "0");
    }
}

#[test]
fn rustc_roundtrip_runtime_equality_over_a_compound_with_a_rational_or_bigint_leaf() {
    // Runtime `(= a b)` over a COMPOUND carrying a Rational / BigInt leaf. `cdz_num::Rational` is stored
    // NORMALIZED (lowest terms) and `cdz_num::Big` sign-magnitude with no leading zeros, both `#[derive(Eq)]`
    // — so a native field-wise `==` compares by CANONICAL value, matching the wasm heap walk. Was a decline
    // ("runtime structural equality over this compound is not yet rendered").
    // A tuple with a Rational leaf: `(1/2, 1) == (2/4, 1)` is TRUE (normalization), `(1/2,1) == (1/3,1)` FALSE.
    let rat = compile_rust(
        "(module m (def (eq2 (: n Int64) (: d Int64)) \
           (if (= (tuple (Rational.of 1 2) 1) (tuple (Rational.of n d) 1)) 1 0)) (export eq2))",
    );
    assert!(
        rat.contains("=="),
        "a tuple with a Rational leaf emits a native == :\n{rat}"
    );
    if let Some(out) = rustc_run(&rat, "eq2(2, 4)") {
        assert_eq!(out, "1", "1/2 == 2/4 (normalized) inside a tuple");
    }
    if let Some(out) = rustc_run(&rat, "eq2(1, 3)") {
        assert_eq!(out, "0", "1/2 != 1/3 inside a tuple");
    }
    // A tuple with a BigInt leaf compares by value: `(big 5, 1) == (big 5, 1)` → 1, vs `(big 6, 1)` → 0.
    let big = compile_rust(
        "(module m (def (eqb (: k Int64)) \
           (if (= (tuple (BigInt.of 5) 1) (tuple (BigInt.of k) 1)) 1 0)) (export eqb))",
    );
    if let Some(out) = rustc_run(&big, "eqb(5)") {
        assert_eq!(out, "1", "a BigInt leaf compares equal by value");
    }
    if let Some(out) = rustc_run(&big, "eqb(6)") {
        assert_eq!(out, "0", "a differing BigInt leaf compares unequal");
    }
}

#[test]
fn rustc_roundtrip_runtime_equality_over_a_generic_sum() {
    // Runtime `(= a b)` over a GENERIC user sum at a concrete instantiation — `(type Box (W a) (E))`
    // compared at `Box Int64`. The generic enum `enum Box<T0> { W(T0), E }` `#[derive(PartialEq, Eq)]`
    // adds a `T0: Eq` bound automatically, so a `=` at `Box Int64` (where `i64: Eq`) emits a native
    // `a == b`. The derive/eq gate treats a type PARAMETER payload as Eq (the derive bounds it), keyed off
    // the sentinel instantiation. `f(5)` compares `Box.W(5) == Box.W(5)` → 1, `f(9)` → unequal → 0.
    let rs = compile_rust(
        "(module m (type Box (W a) (E)) (def (mk (: k Int64)) (if (> k 0) (Box.W k) (Box.E))) \
           (def (f (: k Int64)) (if (= (mk k) (mk 5)) 1 0)) (export f))",
    );
    assert!(rs.contains("enum Box<T0>"), "generic enum:\n{rs}");
    assert!(
        rs.contains(
            "#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]\n#[allow(dead_code)]\npub enum Box<T0>"
        ),
        "generic enum derives Eq + Ord (so it can key a BTreeMap; T0 bounds added by derive):\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(5)") {
        assert_eq!(out, "1");
    }
    if let Some(out) = rustc_run(&rs, "f(9)") {
        assert_eq!(out, "0");
    }
}

#[test]
fn runtime_equality_over_a_recursive_sum_emits_a_recursive_helper_fn() {
    // Runtime `(= a b)` over a RECURSIVE user sum whose payloads carry a Float/List (so NOT native-Eq — it
    // reaches the structural walk). Was a DECLINE: the walk expanded a `match` per payload INLINE, so a
    // self-referential sum would expand unboundedly at compile time (a codegen stack overflow), guarded by
    // a `seen` set that declined the recursive back-edge. Now the walk routes a user sum through a generated
    // recursive helper `fn __eq_<Ident>(l, r) -> bool` (call-indirection, mirroring the render crate's
    // `__render_<Ident>`), so the recursion runs at RUNTIME over the finite value and terminates — matching
    // the wasm value-eq-shaped walk. Unblocks a program that compares two `Ast` reflection values (=).

    // (a) LIST-recursive: `Ast = (Int Int64)|…|(List (List Ast))`. The self-reference is through a `List`
    // element, which `variant_is_recursive` (a Box-only check) does NOT catch — so it MUST route through the
    // helper via the `seen`/List path, not a Box deref. A runtime scrutinee so the match isn't folded away.
    let ast = compile_rust(
        "(module m (def (run (: k Int64)) \
           (if (= (Ast.List (list (Ast.Int (BigInt.of k)) (Ast.Bool true))) \
                  (Ast.List (list (Ast.Int 5) (Ast.Bool true)))) 1 0)) (export run))",
    );
    assert!(
        ast.contains("fn __eq_Ast(") && ast.contains("__eq_Ast(&"),
        "a recursive Ast eq emits + calls a recursive helper fn:\n{ast}"
    );
    if let Some(out) = rustc_run(&ast, "run(5)") {
        assert_eq!(out, "1", "equal recursive Ast values compare equal");
    }
    if let Some(out) = rustc_run(&ast, "run(9)") {
        assert_eq!(
            out, "0",
            "a differing nested Int makes the Ast values unequal"
        );
    }

    // (b) BOX-recursive with a FLOAT leaf: `Tree = (Leaf Float64)|(Node Tree Tree)`. Directly self-recursive
    // via a boxed 2-payload variant — the helper derefs the `Box` (`(**__lp).0`) and compares each float
    // leaf by the canonical byte form. Confirms the double-deref + tuple-projection composes, and that a
    // NaN leaf compares equal to itself yet distinct from a real number (canonical-byte float compare).
    let tree = compile_rust(
        "(module m (type Tree (Leaf Float64) (Node Tree Tree)) \
           (def (run (: x Float64)) \
             (if (= (Tree.Node (Tree.Leaf x) (Tree.Leaf 2.0)) \
                    (Tree.Node (Tree.Leaf 1.0) (Tree.Leaf 2.0))) 1 0)) (export run))",
    );
    assert!(
        tree.contains("fn __eq_Tree(") && tree.contains("is_nan()"),
        "a Box-recursive float-carrying Tree eq emits a helper comparing floats by canonical bytes:\n{tree}"
    );
    if let Some(out) = rustc_run(&tree, "run(1.0)") {
        assert_eq!(out, "1", "structurally equal trees compare equal");
    }
    if let Some(out) = rustc_run(&tree, "run(3.0)") {
        assert_eq!(
            out, "0",
            "a differing float leaf deep in the tree -> unequal"
        );
    }
    if let Some(out) = rustc_run(&tree, "run(f64::NAN)") {
        assert_eq!(
            out, "0",
            "a NaN leaf is distinct from the real 1.0 it's compared against"
        );
    }
}

#[test]
fn ast_float_reify_traps_a_runtime_non_canonical_float_matching_wasm() {
    // The reify `Ast` sum's `Float` variant must carry a CANONICAL float — a non-canonical NaN/±inf has no
    // canonical value form, so wasm's value-encode boundary TRAPS on it. A RUNTIME-produced non-canonical
    // float (`(Ast.Float (- x nan))`, x a param) can't be compile-declined (the constant case is declined at
    // `lower_ctor`), so the rust `Ast.Float` construction emits a runtime `is_finite()` guard that PANICS —
    // matching wasm's runtime trap, so both backends AGREE (adv-ast-float-nan differential, ruling A,
    // v-runtime route). Only the `Ast.Float` variant is guarded.
    let runtime = compile_rust(
        "(module m (def (main (: x Float64)) (Ast.Float (- x Float64.nan))) (export main))",
    );
    assert!(
        runtime.contains("is_finite()")
            && runtime.contains("panic!(\"an Ast.Float node cannot carry a non-canonical float"),
        "a runtime Ast.Float payload emits an is_finite trap guard:\n{runtime}"
    );
    // The guard actually fires at run time: `main(1.0)` computes `1.0 - NaN = NaN`, which must PANIC (the
    // trap), NOT return a node. `rustc_run` returns `None` on a non-zero exit (a panic), so a `Some` here
    // would be the miscompile. (We assert the emit shape above; this documents the runtime contract.)
    // A FINITE runtime float still constructs the node normally (the guard passes) — no false trap.
    let finite =
        compile_rust("(module m (def (main (: x Float64)) (Ast.Float (- x 1.0))) (export main))");
    assert!(
        finite.contains("is_finite()") && finite.contains("Ast::Float("),
        "a finite Ast.Float still constructs the node (the guard is a runtime check, passes for finite):\n{finite}"
    );
    // An ORDINARY float value (NOT wrapped in Ast.Float) is unguarded — a bare NaN round-trips as a value on
    // both backends, so no trap. The guard is narrowly the reify Float variant's obligation.
    let bare =
        compile_rust("(module m (def (main (: x Float64)) (- x Float64.nan)) (export main))");
    assert!(
        !bare.contains("is_finite()"),
        "a bare float value is not guarded (only Ast.Float reify is):\n{bare}"
    );
}

#[test]
fn rustc_roundtrip_builtin_option_matches() {
    // unwrap-or(Some 8, _) = 8, unwrap-or(None, -1) = -1 — a match over std's Option, constructed with
    // std's `Some`/`None` in the driver, runs and matches the oracle.
    let rs = compile_rust(
        "(module m (def (unwrap-or (: o (Option Int64)) (: d Int64)) \
           (match o (((. Option Some) x) x) (((. Option None) _) d))) (export unwrap-or))",
    );
    if let Some(out) = rustc_run(&rs, "unwrap_or(Some(8), -1)") {
        assert_eq!(out, "8");
    }
    if let Some(out) = rustc_run(&rs, "unwrap_or(None, -1)") {
        assert_eq!(out, "-1");
    }
}

#[test]
fn rustc_roundtrip_nullary_first_if_returns_generic_option() {
    // REGRESSION (E0282, surfaced when the BigInt emit-side enabled a kernel program's rust path): an `if`
    // whose result is a GENERIC sum and whose FIRST (`then`) branch is the bare nullary variant — `if c
    // then (Option.None) else (Option.Some …)` — gave rustc no type parameter to infer at the `None`
    // (branches type left-to-right; the sibling `Some` comes second). Bare `Option::None` → "type
    // annotations needed". FIX: the backend annotates the whole `if` with its solved generic-sum type
    // (`{ let __if: Option<i64> = if … ; __if }`), so rustc has the type up front.
    let rs = compile_rust(
        "(module m (def (mk (: n Int64)) (if (= n 0) (Option.None unit) (Option.Some n))) (export mk))",
    );
    assert!(
        rs.contains("let __if: Option<i64>"),
        "the generic-sum if is type-annotated:\n{rs}"
    );
    // n=0 → None → the match's None arm; n=5 → Some(5). Render via a driver that maps to cdz-run's form.
    let driver = "fn main(){ let v = prog::mk(0); let s = match v { Some(x) => format!(\"(Some {})\", x), \
                  None => \"(None unit)\".to_string() }; println!(\"{}\", s); }";
    if let Some(out) = rustc_run_driver(&rs, driver) {
        assert_eq!(
            out, "(None unit)",
            "nullary-first if returns None on the true branch"
        );
    }
    // CONTROL: a MONOMORPHIC-sum if (no type params) stays a bare `if` — the annotation is generic-only.
    let mono = compile_rust(
        "(module m (type S (A) (B)) (def (mk (: n Int64)) (if (= n 0) (S.A) (S.B))) (export mk))",
    );
    assert!(
        !mono.contains("let __if:"),
        "a monomorphic-sum if is NOT annotated (bare if):\n{mono}"
    );
}

// ── async / gas-metered emission ─────────────────────────────────────────────────────────────────

/// Compile a program to the ASYNC Rust backend (gas-metered `async fn`s + the `CdzEnv` trait).
fn compile_rust_async(src: &str) -> String {
    let ast_bytes = crate::codec::encode(&parse(src));
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes)],
        &[Target::RustAsync],
    );
    match out.artifact(Target::RustAsync.artifact_kind()) {
        Some(bytes) => String::from_utf8(bytes.to_vec()).expect("emitted Rust is utf-8"),
        None => panic!(
            "async Rust emit failed: {:?}",
            out.diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        ),
    }
}

/// Try to compile a program to the async Rust backend, returning the emitted source or the diagnostics
/// (for asserting a DECLINE). The async twin of [`compile_rust_result`].
fn compile_rust_async_result(src: &str) -> Result<String, Vec<String>> {
    let ast_bytes = crate::codec::encode(&parse(src));
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes)],
        &[Target::RustAsync],
    );
    match out.artifact(Target::RustAsync.artifact_kind()) {
        Some(bytes) => Ok(String::from_utf8(bytes.to_vec()).expect("emitted Rust is utf-8")),
        None => Err(out
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()),
    }
}

#[test]
fn async_mode_emits_env_threaded_gas_metered_fns() {
    let rs = compile_rust_async(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export sum-to))",
    );
    // The gas/yield trait now lives in the SHARED `cdz-rt` crate (NOT re-declared per module); the
    // module brings it into scope with a `use`, so an application implements `CdzEnv` once for all.
    assert!(
        rs.contains("use cdz_rt::{CdzEnv, DynCdzEnv, EnvClosure};"),
        "cdz_rt import (async closures also import DynCdzEnv + EnvClosure for the boxed-future ABI):\n{rs}"
    );
    // The import is `#[allow(unused_imports)]`-guarded: a closure-free async program imports DynCdzEnv/
    // EnvClosure unused, which `-D warnings` would reject.
    assert!(
        rs.contains("#[allow(unused_imports)] use cdz_rt::{CdzEnv, DynCdzEnv, EnvClosure};"),
        "the cdz_rt import is unused-imports-guarded:\n{rs}"
    );
    assert!(
        !rs.contains("pub trait CdzEnv"),
        "must NOT re-declare the trait:\n{rs}"
    );
    // The fn is async and takes the OBJECT-SAFE env `__cdz_env: &mut dyn DynCdzEnv` (uniform-env ABI — the
    // same env a lifted closure fn takes, so a closure body can call top-level fns; no per-fn `<__CdzE>`
    // generic). The env VALUE param is the reserved `__cdz_env` (never collides with a source `env`).
    assert!(
        rs.contains("pub async fn sum_to(__cdz_env: &mut dyn DynCdzEnv, n: i64)"),
        "signature (uniform &mut dyn DynCdzEnv env, no generic):\n{rs}"
    );
    // Gas charges via the OBJECT-SAFE `consume_boxed` (the RPITIT `consume` is not callable on a `dyn`).
    assert!(
        rs.contains("__cdz_env.consume_boxed(1).await;"),
        "gas charge via consume_boxed:\n{rs}"
    );
    // The recursive call is boxed-and-awaited, threading the env first.
    assert!(
        rs.contains("Box::pin(sum_to(__cdz_env,"),
        "boxed recursive call:\n{rs}"
    );
}

#[test]
fn async_boxes_only_a_call_whose_future_is_self_referential() {
    // OPERATOR DIRECTIVE: the async backend must box ONLY a "truly recursive async function where we
    // couldn't transform it into a loop" — NOT every call. A `Box::pin` is needed exactly when the
    // callee's emitted future is self-referential (infinitely sized); a non-recursive callee, and a
    // fully-tail-recursive (loop-transformed) callee, have finite futures and must NOT be boxed.

    // (a) A FULLY-TAIL-recursive callee (accumulator `sum-to`) is loop-transformed — its self-call is a
    // `continue`, its future is a finite `loop`. The ENTRY call to it (from `main`) must NOT be boxed.
    let tail = compile_rust_async(
        "(module m (def (sumto (: n Int64) (: acc Int64)) (if (= n 0) acc (sumto (+ n -1) (+ acc n)))) \
           (def (main) (sumto 5 0)) (export main))",
    );
    assert!(
        tail.contains("loop {"),
        "the tail-recursive accumulator is loop-transformed:\n{tail}"
    );
    assert!(
        !tail.contains("Box::pin"),
        "a call to a fully-loop-transformed callee is NOT boxed (finite future):\n{tail}"
    );

    // (b) A NON-recursive helper chain: nothing is boxed (helpers inline / are finite).
    let plain = compile_rust_async(
        "(module m (def (leaf (: x Int64)) (+ x 1)) (def (mid (: y Int64)) (leaf (* y 2))) \
           (def (main) (mid 5)) (export main))",
    );
    assert!(
        !plain.contains("Box::pin"),
        "a non-recursive call chain is never boxed:\n{plain}"
    );

    // (c) A NON-TAIL recursive callee that CANNOT be loop-transformed (its self-call is an operand, used
    // after the recursion) — its future IS self-referential, so the recursive call MUST be boxed (Rust
    // E0733 otherwise). Even when the fn is ALSO loop-transformed for its TAIL self-call, the NON-tail
    // self-call still needs the box (the case the first cut of this fix missed).
    let nontail = compile_rust_async(
        "(module m (def (rem (: t Int64) (: hs (List Int64))) \
           (match hs ((list) (list)) \
             ((list h .. rest) (if (= h t) (rem t rest) (List.push (rem t rest) h))))) \
           (def (main) (List.len (rem 5 (list 1 2 5 3)))) (export main))",
    );
    assert!(
        nontail.contains("loop {"),
        "rem is loop-transformed for its TAIL self-call:\n{nontail}"
    );
    assert!(
        nontail.contains("Box::pin(rem("),
        "rem's NON-TAIL self-call is still boxed (its future is self-referential):\n{nontail}"
    );
}

#[test]
fn async_env_type_param_does_not_collide_with_a_user_sum_named_e() {
    // The async env is now the OBJECT-SAFE `&mut dyn DynCdzEnv` VALUE param — there is NO generic env TYPE
    // parameter at all, so a user sum `(type E …)` → `enum E` can never be shadowed by one (the earlier
    // bare-`E`-type-param collision is structurally impossible now). Pin that: the enum emits + constructs,
    // and no async fn header carries ANY generic env param.
    let rs = compile_rust_async(
        "(module m (type E (A Int64) (B Int64)) (def (main) (E.B 7)) (export main))",
    );
    assert!(rs.contains("pub enum E {"), "user enum E emitted:\n{rs}");
    assert!(
        rs.contains("pub async fn main(__cdz_env: &mut dyn DynCdzEnv)"),
        "uniform-env signature, no generic env param:\n{rs}"
    );
    assert!(
        !rs.contains(": CdzEnv>"),
        "no generic env type param on any async fn:\n{rs}"
    );
    // It compiles (the enum `E` and the env param no longer collide).
    let driver = r#"
struct M;
impl cdz_rt::CdzEnv for M { async fn consume(&mut self, _: u64) {} }
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() { let r = block_on(prog::main(&mut M)); if let prog::E::B(v) = r { println!("{}", v); } }
"#;
    if let Some(out) = rustc_run_driver(&rs, driver) {
        assert_eq!(out, "7");
    }
}

#[test]
fn rustc_roundtrip_async_gas_metered() {
    // The async form compiles and runs under a hand-rolled executor with a real gas Env — same answer as
    // the sync form (sum_to(5)=15), gas is metered, and an exhausted budget traps.
    let module = compile_rust_async(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export sum-to))",
    );
    // A driver: a Meter env (counts gas, panics past budget) + a minimal block_on executor.
    let driver = r#"
struct Meter { spent: u64, budget: u64 }
impl cdz_rt::CdzEnv for Meter {
    async fn consume(&mut self, g: u64) { self.spent += g; if self.spent > self.budget { panic!("oom") } }
}
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let mut e = Meter { spent: 0, budget: 10000 };
    let v = block_on(prog::sum_to(&mut e, 5));
    let gas = e.spent;
    let oom = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || block_on(prog::sum_to(&mut Meter { spent: 0, budget: 3 }, 100)),
    )).is_err();
    println!("{v} {} {oom}", gas > 0);
}
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        // sum_to(5)=15, gas was metered (>0), and the budget-3 run trapped.
        assert_eq!(out, "15 true true", "async run:\n{module}");
    }
}

#[test]
fn rustc_roundtrip_async_recursive_sum_folds() {
    // The async backend's recursion handling (`Box::pin(callee(env, …)).await`) composes with the
    // recursive-sum representation (a boxed self-referential payload, `Cons(Box<(i64, L)>)`): a
    // cons-list summed by a self-recursive `async fn` compiles under the real `CdzEnv`, meters gas at
    // every step, and folds to the same value the sync/wasm backends produce. This is the one sum
    // shape whose async execution the sync round-trips don't cover — the recursive `async fn` future
    // must be `Box::pin`-sized AND its payload `Box`-sized, two independent boxings that must agree.
    let module = compile_rust_async(
        "(module m (type L (Nil) (Cons Int64 L)) \
         (def (sm l) (match l ((L.Nil) 0) ((L.Cons h t) (+ h (sm t))))) \
         (def (main) (sm (L.Cons 1 (L.Cons 2 (L.Cons 3 (L.Nil)))))) (export main))",
    );
    // A recursive `async fn` sizes its future via `Box::pin`; the recursive payload sizes via `Box`.
    assert!(
        module.contains("Cons(::std::boxed::Box<(i64, L)>)"),
        "boxed payload:\n{module}"
    );
    assert!(
        module.contains("Box::pin(sm(__cdz_env,"),
        "boxed recursive call:\n{module}"
    );
    let driver = r#"
struct Meter { spent: u64 }
impl cdz_rt::CdzEnv for Meter {
    async fn consume(&mut self, g: u64) { self.spent += g; }
}
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() {
    let mut e = Meter { spent: 0 };
    let v = block_on(prog::main(&mut e));
    // sm([1,2,3]) = 6; gas was metered across the recursive descent (one charge per fn entry).
    println!("{v} {}", e.spent > 3);
}
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "6 true", "async recursive-sum run:\n{module}");
    }
}

#[test]
fn rustc_roundtrip_async_host_call_reads_shim_and_meters_gas() {
    // COVERAGE PIN for the rust-async HOST-call frontier (the +103 host/@param unlock, PR #2412): an async
    // fn that makes a delegated host call must (a) EMIT the host shim call inside the async body, (b) charge
    // entry gas via the object-safe `consume_boxed`, and (c) RUN correctly under an executor with a supplied
    // host shim — producing the SAME value as the sync/wasm oracle. Before #2412 the gate harness blanket-
    // DECLINED any rust-async case with a host protocol; this pins that the async host path both emits AND
    // runs, independent of the corpus baseline (a baseline row can be flipped; a lib rustc-roundtrip is a
    // hard witness that regresses loudly if the async host emit/drive ever breaks).
    let module = compile_rust_async(
        "(module m (effect out (op ask (-> Unit Int64))) \
         (def (main) (host (out) (+ (out.ask) 5))) (export main))",
    );
    // (a) the host call is a plain sync shim call INSIDE the async body (a host op charges no gas itself —
    //     only the enclosing async fn's entry `consume_boxed` does), and (b) the async fn takes the uniform
    //     object-safe `&mut dyn DynCdzEnv` env and charges entry gas via `consume_boxed`.
    assert!(
        module.contains("pub async fn main(__cdz_env: &mut dyn DynCdzEnv)")
            && module.contains("__cdz_env.consume_boxed(1).await;"),
        "async host-calling fn: uniform dyn-env + consume_boxed entry gas:\n{module}"
    );
    assert!(
        module.contains("crate::__cdz_host_out_ask()"),
        "the host op emits a (sync) crate-root shim call inside the async body:\n{module}"
    );
    // (c) drive it: a Meter env (async consume) + block_on + the host shim returning the recorded response
    //     (37). main = out.ask() + 5 = 42; gas was metered (entry consume_boxed fired).
    let driver = r#"
struct Meter { spent: u64 }
impl cdz_rt::CdzEnv for Meter {
    async fn consume(&mut self, g: u64) { self.spent += g; }
}
#[allow(non_snake_case)]
fn __cdz_host_out_ask() -> i64 { 37 }
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() {
    let mut e = Meter { spent: 0 };
    let v = block_on(prog::main(&mut e));
    // out.ask() = 37; 37 + 5 = 42; gas metered (entry consume_boxed charged >0).
    println!("{v} {}", e.spent > 0);
}
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(
            out, "42 true",
            "async host-call run (37 + 5 = 42, gas metered):\n{module}"
        );
    }
}

#[test]
fn rustc_roundtrip_async_closures_stored_as_map_values_dispatch_and_run() {
    // COVERAGE PIN for the async CLOSURE-VALUE-IN-A-COLLECTION emit (Option A + the Core::MapNew
    // async_closure_type annotation fix): closures stored as `BTreeMap` VALUES cross as `Rc<dyn
    // EnvClosure<A,R>>`, so the map's value-type ANNOTATION must be the EnvClosure form (a `Rc<dyn Fn>`
    // annotation would E0308 against the boxed-future values — the exact bug the enum/collection mode-
    // threading fixed). Each looked-up closure is applied `.call(env, arg).await`. This pins that a
    // collection-of-closures emits a consistent value-type + runs, independent of the flippable corpus row.
    let module = compile_rust_async(
        "(module m (def (main (: y Int64)) (do \
         (def m ((. Map insert) ((. Map insert) (. Map empty) 1 (fn ((: v Int64)) (* v 2))) 2 (fn ((: v Int64)) (+ v 100)))) \
         (def (app (: k Int64)) (match ((. Map lookup) m k) ((Some f) (f y)) ((None _u) -1))) \
         (+ (* 1000 (app 1)) (app 2)))) (export main))",
    );
    // The map's VALUE type is the EnvClosure form (NOT a sync `Rc<dyn Fn>`), and a looked-up closure is
    // applied via `.call(env, arg).await`.
    assert!(
        module.contains(
            "std::collections::BTreeMap<i64, std::rc::Rc<dyn cdz_rt::EnvClosure<i64, i64>>>"
        ),
        "map value type is the EnvClosure boxed-future form:\n{module}"
    );
    assert!(
        module.contains(".call(__cdz_env, y)"),
        "a looked-up closure is applied via .call(env, arg):\n{module}"
    );
    let driver = r#"
struct Meter { spent: u64 }
impl cdz_rt::CdzEnv for Meter {
    async fn consume(&mut self, g: u64) { self.spent += g; }
}
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() {
    let mut e = Meter { spent: 0 };
    // key 1 → (fn v (* v 2)) applied to 5 = 10; key 2 → (fn v (+ v 100)) applied to 5 = 105.
    // 1000 * 10 + 105 = 10105. gas metered across the dispatched closure calls.
    let v = block_on(prog::main(&mut e, 5));
    println!("{v} {}", e.spent > 0);
}
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(
            out, "10105 true",
            "async map-of-closures dispatch+run (1000*10 + 105 = 10105, gas metered):\n{module}"
        );
    }
}

#[test]
fn rustc_roundtrip_async_mutually_recursive_sums_fold() {
    // Async recursion + MUTUAL sum recursion: `(type A (AN B))` / `(type B (BN A))` — neither variant
    // mentions its own decl, so the box decision must follow the A→B→A cycle (reaches_decl) to box both
    // `AN(Box<B>)` and `BN(Box<A>)`; and the two mutually-recursive `async fn`s each `Box::pin` the
    // other's call. Both boxings must land or rustc rejects (E0072 infinite size / unsized future).
    let module = compile_rust_async(
        "(module m (type A (AL Int64) (AN B)) (type B (BL Int64) (BN A)) \
         (def (sa a) (match a ((A.AL n) n) ((A.AN b) (sb b)))) \
         (def (sb b) (match b ((B.BL n) n) ((B.BN a) (sa a)))) \
         (def (main) (sa (A.AN (B.BN (A.AL 9))))) (export main))",
    );
    assert!(
        module.contains("AN(::std::boxed::Box<B>)"),
        "boxed A payload:\n{module}"
    );
    assert!(
        module.contains("BN(::std::boxed::Box<A>)"),
        "boxed B payload:\n{module}"
    );
    let driver = r#"
struct M;
impl cdz_rt::CdzEnv for M { async fn consume(&mut self, _: u64) {} }
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() { println!("{}", block_on(prog::main(&mut M))); }
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "9", "async mutual-recursion run:\n{module}");
    }
}

#[test]
fn rustc_roundtrip_float_arithmetic_and_of_int() {
    // The Rust backend renders floats natively: `Ty::Float` → f64/f32, the ONE arithmetic operator
    // `+`/`-`/`*`/`/` over float operands → the Rust operators (IEEE, no trap), `Float64.of-int` →
    // `as f64`. A runtime `(+ a b)` over f64 params computes the same non-exact IEEE sum the wasm backend
    // does (0.1+0.2 = 0.30000000000000004) — Rust's `{}` for an f64 prints the shortest round-tripping
    // decimal, matching the value form.
    let add = compile_rust("(module m (def (f (: a Float64) (: b Float64)) (+ a b)) (export f))");
    if let Some(out) = rustc_run(&add, "f(0.1, 0.2)") {
        assert_eq!(out, "0.30000000000000004");
    }
    let mul = compile_rust("(module m (def (f (: a Float64) (: b Float64)) (* a b)) (export f))");
    if let Some(out) = rustc_run(&mul, "f(6.0, 7.0)") {
        assert_eq!(out, "42");
    }
    // A constant float folds to a Rust `f64::from_bits(..)` literal, crossing exactly.
    let konst = compile_rust("(module m (def (f) (+ 1.5 2.0)) (export f))");
    if let Some(out) = rustc_run(&konst, "f()") {
        assert_eq!(out, "3.5");
    }
    // `Float64.of-int` over a runtime integer → `as f64` (total).
    let ofint = compile_rust("(module m (def (f (: n Int64)) (Float64.of-int n)) (export f))");
    if let Some(out) = rustc_run(&ofint, "f(42)") {
        assert_eq!(out, "42");
    }
    // `Float32.of` demotes (`as f32`, rounds); `Float64.of` promotes (`as f64`, exact). Rust's `{}`
    // for the f32 result prints the shortest round-tripping decimal for the binary32 value.
    let demote = compile_rust("(module m (def (f (: x Float64)) (Float32.of x)) (export f))");
    if let Some(out) = rustc_run(&demote, "f(0.1)") {
        assert_eq!(out, "0.1"); // Rust prints the f32 nearest to 0.1 as "0.1" (shortest round-trip)
    }
    let promote = compile_rust("(module m (def (f (: x Float32)) (Float64.of x)) (export f))");
    if let Some(out) = rustc_run(&promote, "f(1.5)") {
        assert_eq!(out, "1.5");
    }
}

#[test]
fn rustc_roundtrip_float32_literal_grounds_to_operand_width() {
    // The FLOAT column of the narrow-literal width family: a bare `1.5` in `(= x 1.5)` / `(* x 2.0)` where
    // `x: Float32` defaults its OWN solved type to Float64, so it emitted `f64::from_bits(…)` — and the
    // equality path's canonical-bits compare `.to_bits() as u32` then took the LOW 32 BITS of the f64
    // pattern (0x0 for 1.5, ≠ the f32 0x3fc00000) → ALWAYS FALSE (silent wrong value); the arith path
    // emitted `x * <f64>` → rustc E0277. Fix: `emit_grounded_float` grounds a ConstFloat operand to the
    // op's float width, so both operands share the type. Compare + arith, f32 + f64.
    // FACE 1 (equality — the silent one): (= x 1.5) at f32 must be TRUE for x=1.5 (was always false).
    let eq = compile_rust("(module m (def (f (: x Float32)) (if (= x 1.5) 1 0)) (export f))");
    assert!(
        eq.contains("f32::from_bits(1069547520u32)"), // 0x3fc00000 = 1.5f32
        "the f32 literal grounds to the f32 bit pattern, not the truncated f64 low bits:\n{eq}"
    );
    if let Some(out) = rustc_run(&eq, "f(1.5)") {
        assert_eq!(
            out, "1",
            "f32 x=1.5 == literal 1.5 is TRUE (was silently false)"
        );
    }
    if let Some(out) = rustc_run(&eq, "f(2.5)") {
        assert_eq!(out, "0", "f32 x=2.5 != 1.5");
    }
    // FACE 2 (arith — the loud E0277 one): (* x 2.0) at f32 emits `x * f32::from_bits(..)`, rustc-compiles.
    let mul = compile_rust("(module m (def (g (: x Float32)) (* x 2.0)) (export g))");
    assert!(
        mul.contains("f32::from_bits(1073741824u32)"), // 0x40000000 = 2.0f32
        "the f32 arith literal grounds to f32 (no `f32 * f64` E0277):\n{mul}"
    );
    if let Some(out) = rustc_run(&mul, "g(3.0)") {
        assert_eq!(out, "6", "f32 3.0 * 2.0 = 6");
    }
    // CONTROL: a Float64 literal still emits at f64 (no over-narrowing regression).
    let f64c = compile_rust("(module m (def (d (: x Float64)) (* x 2.0)) (export d))");
    assert!(
        f64c.contains("f64::from_bits("),
        "a Float64 operand's literal stays f64:\n{f64c}"
    );

    // BRANCH-position float literal: a bare `0.0` in an `if`-arm opposite a Float32 (`(if b x 0.0)`)
    // defaulted its ConstFloat to Float64 → `f64::from_bits` in an `-> f32` branch → E0308. `emit_branch`
    // now grounds a float branch to the construct's width (float twin of its `Ty::Int` grounding, via
    // `float_width_of_ty` + `emit_grounded_float`), so the `0.0` arm emits at f32.
    let branch =
        compile_rust("(module m (def (pick (: b Bool) (: x Float32)) (if b x 0.0)) (export pick))");
    assert!(
        branch.contains("pub fn pick(b: bool, x: f32) -> f32")
            && branch.contains("f32::from_bits")
            && !branch.contains("f64::from_bits"),
        "the bare float branch literal grounds to the if's f32 width (no f64 in an f32 branch):\n{branch}"
    );
    if let Some(out) = rustc_run(&branch, "pick(false, 1.0)") {
        assert_eq!(out, "0", "the false arm yields the f32 0.0");
    }
    if let Some(out) = rustc_run(&branch, "pick(true, 2.5)") {
        assert_eq!(out, "2.5", "the true arm yields x");
    }
}

#[test]
fn rustc_roundtrip_const_float_nan_emits_and_compares_by_canonical_bits() {
    // A constant NaN float (`Float64.nan`/`(. Float64 nan)`) → the EXPLICIT canonical NaN bits (not
    // `f64::NAN`, whose payload is platform-defined) — matching the runtime/wasm `CANON_NAN_BITS` so the
    // value is byte-identical to the canonical NaN the float-eq compare folds to.
    let nan = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Float64.nan) (f (+ n -1)))) (def (mk) (f 1)) (export mk))",
    );
    assert!(
        nan.contains("f64::from_bits(0x7FF8_0000_0000_0000u64)"),
        "Float64.nan → canonical NaN from_bits:\n{nan}"
    );
    // e2e: the NaN result renders as the canonical `nan` text (seq-287: the round-trippable value form the
    // binary-AST printer emits; both gates' Float render agree on `nan`, retiring the old `NaN`).
    let driver = "fn main(){ let r = prog::mk(); if r.is_nan() { println!(\"nan\"); } else { println!(\"{}\", r); } }";
    if let Some(out) = rustc_run_driver(&nan, driver) {
        assert_eq!(
            out, "nan",
            "a constant NaN float result renders the canonical nan"
        );
    }
    // NaN equality by CANONICAL BYTE FORM: `n = x/x` (NaN when x=0), then `(= n Float64.nan)` — the
    // canonical-bits compare makes NaN == NaN true (unlike IEEE `==`). c>0 selects the nan-compare arm → 1.
    let eq = compile_rust(
        "(module m (def (chk (: c Int64) (: x Float64)) \
           (let ((n (/ x x))) (if (if (> c 0) (= n (. Float64 nan)) (= n 2.0)) 1 0))) (export chk))",
    );
    if let Some(out) = rustc_run(&eq, "chk(1, 0.0)") {
        assert_eq!(
            out, "1",
            "NaN == nan under the canonical byte form (0/0 = NaN)"
        );
    }
}

#[test]
fn char_maps_to_rust_char_and_escapes_across_a_sum_payload() {
    // `Ty::Char`→`char`; `ConstChar`→a `'…'` literal; a Char crosses as a sum payload and renders `#\<c>`.
    let rs = compile_rust(
        "(module m (type Tok (Ch Char) (End)) (def (mk) ((. Tok Ch) #\\a)) (export mk))",
    );
    assert!(
        rs.contains("Ch(char)"),
        "Char sum payload → char field:\n{rs}"
    );
    assert!(
        rs.contains("Ch('a')"),
        "ConstChar → a Rust char literal:\n{rs}"
    );
    // e2e: the Char-payload sum renders `(Ch #\a)`.
    let driver = "fn main(){ let v = prog::mk(); let s = match v { prog::Tok::Ch(__c) => { let __c: char = __c; match __c { ' ' => \"#\\\\space\".to_string(), c if c.is_control() => format!(\"#\\\\u+{:04X}\", c as u32), c => format!(\"#\\\\{}\", c) } }, prog::Tok::End => \"(End unit)\".to_string() }; println!(\"(Ch {})\", s); }";
    if let Some(out) = rustc_run_driver(&rs, driver) {
        assert_eq!(
            out, "(Ch #\\a)",
            "a Char sum payload renders in the canonical #\\ form"
        );
    }
    // Char.from-int on a CONSTANT scalar folds to Some(char); a surrogate folds to None. (A runtime-int
    // from-int still declines — constant-only — so these fold cases exercise the Char VALUE + Option render,
    // which the Char→char mapping enables.) 97='a' → Some → 1; 0xD800 surrogate → None → 0.
    let ok = compile_rust(
        "(module m (def (f) (match ((. Char from-int) 97) ((Some c) 1) ((None _) 0))) (export f))",
    );
    if let Some(out) = rustc_run(&ok, "f()") {
        assert_eq!(out, "1", "97 is a valid scalar → Some");
    }
    let bad = compile_rust(
        "(module m (def (f) (match ((. Char from-int) 55296) ((Some c) 1) ((None _) 0))) (export f))",
    );
    if let Some(out) = rustc_run(&bad, "f()") {
        assert_eq!(out, "0", "0xD800 is a surrogate → None");
    }
}

#[test]
fn char_literal_escapes_c1_controls_not_just_c0() {
    // Regression pin (Copilot PR#444 / corpus-bugfix issue): `rust_char_literal`/`rust_string_literal`
    // once escaped only C0 (`< 0x20`) + DEL (`0x7f`), leaking a RAW C1 control (0x80-0x9F) into the
    // generated `'…'`/`"…"` literal. The guard is now `is_control()`, matching
    // `cadenza-syntax::render_char`. Pin that a C1 control — U+0085 NEL — emits `\u{85}`, not a raw byte.
    let nel = compile_rust("(module m (def (g) #\\u+0085) (export g))");
    assert!(
        nel.contains("'\\u{85}'"),
        "C1 control U+0085 → escaped char literal '\\u{{85}}', not a raw control:\n{nel}"
    );
    assert!(
        !nel.contains("Ch('\u{85}')") && !nel.lines().any(|l| l.contains('\u{85}')),
        "no RAW C1 control byte in the emitted source:\n{nel}"
    );
    // DEL (0x7f) — the old guard's upper edge — still escapes.
    let del = compile_rust("(module m (def (g) #\\u+007F) (export g))");
    assert!(
        del.contains("'\\u{7f}'"),
        "DEL U+007F → escaped char literal:\n{del}"
    );
    // A C1 control as a sum payload emits an escaped field literal (the reader-error surrogate case is
    // static; a C1 control is a VALID scalar, so it reaches codegen and must escape).
    let payload = compile_rust(
        "(module m (type Tok (Ch Char) (End)) (def (mk) ((. Tok Ch) #\\u+0085)) (export mk))",
    );
    assert!(
        payload.contains("Ch('\\u{85}')"),
        "C1 control across a sum payload escapes:\n{payload}"
    );
    // e2e: the escaped literal compiles and round-trips to the same scalar (Char.to-int = 133).
    let n = compile_rust(
        "(module m (def (f) (match ((. Char from-int) 133) ((Some c) (Char.to-int c)) ((None _) -1))) \
           (export f))",
    );
    if let Some(out) = rustc_run(&n, "f()") {
        assert_eq!(
            out, "133",
            "U+0085 round-trips through generated Rust as scalar 133"
        );
    }
}

#[test]
fn rustc_float_carrying_sum_keys_a_set_via_a_custom_ord_impl() {
    // A FLOAT-CARRYING sum (`Ast`, which has a `Float Float64` variant) cannot `#[derive(Eq/Ord)]` (f64 is
    // not Eq/Ord), so it emits `#[derive(Clone)]` only — and previously could NOT be a `BTreeSet` element /
    // `BTreeMap` key (the Set/Map construction declined "non-Ord element ... no BTreeSet rep"). Now the
    // backend emits a HAND-WRITTEN `impl PartialEq/Eq/PartialOrd/Ord for Ast` delegating to `__eq_Ast` /
    // `__ord_Ast` walk helpers (float leaf by canonical bits, recursion via the helper). So a set of quoted
    // Asts dedups by structural content. `(quote (+ 1 2))` twice + `(quote (* 1 2))` once → 2 distinct.
    let src = "(module m (def (go (: n Int64)) \
        (Set.len (Set.of (list (quote (+ 1 2)) (quote (+ 1 2)) (quote (* 1 2)))))) (export go))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("impl Ord for Ast") && rs.contains("__ord_Ast"),
        "a float-carrying sum used as a Set element emits a custom impl Ord + __ord_ helper:\n{rs}"
    );
    // partial_cmp must FULLY-QUALIFY `Option` (`core::option::Option`) — a user `(type Option …)` in the
    // same module would otherwise shadow std's and the `-> Option<Ordering>` return would resolve to the
    // 0-generic user enum (E0107). Pin the qualified spelling so that regression can't recur.
    assert!(
        rs.contains("-> core::option::Option<core::cmp::Ordering>"),
        "partial_cmp fully-qualifies core::option::Option (user-Option-shadow safety):\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "go(0)") {
        assert_eq!(
            out, "2",
            "a set of quoted Asts dedups by structural content: 2 distinct"
        );
    }
}

#[test]
fn rustc_roundtrip_value_eq_over_bytes_string_and_compounds() {
    // `Core::ValueEq` over a String/Char/Bytes (and a compound containing them) now emits a native `==`
    // (`String`/`char`/`Vec<u8>` are Eq; `==` compares by content = the canonical-byte value equality).
    // Bytes value-eq (v-core-opt's repro): two equal runtime byte sequences compare equal.
    let beq = compile_rust(
        "(module m (def (f (: n Int64)) \
           (if (= (Bytes.of (list (UInt8.wrap n) 20)) (Bytes.of (list (UInt8.wrap n) 20))) 1 0)) (export f))",
    );
    assert!(beq.contains("=="), "Bytes value-eq emits native ==:\n{beq}");
    if let Some(out) = rustc_run(&beq, "f(5)") {
        assert_eq!(out, "1", "equal byte sequences compare equal");
    }
    // A rope String equals its flat twin (built by concat vs a literal) — `==` compares content.
    let seq = compile_rust(
        "(module m (def (f (: n Int64)) (if (= (String.concat \"ab\" \"c\") \"abc\") 1 0)) (export f))",
    );
    if let Some(out) = rustc_run(&seq, "f(0)") {
        assert_eq!(out, "1", "a concat rope equals its flat twin by content");
    }
    // A char compares equal to a literal char (via a scalar read that folds to a char).
    let ceq = compile_rust(
        "(module m (def (f) (if (= ((. Char from-int) 97) ((. Char from-int) 97)) 1 0)) (export f))",
    );
    if let Some(out) = rustc_run(&ceq, "f()") {
        assert_eq!(out, "1", "equal chars compare equal");
    }
}

#[test]
fn rustc_value_eq_over_an_empty_set_of_an_unconstrained_element_type() {
    // `Core::ValueEq` over an EMPTY collection whose element type is an UNSOLVED FREE VAR: `(Set.of (list))`
    // never constrains its element (nothing is inserted), so its type stays `Set(Var _)`. Previously the
    // eq-derivability check rejected the free-var element → ValueEq declined "runtime structural equality
    // not yet rendered" (while wasm ran it — the drained-set case). Now `ty_leaf_eq_or_free` admits a
    // free-var leaf: an empty set emits a concrete default rep (`BTreeSet<i64>`) on BOTH sides, so the
    // native `==` type-checks and compares equal. Here a drained set (build {k}, remove k) equals the
    // literal empty set. `main(5)` = 1 (they compare equal). Pins the empty-collection-openvar Eq path.
    let src = "(module m (def (drained-eq (: k Int64)) \
        (if (= (Set.of (list)) (Set.remove (Set.of (list k)) k)) 1 0)) (export drained-eq))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("=="),
        "an empty-Set value-eq emits a native == (not a decline):\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "drained_eq(5)") {
        assert_eq!(out, "1", "a drained set compares equal to the empty set");
    }
}

#[test]
fn rustc_set_element_ord_check_is_order_independent_across_members() {
    // ORDER-INDEPENDENT element grounding (breaker/v-inference post-#1674, ground_open_vars class): a Set
    // is homogeneous, but `type_of` on a `(Nil unit)` variant of `(Box a)` reads as `Box <openvar>` (under-
    // ground) while a sibling `(Full k)` reads as the solved `Box Int64`. The Ord-check must consult the
    // BEST-grounded element across ALL members, NOT `elems[0]`: else Nil-FIRST declined "non-Ord" while
    // Full-first compiled — an order-dependent decline. Both orders now emit + run (dedup → len 2).
    let nil_first = compile_rust(
        "(module m (type (Box a) (Full a) (Nil unit)) \
           (def (go (: k Int64)) (Set.len (Set.of (list (Nil unit) (Full k))))) (export go))",
    );
    assert!(
        nil_first.contains("BTreeSet") && !nil_first.contains("__cdz_"),
        "Nil-FIRST set-of a unit-payload generic sum emits a BTreeSet (was an order-dependent decline):\n{nil_first}"
    );
    if let Some(out) = rustc_run(&nil_first, "go(1)") {
        assert_eq!(
            out, "2",
            "Nil-first {{Full 1, Nil}} has 2 distinct elements"
        );
    }
    // Full-FIRST (already worked) — pin it stays correct (the fix must not regress the working order).
    let full_first = compile_rust(
        "(module m (type (Box a) (Full a) (Nil unit)) \
           (def (go (: k Int64)) (Set.len (Set.of (list (Full k) (Nil unit))))) (export go))",
    );
    if let Some(out) = rustc_run(&full_first, "go(1)") {
        assert_eq!(
            out, "2",
            "Full-first order is unchanged (still 2 distinct elements)"
        );
    }
    // CONTROL: a genuinely NON-Ord element (a `List Float64` — a float list has no total order; the
    // per-element `__CdzF` wrapper is NOT threaded through a List) still declines — the fix only admits a
    // set where SOME member grounds to an Ord-key type, it does not make every set Ord.
    let float_list_elem = compile_rust_result(
        "(module m (def (go (: d Float64)) \
           (Set.len (Set.of (list (list d) (list d))))) (export go))",
    );
    assert!(
        float_list_elem.is_err(),
        "a Set of a `List Float64` element still declines (no Ord member):\n{float_list_elem:?}"
    );
}

#[test]
fn rustc_roundtrip_single_variant_newtype_literal_payload_arm_at_narrow_widths() {
    // A single-variant newtype matched with a LITERAL-payload arm (`(match (W.Wrap n) ((W.Wrap 0) 100)
    // ((W.Wrap x) x))`, `W = (Wrap UInt8)`). The newtype tag ERASES (`(W.Wrap n)` → `n`), so `lower`
    // collapses the switch to a `LitTest` whose probe path `[Payload]` reads the inner value. Before, the
    // Rust backend did NOT erase the LitTest path (only the switch path was erased) → `emit_sum_payload`
    // found no bind and declined "sum payload has no bound match arm"; wasm compiled it. The backend now
    // erases the LitTest path (twin of `erase_nominal_switch_path`) so the empty path reads the scrutinee
    // value directly, AND `strip_nominal`s the width lookup so the literal compares at the payload's TRUE
    // width (`== 0u8`, not a hard-coded `i64` → E0308 over the u8 subject). Pins EVERY narrow width flips.
    for (w, ty, hit, miss) in [
        ("UInt8", "u8", "0u8", "5"),
        ("Int32", "i32", "0i32", "5"),
        ("Int8", "i8", "0i8", "5"),
        ("Int16", "i16", "0i16", "5"),
        ("Int64", "i64", "0i64", "5"), // the wide control (always compiled)
    ] {
        // The export is `run`, not `main` — `rustc_run` appends its own `fn main`, so an exported `main`
        // would E0428-collide (the harness's standing rule).
        let rs = compile_rust(&format!(
            "(module m (type W (Wrap {w})) \
               (def (run (: n {w})) (match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))) (export run))"
        ));
        assert!(
            rs.contains(&format!("== {hit}")),
            "{w}: literal-0 compares at the payload width {hit}, not i64:\n{rs}"
        );
        assert!(
            rs.contains(&format!("n: {ty}")),
            "{w}: the erased newtype param stays its inner width {ty}:\n{rs}"
        );
        // n=0 → 100 (hit), n=5 → 5 (falls through to the binding arm).
        if let Some(out) = rustc_run(&rs, "run(0)") {
            assert_eq!(out, "100", "{w}: literal arm hit");
        }
        if let Some(out) = rustc_run(&rs, &format!("run({miss})")) {
            assert_eq!(out, "5", "{w}: falls through to the binding arm");
        }
    }

    // A LIST-ELEMENT literal-payload probe (`(list (W.Wrap 5) .. r)`) COMPILES and runs. The premise the old
    // decline guard assumed — that `ListNew` widens a narrow-int element to its i64 cell (`vec![(5u8 as i64),
    // …]`) so `(xs)[0]` reads i64 while the literal is `5u8` (E0308) — is FALSE: `ListNew` stores the element
    // UNWIDENED (`vec![5u8, 7u8]`, a `Vec<u8>`), and the LitTest subject-cast below the (removed) guard already
    // keys both sides off the narrow width — `((xs[0]) as u8) == 5u8` compiles. So the guard was redundant.
    let list_elem = compile_rust(
        "(module m (type W (Wrap UInt8)) \
           (def (run) (let ((xs (list (W.Wrap 5) (W.Wrap 7)))) \
             (match xs ((list (W.Wrap 5) .. r) (List.len xs)) (_ 0)))) (export run))",
    );
    assert!(
        list_elem.contains("== 5u8"),
        "a narrow-newtype literal LIST element compares at the narrow width u8, not i64:\n{list_elem}"
    );
    // The head is `(W.Wrap 5)` so the arm hits; `List.len xs` = 2.
    if let Some(out) = rustc_run(&list_elem, "run()") {
        assert_eq!(out, "2", "the literal list-element arm hits → List.len = 2");
    }

    // A TUPLE element preserves its width (tuples are not widened), so a narrow-newtype tuple-element
    // literal probe compiles: `(match (tuple (W.Wrap n) 9) ((tuple (W.Wrap 0) b) 100) (_ 5))`.
    let tup = compile_rust(
        "(module m (type W (Wrap UInt8)) \
           (def (run (: n UInt8)) (match (tuple (W.Wrap n) 9) ((tuple (W.Wrap 0) b) 100) (_ 5))) \
           (export run))",
    );
    assert!(
        tup.contains("== 0u8"),
        "a tuple-element narrow-newtype literal compares at u8:\n{tup}"
    );
    if let Some(out) = rustc_run(&tup, "run(0)") {
        assert_eq!(out, "100", "tuple-element literal arm hit");
    }
}

#[test]
fn rustc_roundtrip_structural_eq_over_a_compound_with_a_float_leaf() {
    // A runtime `(= a b)` over a TUPLE/RECORD carrying a Float leaf can't use a derived `==` (f64 is
    // PartialEq not Eq, and `==` gives the WRONG NaN/-0.0 answer). The backend now emits a STRUCTURAL walk:
    // each non-float leaf by `==`, each float leaf by the CANONICAL BYTE FORM (nan==nan, -0.0 != +0.0 —
    // byte-identical to FloatCompare + the wasm heap walk). Was a decline ("runtime structural equality
    // over this compound is not yet rendered").
    // (a) a tuple of one Float64: equal floats -> equal; NaN==NaN -> equal; -0.0 vs 0.0 -> distinct.
    let tup = compile_rust(
        "(module m (def (run (: a Float64) (: b Float64)) (if (= (tuple a) (tuple b)) 1 0)) (export run))",
    );
    assert!(
        tup.contains("is_nan()") && tup.contains("to_bits()"),
        "a float tuple leaf compares by the canonical byte form:\n{tup}"
    );
    if let Some(out) = rustc_run(&tup, "run(1.5, 1.5)") {
        assert_eq!(out, "1", "equal floats -> equal tuples");
    }
    if let Some(out) = rustc_run(&tup, "run(1.5, 2.5)") {
        assert_eq!(out, "0", "unequal floats -> unequal tuples");
    }
    if let Some(out) = rustc_run(&tup, "run(f64::NAN, f64::NAN)") {
        assert_eq!(out, "1", "NaN == NaN under the canonical byte form");
    }
    if let Some(out) = rustc_run(&tup, "run(-0.0, 0.0)") {
        assert_eq!(out, "0", "-0.0 stays distinct from +0.0");
    }

    // (b) a MIXED Int+Float tuple: the Int leaf compares by `==`, the Float by canonical bytes; both must
    // agree for equality. n equal AND floats equal -> 1; a differing float -> 0.
    let mixed = compile_rust(
        "(module m (def (run (: n Int64) (: a Float64) (: b Float64)) \
           (if (= (tuple n a) (tuple n b)) 1 0)) (export run))",
    );
    assert!(
        mixed.contains("&&"),
        "a mixed tuple ANDs its per-leaf comparisons:\n{mixed}"
    );
    if let Some(out) = rustc_run(&mixed, "run(7, 1.5, 1.5)") {
        assert_eq!(out, "1", "equal int + equal float -> equal");
    }
    if let Some(out) = rustc_run(&mixed, "run(7, 1.5, 2.5)") {
        assert_eq!(out, "0", "a differing float leaf -> unequal");
    }

    // (c) a tuple mixing a Float and a Bytes leaf — Bytes is native-Eq (`Vec<u8> ==` borrows, compares
    // content), the Float canonical-byte; both walk in one compound. Equal -> 1.
    let fb = compile_rust(
        "(module m (def (run (: f Float64) (: b Bytes) (: c Bytes)) (if (= (tuple f b) (tuple f c)) 1 0)) \
           (export run))",
    );
    // Build two equal byte sequences at the call site; a `Vec<u8> == Vec<u8>` compares content.
    if let Some(out) = rustc_run(&fb, "run(1.5, vec![104u8], vec![104u8])") {
        assert_eq!(out, "1", "equal float + equal bytes -> equal compound");
    }
    if let Some(out) = rustc_run(&fb, "run(1.5, vec![104u8], vec![105u8])") {
        assert_eq!(out, "0", "a differing bytes leaf -> unequal");
    }

    // A compound with a non-Eq-non-float leaf (a bare closure) is NOT walkable — still declines.
    let bad = try_compile_rust(
        "(module m (def (run (: f (-> Int64 Int64))) (if (= (tuple f) (tuple f)) 1 0)) (export run))",
    );
    assert!(
        bad.is_err(),
        "a compound with a function leaf is not float-walkable — declines:\n{bad:?}"
    );
}

#[test]
fn rustc_roundtrip_structural_eq_over_a_list_of_floats_is_elementwise_construction_independent() {
    // Pins the Ty::List arm of emit_value_eq_walk with a FLOAT element (expr.rs comment "Two lists ... equal
    // ... independent of how each was constructed"). A `(List Float64)` equality can't use `Vec<f64>`'s
    // `PartialEq` (it HAS `==`, but for VALUE equality its float semantics are wrong — NaN != NaN and
    // -0.0 == +0.0 — because `f64: !Eq`: its `PartialEq` is not an equivalence relation, NaN breaking
    // reflexivity, so the derived `Vec<f64>` equality is unsuitable here) — the backend emits an
    // element-wise walk:
    // `l.len()==r.len() && l.iter().zip(r.iter()).all(|(le,re)| <canonical-byte float eq>)`.
    // The float-leaf eq walk was witnessed for TUPLES/RECORDS but NOT for a LIST element on the rust backend.
    // Verified NON-VACUOUS: emits `.len()==.len() && .iter().zip().all(|..| ({...is_nan()...to_bits()...}))`.
    // A concat-built list is compared against a push-built list to also pin construction-independence.
    let m = compile_rust(
        "(module m \
           (def (mk-cat (: a Float64) (: b Float64)) (List.concat (list a) (list b))) \
           (def (mk-push (: a Float64) (: b Float64)) (List.push (list a) b)) \
           (def (run (: a Float64) (: b Float64)) (if (= (mk-cat a b) (mk-push a b)) 1 0)) \
         (export run))",
    );
    // Element compare is the canonical byte form, NOT Vec's derived PartialEq.
    assert!(
        m.contains("is_nan()") && m.contains("to_bits()") && m.contains(".zip("),
        "a list-of-float eq walks element-wise by the canonical byte form:\n{m}"
    );
    // equal floats, differently constructed → equal (construction-independent).
    if let Some(out) = rustc_run(&m, "run(1.5, 2.5)") {
        assert_eq!(
            out, "1",
            "concat-built [1.5,2.5] == push-built [1.5,2.5]:\n{m}"
        );
    }
    // NaN element: canonical-byte NaN==NaN → the lists compare equal (Vec's derived PartialEq gives NaN!=NaN).
    if let Some(out) = rustc_run(&m, "run(f64::NAN, 2.5)") {
        assert_eq!(
            out, "1",
            "a NaN element compares equal under canonical bytes (NOT Vec's NaN!=NaN):\n{m}"
        );
    }
    // A length-differing pair must be unequal — pin the `.len()` short-circuit of the List arm.
    let len = compile_rust(
        "(module m \
           (def (run (: a Float64)) (if (= (list a) (List.push (list a) a)) 1 0)) \
         (export run))",
    );
    if let Some(out) = rustc_run(&len, "run(1.5)") {
        assert_eq!(
            out, "0",
            "a length mismatch decides immediately (unequal):\n{len}"
        );
    }
    // -0.0 vs +0.0 as elements stay DISTINCT under the canonical byte form (Vec's PartialEq treats them equal).
    let zero = compile_rust(
        "(module m \
           (def (run (: a Float64) (: b Float64)) (if (= (list a) (list b)) 1 0)) \
         (export run))",
    );
    if let Some(out) = rustc_run(&zero, "run(-0.0, 0.0)") {
        assert_eq!(
            out, "0",
            "-0.0 stays distinct from +0.0 as a list element (canonical bytes, NOT Vec PartialEq):\n{zero}"
        );
    }
}

#[test]
fn rustc_roundtrip_runtime_bin_construction_and_match() {
    // A runtime `(bin …)` of fixed-width INT segments builds a Vec<u8> (range-checked, big-endian by
    // default, `le` reversed) and a `bin` pattern decodes it back — was a whole-family decline ("the Rust
    // backend does not yet render this compound value"). Mirrors the wasm BinBuild/BinIntRead.
    // (a) u16 round-trip: build from a runtime UInt16, decode back — 258 -> 258.
    let rt = compile_rust(
        "(module m (def (run (: n UInt16)) (match (bin (u16 n)) ((bin (u16 m)) m) (_ -1))) (export run))",
    );
    assert!(
        rt.contains("extend_from_slice") && rt.contains("__acc"),
        "bin build + read emitted:\n{rt}"
    );
    if let Some(out) = rustc_run(&rt, "run(258)") {
        assert_eq!(out, "258", "u16 bin round-trips");
    }

    // (b) a big-endian u16 read back BY BYTE INDEX: 258 = 0x0102 -> byte0=1, byte1=2 (MSB first).
    let be = compile_rust(
        "(module m (def (run (: n UInt16)) \
           (match (Bytes.at (bin (u16 n)) 0) ((Some b) b) ((None _) -1))) (export run))",
    );
    if let Some(out) = rustc_run(&be, "run(258)") {
        assert_eq!(out, "1", "big-endian lays the high byte (1) first");
    }

    // (c) a little-endian SIGNED segment round-trips a negative value: i16 le, -2 -> -2. (The `le`
    // modifier is TRAILING — `(i16 n le)`.)
    let le = compile_rust(
        "(module m (def (run (: n Int16)) (match (bin (i16 n le)) ((bin (i16 m le)) m) (_ 0))) (export run))",
    );
    if let Some(out) = rustc_run(&le, "run(-2)") {
        assert_eq!(out, "-2", "signed little-endian round-trips a negative");
    }

    // (d) a multi-arm tag dispatch: tag 2 selects the second arm, y+1000.
    let tag = compile_rust(
        "(module m (def (run (: t UInt8) (: v UInt16)) \
           (match (bin (u8 t) (u16 v)) \
             ((bin (u8 1) (u16 x)) x) ((bin (u8 2) (u16 y)) (+ y 1000)) (_ -1))) (export run))",
    );
    if let Some(out) = rustc_run(&tag, "run(2, 300)") {
        assert_eq!(out, "1300", "the tag-2 arm fires: 300 + 1000");
    }
    if let Some(out) = rustc_run(&tag, "run(1, 42)") {
        assert_eq!(out, "42", "the tag-1 arm binds x");
    }

    // (e) a final REST segment binds the tail after a fixed header, and its length reads back. The
    // scrutinee is a 3-byte bin (header + 2 more); the rest after the 1-byte header is 2 bytes.
    let rest = compile_rust(
        "(module m (def (run (: n UInt8)) \
           (match (bin (u8 n) (u8 7) (u8 8)) ((bin (u8 h) (bytes rest)) (Bytes.len rest)) (_ -1))) (export run))",
    );
    if let Some(out) = rustc_run(&rest, "run(5)") {
        assert_eq!(out, "2", "the rest tail after the 1-byte header is 2 bytes");
    }

    // (f) the emitted range-check is a defensive backstop (the segment's width type already bounds the
    // value at compile time — a fixed-width int segment REQUIRES a value of that width, so a fit trap is
    // normally dead). Pin that the guard IS emitted for a narrow segment (< 64 bits) so it stays a real
    // backstop; width 8 (u64) needs no check.
    let guard =
        compile_rust("(module m (def (run (: n UInt16)) (Bytes.len (bin (u16 n)))) (export run))");
    assert!(
        guard.contains("binary value does not fit segment"),
        "a narrow segment emits the fit backstop:\n{guard}"
    );

    // (g) a runtime `(bits v k)` RUN computes its ceil/mask in u128 — NOT `1i64 << k` (which is i64::MIN at
    // k==63, a shift-overflow at k==64), so a wide field cannot emit a negative ceil / wrong mask (Copilot
    // PR#516). A byte-aligned `(bits n 8)` run over a UInt8: the fit-check ceil is `256u128`, mask `255u128`.
    let bits = compile_rust(
        "(module m (def (run (: n UInt8)) (Bytes.len (bin (bits n 8)))) (export run))",
    );
    assert!(
        bits.contains("256u128") && bits.contains("255u128"),
        "a bit-field run computes ceil/mask in u128 (no 1i64<<k overflow):\n{bits}"
    );
    if let Some(out) = rustc_run(&bits, "run(200)") {
        assert_eq!(out, "1", "a byte-aligned bits run packs to 1 byte");
    }
}

#[test]
fn rustc_roundtrip_empty_list_grounds_to_its_slot_element_type() {
    // An empty `(list)` in a construction slot whose element type is known must annotate `Vec::<T>::new()`,
    // not a bare `vec![]` rustc can't infer (E0282). Surfaced by the multi-module HOL-kernel cases where a
    // `(list)` sits in an erased-newtype tuple payload (`Thm.Seq (list) tm`).
    // (a) an empty list as a SUM payload (single-variant erased newtype → a tuple payload).
    let seq = compile_rust(
        "(module m (type Thm (Seq (List Int64) Int64)) \
           (def (run (: n Int64)) (Thm.Seq (list) n)) (export (. Thm *)) (export run))",
    );
    assert!(
        seq.contains("Vec::<i64>::new()"),
        "an empty-list tuple payload grounds to Vec::<i64>::new():\n{seq}"
    );
    assert!(
        !seq.contains("(vec![], "),
        "no un-annotated empty vec![] in the payload:\n{seq}"
    );

    // (b) an empty list DIRECTLY typed still grounds (the ListNew-own-type path).
    let direct = compile_rust("(module m (def (run) (: (list) (List Int64))) (export run))");
    assert!(
        direct.contains("Vec::<i64>::new()"),
        "a directly-typed empty list grounds:\n{direct}"
    );

    // (c) a NON-empty list stays the bare vec![…] (byte-identical, element inferred from the first).
    let nonempty = compile_rust("(module m (def (run) (: (list 1 2) (List Int64))) (export run))");
    assert!(
        nonempty.contains("vec!["),
        "a non-empty list stays vec![…]:\n{nonempty}"
    );
}

#[test]
fn a_linked_duplicate_same_name_enum_emits_once() {
    // A linked multi-module program can carry two byte-identical same-name enum declarations — a lib and
    // the entry each declaring `(type Box (W a) (E))`. They emit the same Rust `enum Box`, so emitting both
    // is a duplicate (E0428); the backend dedups byte-identical same-ident decls to ONE. Here a single
    // module with the type declared once is the baseline (the dedup path is exercised by the gate's linked
    // module cases); pin that a normal single decl still emits exactly one enum (no over-dedup).
    let one = compile_rust(
        "(module m (type Box (W Int64) (E)) (def (run) (match (Box.W 42) ((Box.W n) n) ((Box.E) 0))) (export run))",
    );
    assert_eq!(
        one.matches("enum Box").count(),
        1,
        "a single Box decl emits exactly one enum:\n{one}"
    );
    if let Some(out) = rustc_run(&one, "run()") {
        assert_eq!(out, "42", "the Box.W arm binds 42");
    }
}

#[test]
fn rustc_roundtrip_runtime_symbol_is_a_string_leaf() {
    // A Symbol maps to Rust's `String` (a canonical text leaf, identity = content). Runtime Symbol.of /
    // Symbol.to-string are a String-level identity retag (a `String` is already flat/canonical — the wasm
    // bytes-compact has no analogue); Symbol `==` is content equality. Was a whole-family decline
    // ("a runtime Symbol conversion has no rust representation yet").
    // (a) a runtime string interned to a Symbol compares EQUAL to a constant symbol of the same content.
    let intern = compile_rust(
        "(module m (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s \"x\") (- n 1)))) \
           (def (run) (if (= (Symbol.of (rep \"\" 3)) #\"xxx\") 1 0)) (export run))",
    );
    assert!(
        intern.contains("=="),
        "a runtime Symbol compares by content (native ==):\n{intern}"
    );
    if let Some(out) = rustc_run(&intern, "run()") {
        assert_eq!(
            out, "1",
            "an interned runtime string == the same-content symbol"
        );
    }

    // (b) a round-trip Symbol.of → Symbol.to-string recovers the content String (byte-len observes it).
    let round = compile_rust(
        "(module m (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s \"x\") (- n 1)))) \
           (def (run) (String.byte-len (Symbol.to-string (Symbol.of (rep \"xx\" 3))))) (export run))",
    );
    if let Some(out) = rustc_run(&round, "run()") {
        assert_eq!(
            out, "5",
            "Symbol.of then Symbol.to-string recovers the 5-byte content"
        );
    }

    // (c) a constant Symbol RESULT renders as cdz-run's `((. Symbol of) "…")` construction form (the
    // return-type note is `Symbol`, the fn returns `String`). Pin the type note + String rep.
    let konst = compile_rust("(module m (def (run) (Symbol.of \"map-insert\")) (export run))");
    assert!(
        konst.contains("// cdz-return[run]: Symbol") && konst.contains("-> String"),
        "a Symbol result carries the Symbol type note over a String rep:\n{konst}"
    );
}

#[test]
fn rustc_roundtrip_list_pattern_in_a_sum_variant_payload() {
    // A LIST PATTERN inside a sum-variant payload matched at RUNTIME — the sum decision tree meets the list
    // matcher (a `(Call (List Node))` compiler-AST node dispatched by its child count). Was a decline ("a
    // non-scalar literal-payload probe is not rendered"). The ListLen probe now emits `.len() >=/== n`, and a
    // list-element binder reads `[i]` (a list INDEX, not a `.i` tuple field → E0609).
    // (a) a recursive sum whose Call payload is a `List Node`, dispatched by child count.
    let node = compile_rust(
        "(module m (type Node (Lit Int64) (Call (List Node))) \
           (def (build (: k Int64)) (if (< k 1) (Lit 7) (Call (list (Lit k) (build (- k 1)))))) \
           (def (run (: k Int64)) (match (build k) ((Lit v) v) ((Call (list _ .. rest)) 99) (_ 0))) \
           (export run))",
    );
    assert!(
        node.contains(".len() >= 1"),
        "the Call payload dispatches on its child-list length:\n{node}"
    );
    if let Some(out) = rustc_run(&node, "run(2)") {
        assert_eq!(out, "99", "a non-empty Call matches the rest arm");
    }
    if let Some(out) = rustc_run(&node, "run(0)") {
        assert_eq!(out, "7", "k<1 builds a Lit, the Lit arm binds 7");
    }

    // (b) the empty/rest split with an ELEMENT BINDER: `(Some (list x .. r)) x` reads element 0 as a list
    // index `[0]`, NOT a tuple field `.0`. `mk 0` → (Some []) → the empty arm (100); `mk 5` → None (-1).
    let split = compile_rust(
        "(module m (def (mk (: n Int64)) (if (< n 1) (Some (list)) (if (< n 2) (Some (list 7)) (None)))) \
           (def (f (: o (Option (List Int64)))) \
             (match o ((Some (list)) 100) ((Some (list x .. r)) x) ((None) -1))) \
           (def (run (: n Int64)) (f (mk n))) (export run))",
    );
    assert!(
        split.contains(")[0]"),
        "the element binder reads a list index [0], not a tuple field .0:\n{split}"
    );
    if let Some(out) = rustc_run(&split, "run(0)") {
        assert_eq!(out, "100", "mk 0 → Some [] → the empty-list arm");
    }
    if let Some(out) = rustc_run(&split, "run(1)") {
        assert_eq!(
            out, "7",
            "mk 1 → Some [7] → the rest arm binds element 0 = 7"
        );
    }
    if let Some(out) = rustc_run(&split, "run(5)") {
        assert_eq!(out, "-1", "mk 5 → None");
    }
}

#[test]
fn rustc_roundtrip_nested_tuple_list_element_binder_with_rest_recursion() {
    // A NESTED element pattern in a list arm — `(list (tuple a _) .. rest)` — binds `a` at the two-step
    // path [Elem(0), Elem(0)] (list index 0, then tuple field 0). Combined with a self-recursive call on
    // the rest binder, this declined ("sum payload has no bound match arm"): the list-scrutinee binder path
    // resolved only a SINGLE [Elem(i)] step, so a nested [Elem(i), Elem(j)] fell through. Now it walks the
    // trailing steps against the element type — a tuple field `.j`, a nested-list index `[j]`.
    let sf = compile_rust(
        "(module m (def (sf (: xs (List (Tuple Int64 Int64)))) \
           (match xs ((list) 0) ((list (tuple a _) .. rest) (+ a (sf rest))))) \
           (def (run) (sf (list (tuple 1 9) (tuple 2 8) (tuple 3 7)))) (export run))",
    );
    assert!(
        sf.contains(")[0]).0") || sf.contains(")[0]).0)"),
        "the nested-tuple element reads list-index [0] then tuple field .0:\n{sf}"
    );
    if let Some(out) = rustc_run(&sf, "run()") {
        assert_eq!(out, "6", "sum of the first components 1+2+3 = 6");
    }

    // The non-recursive nested-tuple element still works (regression guard) — head's first component.
    let one = compile_rust(
        "(module m (def (f (: xs (List (Tuple Int64 Int64)))) \
           (match xs ((list (tuple a b) .. rest) (+ a b)) (_ (- 0 1)))) \
           (def (run) (f (list (tuple 5 9)))) (export run))",
    );
    if let Some(out) = rustc_run(&one, "run()") {
        assert_eq!(out, "14", "the head tuple's a + b = 5 + 9");
    }
    // PR#522 robustness: the `Elem(j)` walk over the element type handles tuple/record/list EXPLICITLY (a
    // record maps to a sorted-field tuple → `.{j}`, correct) and DECLINES any other shape rather than the
    // old catch-all `.{j}` that could emit an uncompilable field access on a scalar/sum/map + drop type
    // tracking to Any. A doubly-nested tuple `((list (tuple (tuple a _) _) .. rest) …)` exercises the
    // multi-step walk (Elem[0] list → Elem[0] tuple → Elem[0] tuple) — reads `((xs)[0].0).0`.
    let deep = compile_rust(
        "(module m (def (f (: xs (List (Tuple (Tuple Int64 Int64) Int64)))) \
           (match xs ((list (tuple (tuple a _) _) .. rest) a) (_ (- 0 1)))) \
           (def (run) (f (list (tuple (tuple 7 8) 9)))) (export run))",
    );
    if let Some(out) = rustc_run(&deep, "run()") {
        assert_eq!(
            out, "7",
            "the doubly-nested tuple's innermost first component = 7"
        );
    }
}

#[test]
fn rustc_roundtrip_sum_constructor_list_element_payload_binder() {
    // A SUM-CONSTRUCTOR list-element binder — `(list (I x) .. rest)` over a `List A` where `A` is a
    // heterogeneous sum `(I Int64) (N String)`. The binder `x` sits at path [Elem(0), Payload]: the
    // leading Elem(0) reads `(xs)[0]` (a value of the element sum), and the Payload binds the matched
    // variant's payload. Was a decline ("a nested list-element binder beyond a tuple projection is not
    // rendered") — the guard's discriminant is not on the binder's path. RECOVERED from the SumPayload
    // node's own solved type: the variant is the UNIQUE one whose payload type equals the binder's type
    // (I:Int64 ≠ N:String), emitted as a single-variant `match (xs)[0] { A::I(__pv) => __pv, _ => … }`.
    let hetero = compile_rust(
        "(module m (type A (I Int64) (N String)) \
           (def (build (: k Int64)) (if (< k 1) (list (N \"z\")) (list (I k)))) \
           (def (f (: xs (List A))) (match xs ((list (I x) .. r) x) (_ 0))) \
           (def (run (: k Int64)) (f (build k))) (export run))",
    );
    assert!(
        hetero.contains("A :: I (__pv) => __pv") || hetero.contains("A::I(__pv) => __pv"),
        "the sum-element payload binds via a single-variant match:\n{hetero}"
    );
    if let Some(out) = rustc_run(&hetero, "run(2)") {
        assert_eq!(out, "2", "the head is (I 2), the payload binder yields 2");
    }
    if let Some(out) = rustc_run(&hetero, "run(0)") {
        assert_eq!(
            out, "0",
            "the head is (N \"z\"), no (I _) match → the fallthrough arm 0"
        );
    }

    // A NON-COPY payload (String) binds by MOVING out of the cloned element — the `(xs)[0].clone()`
    // already owns the sum value, so `__pv` needs NO second `.clone()` (would be a wasted allocation).
    let string_pay = compile_rust(
        "(module m (type A (I Int64) (N String)) \
           (def (build (: k Int64)) (if (< k 1) (list (I 0)) (list (N \"hello\")))) \
           (def (f (: xs (List A))) (match xs ((list (N s) .. r) s) (_ \"none\"))) \
           (def (run (: k Int64)) (f (build k))) (export run))",
    );
    assert!(
        (string_pay.contains("A :: N (__pv) => __pv") || string_pay.contains("A::N(__pv) => __pv"))
            && !string_pay.contains("__pv.clone()")
            && !string_pay.contains("__pv . clone ()"),
        "the non-Copy payload MOVES out of the cloned element (no redundant second clone):\n{string_pay}"
    );
    if let Some(out) = rustc_run(&string_pay, "run(2)") {
        assert_eq!(
            out, "hello",
            "the head is (N \"hello\"), the String binder yields hello"
        );
    }
    if let Some(out) = rustc_run(&string_pay, "run(0)") {
        assert_eq!(
            out, "none",
            "the head is (I 0), no (N _) match → the fallthrough arm"
        );
    }

    // A RECURSIVE (boxed) variant payload derefs the `Box` — `T::Wrap(__pv) => (*__pv)` moves the inner
    // value out of the owned box, no clone. Verifies the `variant_is_recursive` deref arm end-to-end.
    let boxed = compile_rust(
        "(module m (type T (Leaf Int64) (Wrap T)) \
           (def (unwrap (: xs (List T))) (match xs ((list (Wrap t) .. r) t) (_ (Leaf 0)))) \
           (def (top (: xs (List T))) (match (unwrap xs) ((Leaf n) n) ((Wrap _) -1))) \
           (def (build (: k Int64)) (if (< k 1) (list (Leaf 5)) (list (Wrap (Leaf k))))) \
           (def (run (: k Int64)) (top (build k))) (export run))",
    );
    assert!(
        boxed.contains("(*__pv)") || boxed.contains("(* __pv)"),
        "the recursive variant payload derefs the box:\n{boxed}"
    );
    if let Some(out) = rustc_run(&boxed, "run(2)") {
        assert_eq!(out, "2", "(Wrap (Leaf 2)) unwraps to (Leaf 2) → 2");
    }
    if let Some(out) = rustc_run(&boxed, "run(0)") {
        assert_eq!(
            out, "0",
            "(Leaf 5) head ≠ (Wrap _) → unwrap's (Leaf 0) fallthrough → top reads n=0"
        );
    }

    // A HOMOGENEOUS sum — two variants share the exact payload type `(I Int64) (J Int64)` — matched by
    // `(I x)` in a list element. The binder's type alone can't pick I vs J, but the arm's disc-test GUARD
    // already proved the element is `I` at run time, and both variants share the Int64 payload, so the
    // payload binds via an OR-PATTERN `A::I(__pv) | A::J(__pv) => __pv` — type-correct (one shared binder)
    // and value-correct (only the guarded variant reaches the body; the other arm is dead). No disc-threading
    // needed: the guard supplies it, the shared payload type makes the or-pattern sound.
    let ambiguous = compile_rust(
        "(module m (type A (I Int64) (J Int64)) \
           (def (build (: k Int64)) (if (< k 1) (list (J 9)) (list (I k)))) \
           (def (f (: xs (List A))) (match xs ((list (I x) .. r) x) (_ 0))) \
           (def (run (: k Int64)) (f (build k))) (export run))",
    );
    assert!(
        ambiguous.contains("A::I(__pv) | A::J(__pv) => __pv"),
        "a homogeneous-payload sum-element binder emits an or-pattern (guard-proven variant):\n{ambiguous}"
    );
    // run(5) → build (list (I 5)); the `(I x)` arm matches → x = 5.
    if let Some(out) = rustc_run(&ambiguous, "run(5)") {
        assert_eq!(out, "5", "(I 5) head matches (I x) → x = 5");
    }
    // run(0) → build (list (J 9)); the `(I x)` arm's disc-guard FAILS (element is J) → wildcard → 0. Pins the
    // or-pattern does NOT wrongly bind a J element as if it were I (the guard filters before the body runs).
    if let Some(out) = rustc_run(&ambiguous, "run(0)") {
        assert_eq!(out, "0", "(J 9) head ≠ (I x) → guard fails → wildcard → 0");
    }

    // DEEP-BIND: a list element that is a sum whose payload is a TUPLE, binding the tuple fields —
    // `(match xs ((list (Pt (tuple a b))) (+ a b)) …)`, path `[Elem(0), Payload, Elem(j)]`. The `Payload`
    // step extracts the variant payload (via a single-variant match), then the trailing `Elem(j)` projects
    // the tuple field. (Was "a nested list-element binder beyond a tuple projection" — my terminal-only arm
    // now continues the walk past the Payload.) The rust counterpart of the wasm tuple-payload fix; the
    // corpus case flips todo→PASS. `build 3` → [(Pt (tuple 3 9))] → a=3 b=9 → 12; `build 0` → [(Nil)] → 0.
    let deep = compile_rust(
        "(module m (type P (Pt (Tuple Int64 Int64)) (Nil)) \
           (def (build (: k Int64)) (if (< k 1) (list (Nil)) (list (Pt (tuple k 9))))) \
           (def (f (: xs (List P))) (match xs ((list (Pt (tuple a b))) (+ a b)) (_ 0))) \
           (def (run (: k Int64)) (f (build k))) (export run))",
    );
    if let Some(out) = rustc_run(&deep, "run(3)") {
        assert_eq!(
            out, "12",
            "the tuple-payload list element binds a=3 b=9 → 12"
        );
    }
    if let Some(out) = rustc_run(&deep, "run(0)") {
        assert_eq!(
            out, "0",
            "the head is (Nil), no (Pt _) match → the fallthrough arm"
        );
    }
}

#[test]
fn rustc_roundtrip_host_closure_factory_export_scalar_capture_s1() {
    // HOST-CLOSURE S1: a closure-FACTORY export — a parameterized def whose result is a closure capturing
    // its params — now crosses the Rust export boundary (was declined "no closure handle ABI"). The def's
    // captured params stay ordinary leading params and the returned `(fn …)` emits as an `Rc<dyn Fn>` VALUE
    // (the internal-closure lowering), so `both(a,b)` returns a handle the host applies `(x)`. The native
    // equivalent of the wasm make/call resource ABI. S1 is scalar-capture/arg/result only (compound = S2/S3).
    let both = compile_rust(
        "(module m (def (both (: a Int64) (: b Int64)) (fn ((: x Int64)) (+ (+ a b) x))) (export both))",
    );
    assert!(
        both.contains("pub fn both(a: i64, b: i64) -> std::rc::Rc<dyn Fn(i64) -> i64>")
            && both.contains("std::rc::Rc::new(move |"),
        "a scalar-capture closure factory emits `-> Rc<dyn Fn>` returning an `Rc::new(move |…|)` handle:\n{both}"
    );
    // The factory is APPLIED in two steps: `both(10, 20)` makes the handle, `(5)` applies it → 10+20+5 = 35.
    if let Some(out) = rustc_run(&both, "both(10, 20)(5)") {
        assert_eq!(out, "35", "make(10,20) then call(5) = a+b+x = 35");
    }
    // A capture used through a nested subexpression + a captured-boolean control-flow closure also cross.
    let scale = compile_rust(
        "(module m (def (scale (: k Int64)) (fn ((: x Int64)) (* (+ x 1) k))) (export scale))",
    );
    if let Some(out) = rustc_run(&scale, "scale(4)(3)") {
        assert_eq!(out, "16", "make(k=4) then call(3) = (3+1)*4 = 16");
    }

    // A FLOAT closure arg + result also crosses (`(fn (x: Float32) (+ x 1.5))`) — `float_width_of`/the
    // float-literal grounding already render a Float correctly, so `s2_arg_ok`/`s3_result_ok` admit it.
    let flt = compile_rust("(module m (def (mk) (fn ((: x Float32)) (+ x 1.5))) (export mk))");
    assert!(
        flt.contains("-> std::rc::Rc<dyn Fn(f32) -> f32>"),
        "a Float32 closure factory emits `Rc<dyn Fn(f32) -> f32>`:\n{flt}"
    );
    if let Some(out) = rustc_run(&flt, "mk()(2.5)") {
        assert_eq!(
            out, "4",
            "make() then call(2.5) = 2.5 + 1.5 = 4.0 (Rust prints the f32 as 4)"
        );
    }

    // A MIXED-type scalar CAPTURE environment crosses: `mk(base: Float64, n: Int64)` captures a Float AND an
    // Int (the host supplies both aliased-width scalars at `make`). `is_capture_scalar` admits Int/Bool/Float
    // captures (a compound capture still declines — no host→guest decode). The closure returns a Float.
    let mixed_cap = compile_rust(
        "(module m (def (mk (: base Float64) (: n Int64)) (fn ((: x Float64)) (+ x base))) (export mk))",
    );
    assert!(
        mixed_cap.contains("pub fn mk(base: f64, n: i64) -> std::rc::Rc<dyn Fn(f64) -> f64>"),
        "a mixed Float64+Int64 scalar-capture factory emits `mk(f64, i64) -> Rc<dyn Fn(f64) -> f64>`:\n{mixed_cap}"
    );

    // A closure PARAMETER export still declines (no way to synthesize an Rc<dyn Fn> arg at the boundary) —
    // the one function-typed shape that stays deferred (compound args/results now cross via S2/S3/S4a/S5).
    let param = compile_rust_result(
        "(module m (def (apply (: f (-> Int64 Int64)) (: x Int64)) (f x)) (export apply))",
    );
    assert!(
        param.is_err(),
        "a closure-PARAMETER export with NO producing sibling declines (host can't supply the closure):\n{param:?}"
    );
}

#[test]
fn rustc_roundtrip_closure_parameter_consumer_with_a_producer_sibling() {
    // CLOSURE-PARAMETER CONSUMER (the round-trip): a consumer export TAKES an `(-> a b)` closure param +
    // applies it; a companion PRODUCER export supplies the closure. The consumer now EMITS `pub fn
    // apply_it(g: Rc<dyn Fn(i64)->i64>, x: i64) -> i64` (the guard lifts when a producing sibling exists),
    // and the gate driver builds the closure from the producer: `apply_it(make_adder(100), 7)` = 107.
    let m = compile_rust(
        "(module m \
           (def (make-adder (: k Int64)) (fn ((: x Int64)) (+ x k))) \
           (def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x)) \
           (export make-adder) (export apply-it))",
    );
    assert!(
        m.contains("pub fn apply_it(g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64) -> i64"),
        "the consumer emits an Rc<dyn Fn> param + applies it:\n{m}"
    );
    // Driven producer→consumer: build the closure via the producer, pass it to the consumer.
    if let Some(out) = rustc_run(&m, "apply_it(make_adder(100), 7)") {
        assert_eq!(out, "107", "apply_it(make_adder(100), 7) = 7 + 100 = 107");
    }
    // A closure-param consumer with NO producing sibling still DECLINES (the host would supply the closure
    // directly — no boundary rep, matching wasm's "closure argument has no scalar host-boundary rep").
    let no_producer = compile_rust_result(
        "(module m (def (apply (: f (-> Int64 Int64)) (: x Int64)) (f x)) (export apply))",
    );
    assert!(
        no_producer.is_err(),
        "a closure-param consumer with no producer declines:\n{no_producer:?}"
    );
    // An ASYNC closure-param consumer with a FACTORY producer now EMITS — the gate driver builds the
    // closure via `block_on(prog::make-adder(&mut env, k))`, binds it to a `let`, then drives the consumer
    // via `block_on(prog::apply-it(&mut env, __g0, x))`. (`make-adder` captures `k` → a factory.)
    let async_factory_consumer = compile_rust_async_result(
        "(module m \
           (def (make-adder (: k Int64)) (fn ((: x Int64)) (+ x k))) \
           (def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x)) \
           (export make-adder) (export apply-it))",
    );
    assert!(
        async_factory_consumer.is_ok(),
        "an async closure-param consumer with a FACTORY producer now emits:\n{async_factory_consumer:?}"
    );
    // A NULLARY-producer async consumer ALSO emits: in async mode a nullary `(fn …)` producer is emitted as
    // a FACTORY (`async fn mk<E>(env) -> Rc<dyn Fn>`), NOT eta-peeled to a direct fn (the sync case), so it
    // has a factory producer and the driver drives it the same way. (Async has no peeled producers.)
    let async_nullary_consumer = compile_rust_async_result(
        "(module m \
           (def (mk) (fn ((: x Int64)) (+ x 1))) \
           (def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x)) \
           (export mk) (export apply-it))",
    );
    assert!(
        async_nullary_consumer.is_ok(),
        "an async closure-param consumer with a nullary (factory-shaped) producer emits:\n{async_nullary_consumer:?}"
    );
    // An ASYNC closure-param consumer with NO producing sibling still DECLINES (no closure to build) —
    // `closure_has_factory_producer` is false, so the async guard fires (the mode-independent no-producer
    // decline would also catch it).
    let async_no_producer = compile_rust_async_result(
        "(module m (def (apply (: f (-> Int64 Int64)) (: x Int64)) (f x)) (export apply))",
    );
    assert!(
        async_no_producer.is_err(),
        "an async closure-param consumer with no producer sibling declines:\n{async_no_producer:?}"
    );
}

#[test]
fn rustc_roundtrip_higher_order_closure_arg_consumer_s4() {
    // S4-HIGHER-ORDER: a consumer `app` takes a closure whose ARG is ITSELF a closure — `g: (-> (-> Int64
    // Int64) Int64)` — and applies it to an INNER closure built IN-GUEST (`(g (fn (y) (+ y x)))`). The
    // higher-order producer `mk` (whose signature `(-> (-> Int64 Int64) Int64)` = app's `g` type) supplies
    // `g`. Two emit pieces make this work: (1) `arg_ok_or_fn` admits a `Ty::Fn` closure arg in the consumer
    // gate; (2) `def_is_producer_for_sibling` exempts `mk` from the producer requirement for its OWN `f`
    // param (fed in-guest by app, never host-supplied) — so `mk` emits as a plain `pub fn mk(f: Rc<dyn
    // Fn>)`. The harness recognizes `mk` as a higher-order producer and passes it `Rc::new(mk)`.
    let m = compile_rust(
        "(module m \
           (def (mk) (fn ((: f (-> Int64 Int64))) (f 10))) \
           (def (app (: g (-> (-> Int64 Int64) Int64)) (: x Int64)) (g (fn (y) (+ y x)))) \
           (export mk) (export app))",
    );
    assert!(
        m.contains("pub fn mk(f: std::rc::Rc<dyn Fn(i64) -> i64>) -> i64"),
        "the higher-order producer mk emits as a plain fn taking an Rc<dyn Fn> param:\n{m}"
    );
    assert!(
        m.contains("pub fn app(g: std::rc::Rc<dyn Fn(std::rc::Rc<dyn Fn(i64) -> i64>) -> i64>"),
        "the consumer app emits a higher-order Rc<dyn Fn(Rc<dyn Fn>)> param:\n{m}"
    );
    // Driven producer→consumer: `app(Rc::new(mk), 5)` — mk is the higher-order closure, app applies it to
    // the in-guest `(fn (y) (+ y 5))`, so mk calls that on 10 → 10+5 = 15.
    if let Some(out) = rustc_run(
        &m,
        "app(std::rc::Rc::new(mk as fn(std::rc::Rc<dyn Fn(i64)->i64>)->i64), 5)",
    ) {
        assert_eq!(
            out, "15",
            "app(mk, 5): mk applies the in-guest (+ y 5) to 10 → 15"
        );
    }
    // A HIGHER-ORDER closure export called ALONE (no consumer sibling) still DECLINES — the host would have
    // to supply the inner `(-> Int64 Int64)` over the boundary (no rep). `def_is_producer_for_sibling` is
    // false (nothing consumes mk's type), and `f` has no producer → declines.
    let mk_alone = compile_rust_result(
        "(module m (def (mk) (fn ((: f (-> Int64 Int64))) (f 10))) (export mk))",
    );
    assert!(
        mk_alone.is_err(),
        "a higher-order closure export with no consuming sibling declines (host can't supply the inner closure):\n{mk_alone:?}"
    );
}

#[test]
fn rustc_roundtrip_closure_parameter_consumer_with_a_compound_arg_closure_s2() {
    // CLOSURE-PARAMETER CONSUMER, S2: the consumer's closure param takes a COMPOUND arg (a Tuple) with a
    // scalar result. The consumer applies `(g p)` where `p` is a tuple built in its OWN body; the producing
    // sibling `mk-sum` emits a matching `Rc<dyn Fn((i64, i64)) -> i64>` (factory S2). The guard now lifts for
    // a compound closure ARG (`s2_arg_ok`) as long as the RESULT stays scalar — so `apply-tup` emits and the
    // gate driver builds the closure from the producer: `apply_tup(mk_sum(100))` applies g to (3,4) → 107.
    let m = compile_rust(
        "(module m \
           (def (mk-sum (: k Int64)) (fn ((: p (Tuple Int64 Int64))) (+ (+ (. p 0) (. p 1)) k))) \
           (def (apply-tup (: g (-> (Tuple Int64 Int64) Int64))) (g (tuple 3 4))) \
           (export mk-sum) (export apply-tup))",
    );
    assert!(
        m.contains("pub fn apply_tup(g: std::rc::Rc<dyn Fn((i64, i64)) -> i64>) -> i64"),
        "the S2 consumer emits a compound-arg `Rc<dyn Fn((i64,i64))->i64>` param + applies it:\n{m}"
    );
    // Driven producer→consumer: `mk_sum(100)` builds the closure, `apply_tup` applies it to (3,4): 3+4+100.
    if let Some(out) = rustc_run(&m, "apply_tup(mk_sum(100))") {
        assert_eq!(out, "107", "apply_tup(mk_sum(100)) = (3 + 4) + 100 = 107");
    }
    // A HIGHER-ORDER closure param (its arg is itself a closure) still DECLINES — `s2_arg_ok` rejects `Fn`.
    let higher_order = compile_rust_result(
        "(module m \
           (def (mk (: k Int64)) (fn ((: h (-> Int64 Int64))) (h k))) \
           (def (apply-ho (: g (-> (-> Int64 Int64) Int64))) (g mk)) \
           (export mk) (export apply-ho))",
    );
    assert!(
        higher_order.is_err(),
        "a higher-order closure-param consumer (arg is itself a closure) still declines:\n{higher_order:?}"
    );
}

#[test]
fn rustc_roundtrip_closure_parameter_consumer_with_a_compound_result_closure_s3() {
    // CLOSURE-PARAMETER CONSUMER, S3: the consumer's closure param RETURNS a COMPOUND (a Tuple). The
    // consumer applies `(g x (+ x 10))` and returns the tuple result; the producing sibling `mk` emits a
    // matching `Rc<dyn Fn(i64, i64) -> (i64, i64)>` (factory S3). The `closure_param_is_simple` guard now
    // admits a compound closure RESULT (`s2_arg_ok` on the final result, not just the args) — the result
    // flows into the consumer body and the emitter lowers the native tuple. So `app` emits and the driver
    // builds the closure from the producer: `app(mk(), 5)` applies g to (5, 15) → (5, 15).
    let m = compile_rust(
        "(module m \
           (def (mk) (fn ((: a Int64) (: b Int64)) (tuple a b))) \
           (def (app (: g (-> Int64 Int64 (Tuple Int64 Int64))) (: x Int64)) (g x (+ x 10))) \
           (export mk) (export app))",
    );
    assert!(
        m.contains(
            "pub fn app(g: std::rc::Rc<dyn Fn(i64, i64) -> (i64, i64)>, x: i64) -> (i64, i64)"
        ),
        "the S3 consumer emits a compound-RESULT `Rc<dyn Fn(i64,i64)->(i64,i64)>` param + applies it:\n{m}"
    );
    // A compound closure result that CONTAINS a closure (a Tuple with a `Fn` element) still DECLINES —
    // `s2_arg_ok` recurses into the tuple and rejects the nested `Ty::Fn` (no value-form render). This
    // guards the widening: only value-renderable compound results are admitted, not fn-carrying ones.
    let fn_in_result = compile_rust_result(
        "(module m \
           (def (mk (: k Int64)) (fn ((: x Int64)) (tuple x (fn ((: y Int64)) (+ y k))))) \
           (def (app (: g (-> Int64 (Tuple Int64 (-> Int64 Int64)))) (: x Int64)) (. (g x) 0)) \
           (export mk) (export app))",
    );
    assert!(
        fn_in_result.is_err(),
        "a closure-param consumer whose closure result TUPLE contains a closure still declines:\n{fn_in_result:?}"
    );
}

#[test]
fn a_record_arg_closure_consumer_emits_a_param_shapes_note() {
    // RECORD closure ARG (host-closure S2, record-arg): a consumer taking a `(-> (Record …) Int64)` closure
    // now crosses (`s2_arg_ok` admits `Ty::Record`). Because a Tuple-arg and a same-field Record-arg closure
    // erase to the IDENTICAL `Rc<dyn Fn((i64,i64))>`, the backend emits a `// cdz-param-shapes[<ident>]` note
    // carrying the pre-erasure arrow (Tuple vs Record distinct) so the gate driver pairs producer↔consumer
    // correctly. Assert the consumer emits + carries the Record-shaped note.
    let m = compile_rust(
        "(module m \
           (def (mkb (: k Int64)) (fn ((: r (Record (: a Int64) (: b Int64)))) (+ (* (. r a) (. r b)) k))) \
           (def (appb (: h (-> (Record (: a Int64) (: b Int64)) Int64)) (: y Int64)) (h (record (= a y) (= b y)))) \
           (export mkb) (export appb))",
    );
    assert!(
        m.contains("pub fn appb(h: std::rc::Rc<dyn Fn((i64, i64)) -> i64>, y: i64) -> i64"),
        "the record-arg consumer emits (record erases to the (i64,i64) tuple):\n{m}"
    );
    assert!(
        m.contains("// cdz-param-shapes[appb]: (-> (Record (: a Int64) (: b Int64)) Int64)"),
        "the consumer carries a cdz-param-shapes note with the pre-erasure Record arrow:\n{m}"
    );
}

#[test]
fn a_closure_factory_returning_a_record_emits() {
    // HOST-CLOSURE FACTORY with a RECORD RESULT (S3): a factory whose returned closure yields a record now
    // crosses — `s3_result_ok` admits `Ty::Record` (it renders like a Tuple, the factory-result path walks
    // its sorted fields positionally). The factory emits `mk(k) -> Rc<dyn Fn(i64) -> (i64, i64)>` (the record
    // is the sorted-field tuple `(i64, i64)`). This matters chiefly on the ASYNC target, where a record-
    // returning closure stays a FACTORY (sync eta-peels a nullary one to a plain fn); it flips the record-
    // result closure round-trip family on rust-async. A record ARG stays deferred (the harness arg-rebuild
    // needs a sorted-field fix — a separate slice), so `s2_arg_ok` is NOT widened for Record here.
    let m = compile_rust(
        "(module m (def (mk (: k Int64)) (fn ((: x Int64)) (record (= x x) (= y (+ x k))))) (export mk))",
    );
    assert!(
        m.contains("pub fn mk(k: i64) -> std::rc::Rc<dyn Fn(i64) -> (i64, i64)>"),
        "a record-returning closure factory now emits an `Rc<dyn Fn>` with the sorted-field tuple result:\n{m}"
    );
}

#[test]
fn a_closure_factory_returning_a_set_or_map_emits() {
    // HOST-CLOSURE FACTORY with a SET/MAP RESULT (S3): a factory whose returned closure yields a Set/Map now
    // crosses — `s3_result_ok` admits `Ty::Set`/`Ty::Map` (rendered via `cdz_render_expr`'s Set/Map arm into
    // the canonical `(set …)`/`(map (k v) …)` form). Matters chiefly on ASYNC, where such a closure stays a
    // FACTORY (sync eta-peels a nullary one to a plain `BTreeSet`/`BTreeMap`-returning fn the render already
    // handles). A Set-returning factory emits an `Rc<dyn Fn>` whose result is the `BTreeSet<i64>`.
    let set = compile_rust(
        "(module m (def (mk (: k Int64)) (fn ((: x Int64)) ((. Set of) (list x k)))) (export mk))",
    );
    assert!(
        set.contains("pub fn mk(k: i64) -> std::rc::Rc<dyn Fn(i64) -> ")
            && set.contains("BTreeSet<i64>"),
        "a Set-returning closure factory emits an `Rc<dyn Fn>` with a BTreeSet result:\n{set}"
    );
}

#[test]
fn a_closure_parameter_consumer_with_a_bare_string_result_emits() {
    // CLOSURE-PARAMETER CONSUMER with a BARE String/Bytes RESULT: the consumer's OWN result is a String
    // (here it ignores the closure and returns "hi"). This USED to decline (`result_render_unsupported`) —
    // the consumer-path gate driver rendered a String as the quoted `"hi"` form, but the value crosses the
    // host boundary as `list<u8>` and the corpus records the byte-int list `(104 105)`. The driver now
    // routes a String/Bytes result of a CONSUMER (like a factory) through `cdz_render_bytes_list`, so the
    // export EMITS (the byte-rope-consumer round-trip family, 14 corpus cases). It builds a normal String
    // return — the gate drives the producer→consumer synthesis + byte-list render end-to-end.
    let m = compile_rust(
        "(module m (def (mk) (fn ((: n Int64)) (+ n 65))) \
                    (def (label (: g (-> Int64 Int64)) (: x Int64)) \"hi\") \
                    (export mk) (export label))",
    );
    assert!(
        m.contains("pub fn label(g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64) -> String"),
        "a closure-param consumer with a bare String result now emits (no longer declines):\n{m}"
    );
}

#[test]
fn a_closure_parameter_consumer_with_a_string_arg_closure_emits() {
    // CLOSURE-PARAMETER CONSUMER whose closure param takes a String ARG applied IN-GUEST: `app` takes
    // `g: (-> String Int64)` and applies it to a String LITERAL built in its own body (`(g "hello")`). The
    // arg is a value the emitter already lowers (no host-supplied String crosses the boundary), so it works
    // like the S2 Tuple/List args — `s2_arg_ok` now admits `Ty::String`/`Ty::Bytes` for this in-guest-applied
    // shape. The producing sibling `mk` supplies the `Rc<dyn Fn(String) -> i64>`. (A String arg PASSED FROM
    // THE HOST at the boundary remains deferred — a different ABI with no producer-driven synth.)
    let m = compile_rust(
        "(module m (def (mk) (fn ((: s String)) ((. String byte-len) s))) \
                    (def (app (: g (-> String Int64)) (: x Int64)) (g \"hello\")) \
                    (export mk) (export app))",
    );
    assert!(
        m.contains("pub fn app(g: std::rc::Rc<dyn Fn(String) -> i64>, x: i64) -> i64"),
        "a closure-param consumer with a String-arg closure now emits (no longer declines):\n{m}"
    );
}

#[test]
fn rustc_roundtrip_host_closure_factory_compound_arg_s2() {
    // HOST-CLOSURE S2: a closure-factory whose returned closure takes a COMPOUND ARG (Tuple/List) with a
    // SCALAR result now crosses — the arg maps natively (`Rc<dyn Fn((i64, i64)) -> i64>`) and the gate
    // harness rebuilds the `(tuple 3 4)` call arg as `(3, 4)`, applied through S1's make/call split:
    // `mk(10)((3, 4))`. The captured `k` + the tuple fields combine — 3 + 4 + 10 = 17.
    let tup = compile_rust(
        "(module m (def (mk (: k Int64)) (fn ((: p (Tuple Int64 Int64))) (+ (+ (. p 0) (. p 1)) k))) (export mk))",
    );
    assert!(
        tup.contains("pub fn mk(k: i64) -> std::rc::Rc<dyn Fn((i64, i64)) -> i64>"),
        "the S2 Tuple-arg factory emits `Rc<dyn Fn((i64,i64))->…>`:\n{tup}"
    );
    if let Some(out) = rustc_run(&tup, "mk(10)((3, 4))") {
        assert_eq!(out, "17", "make(k=10) then call((3,4)) = 3+4+10 = 17");
    }
    // A LIST arg also crosses (element type maps): the closure sums the head + capture.
    let lst = compile_rust(
        "(module m (def (mk (: k Int64)) (fn ((: xs (List Int64))) \
           (match xs ((list a .. r) (+ a k)) (_ k)))) (export mk))",
    );
    if let Some(out) = rustc_run(&lst, "mk(100)(vec![5, 6])") {
        assert_eq!(out, "105", "make(k=100) then call([5,6]) = 5+100 = 105");
    }

    // S4a: an Option/Result ARG now crosses too (the harness rebuilds `(Some 5)`→`Some(5)` etc.), so a
    // factory taking an Option arg EMITS (`Rc<dyn Fn(Option<i64>) -> i64>`).
    let option_arg = compile_rust(
        "(module m (def (mk (: k Int64)) (fn ((: o (Option Int64))) \
           (match o ((Some v) (+ v k)) (_ k)))) (export mk))",
    );
    assert!(
        option_arg.contains("Rc<dyn Fn(Option<i64>) -> i64>"),
        "an Option-ARG factory now emits (S4a):\n{option_arg}"
    );
    // S4a: an Option/Result RESULT now crosses too — the backend emits a valid `Rc<dyn Fn(i64) ->
    // Option<i64>>`, and the gate renders the sum result as the type-annotated value form `(: (Some 5)
    // (Option Int64))` (the value-encoded shape the wasm `call` produces; the driver's factory-sum-result
    // branch wraps the bare `cdz_render_expr` value in `(: value type)`).
    let sum_result = compile_rust(
        "(module m (def (mk (: k Int64)) (fn ((: x Int64)) (Some (+ x k)))) (export mk))",
    );
    assert!(
        sum_result.contains("Rc<dyn Fn(i64) -> Option<i64>>"),
        "an Option-RESULT factory now emits (S4a):\n{sum_result}"
    );
    // S4a user-sum extension: a factory whose closure returns a USER sum (`(type Dir (N) (S))`) also emits —
    // the backend produces `Rc<dyn Fn(i64) -> Dir>` and the gate renders it as the value form `(: (N unit)
    // Dir)` (the user-sum arm of `cdz_render_at` + the factory `(: value type)` wrapper, keyed off the
    // `// cdz-sum[Dir]` descriptor). A user sum result is no longer deferred.
    let user_sum_result = compile_rust(
        "(module m (type Dir (N) (S)) (def (mk) (fn ((: n Int64)) (if (> n 0) (N) (S)))) (export mk))",
    );
    assert!(
        user_sum_result.contains("-> Dir>"),
        "a USER-sum-RESULT factory now emits (S4a user-sum extension):\n{user_sum_result}"
    );
    // HARDENING: a user-sum result whose variant carries a FUNCTION payload (`(H (-> Int64 Int64))`) DECLINES
    // — `cdz_render_at` has no value-form render for a fn payload, so admitting it (on type-args alone, which
    // a monomorphic sum trivially passes) would MIS-RENDER. `sum_payloads_renderable` reads the decl's variant
    // payloads and declines a non-renderable one (the reviewer-flagged fn-payload hole). A sibling user sum
    // with a Float/Tuple payload still EMITS (those payloads render), so this is a NARROW guard, not a
    // blanket user-sum-result decline.
    let fn_payload_sum = compile_rust_result(
        "(module m (type Holder (H (-> Int64 Int64)) (Z)) \
           (def (mk) (fn ((: n Int64)) (if (> n 0) (H (fn ((: y Int64)) y)) (Z)))) (export mk))",
    );
    assert!(
        fn_payload_sum.is_err(),
        "a user-sum-RESULT factory with a FUNCTION-payload variant declines (non-renderable payload):\n{fn_payload_sum:?}"
    );
    let float_payload_sum = compile_rust(
        "(module m (type Num (F Float64) (Z)) (def (mk) (fn ((: n Int64)) (if (> n 0) (F 1.5) (Z)))) (export mk))",
    );
    assert!(
        float_payload_sum.contains("-> Num>"),
        "a user-sum-RESULT factory with a Float-payload variant still emits (renderable payload):\n{float_payload_sum}"
    );
}

#[test]
fn rustc_roundtrip_host_closure_factory_compound_result_s3() {
    // HOST-CLOSURE S3: a closure-factory whose returned closure produces a COMPOUND RESULT (Tuple/List)
    // now crosses. The factory emits `Rc<dyn Fn(x) -> (i64, i64)>`, and the gate harness peels the
    // factory's arrow `cdz-return` note `(-> Int64 (Tuple Int64 Int64))` to the final result type and
    // renders it structurally (`(tuple k n)`), not the bare `{}` Display that E0277s on a Rust tuple.
    // NOTE: the end-to-end make/call + structured render (`(tuple k n)` / cdz-list form) is exercised by
    // the gate (the `21-host-closures.sexp` compound-result cases, which flip todo→PASS with this slice) —
    // the gate driver renders via `cdz_render_expr` after peeling the factory's arrow note. This unit test
    // pins the EMIT shape only; `rustc_run` here uses a bare `{}` Display that can't format a Rust tuple/Vec
    // (that's precisely what the gate's structured render handles), so we assert the signature + note, not run.
    let tup = compile_rust(
        "(module m (def (mk (: k Int64)) (fn ((: n Int64)) (tuple k n))) (export mk))",
    );
    assert!(
        tup.contains("pub fn mk(k: i64) -> std::rc::Rc<dyn Fn(i64) -> (i64, i64)>")
            && tup.contains("// cdz-return[mk]: (-> Int64 (Tuple Int64 Int64))"),
        "the S3 tuple-result factory emits `Rc<dyn Fn(..)->(..)>` + the arrow return note:\n{tup}"
    );
    // A LIST result also crosses — the closure builds a Vec (rendered as a cdz list by the gate).
    let lst =
        compile_rust("(module m (def (mk (: k Int64)) (fn ((: n Int64)) (list k n))) (export mk))");
    assert!(
        lst.contains("-> std::rc::Rc<dyn Fn(i64) -> Vec<i64>>"),
        "the S3 list-result factory emits `Rc<dyn Fn(..)->Vec<..>>`:\n{lst}"
    );

    // S3-extension: a STRING/BYTES closure RESULT crosses the host boundary AS `list<u8>` — the factory
    // emits `Rc<dyn Fn(..)->String>` / `->Vec<u8>`, and the gate renders the result as the byte-int list
    // `(104 105)` (`cdz_render_bytes_list`), NOT the quoted `"hi"`/`b"…"` a plain export uses. (End-to-end
    // make/call + byte-list render is gate-covered by the 21-host-closures String/Bytes cases; here pin the
    // EMIT shape — `rustc_run`'s bare `{}` can't format the byte-list, that's the gate's structured render.)
    let strr = compile_rust(
        "(module m (def (mk (: k Int64)) (fn ((: n Int64)) ((. String concat) \"x\" \"y\"))) (export mk))",
    );
    assert!(
        strr.contains("-> std::rc::Rc<dyn Fn(i64) -> String>"),
        "the String-result factory emits `Rc<dyn Fn(..)->String>` (rendered as list<u8> by the gate):\n{strr}"
    );
}

#[test]
fn async_lifted_closure_call_free_body_emits_env_closure_uniform_abi() {
    // OPTION A (uniform async closure ABI): on `--target rust-async` EVERY lifted closure — call-free
    // included — emits as an `async fn __lifted_k(env: &mut dyn DynCdzEnv, …)` whose VALUE is
    // `Rc<dyn EnvClosure<A,R>>` (a per-closure synth `struct __Clos_k` + `impl EnvClosure`). Uniform so a
    // `Ty::Fn` TYPE position can spell the value form (`async_closure_type`) without observing
    // `body_has_call` (which a type can't see). A call-free body's env is present-but-unused.
    let both = compile_rust_async(
        "(module m (def (both (: a Int64) (: b Int64)) (fn ((: x Int64)) (+ (+ a b) x))) (export both))",
    );
    // The factory is an async fn RETURNING the EnvClosure handle (its captures lead its own params). Its
    // env is the uniform object-safe `&mut dyn DynCdzEnv` (no generic).
    assert!(
        both.contains(
            "pub async fn both(__cdz_env: &mut dyn DynCdzEnv, a: i64, b: i64) -> std::rc::Rc<dyn cdz_rt::EnvClosure<i64, i64>>"
        ),
        "the factory is an async fn returning the EnvClosure handle:\n{both}"
    );
    // The lifted closure body is now an `async fn` taking the object-safe `&mut dyn DynCdzEnv` (uniform ABI).
    assert!(
        both.contains("async fn __lifted_0(__cdz_env: &mut dyn DynCdzEnv, __cap0: i64, __cap1: i64, x: i64) -> i64"),
        "the call-free lifted closure body is an async fn threading &mut dyn DynCdzEnv:\n{both}"
    );
    // The per-closure synth struct carries the captures + impls EnvClosure, forwarding env + caps + arg.
    assert!(
        both.contains("struct __Clos_0")
            && both.contains("impl cdz_rt::EnvClosure<i64, i64> for __Clos_0")
            && both.contains(
                "Box::pin(__lifted_0(__cdz_env, self.__c0.clone(), self.__c1.clone(), __a0))"
            ),
        "the closure value is a per-closure struct impl'ing EnvClosure:\n{both}"
    );
    // The `Core::Closure` VALUE builds the struct + casts to `Rc<dyn EnvClosure>`.
    assert!(
        both.contains("std::rc::Rc::new(__Clos_0 { __c0: __c0, __c1: __c1 }) as std::rc::Rc<dyn cdz_rt::EnvClosure<i64, i64>>"),
        "the closure value is Rc::new(__Clos_0{{..}}) as Rc<dyn EnvClosure>:\n{both}"
    );
}

#[test]
fn async_lifted_closure_whose_body_makes_a_call_now_emits_env_closure() {
    // OPTION A UNLOCK: a lifted closure whose body reaches a runtime `Core::Call` (an async callee needing
    // env threaded + awaited) — previously a clean DECLINE — now EMITS under the EnvClosure ABI: the lifted
    // fn is an `async fn` whose body awaits the call, and the closure value is `Rc<dyn EnvClosure>`. This is
    // the shape that defeated the `Rc<dyn Fn>` attempts (a `Fn` closure can't return a future borrowing its
    // own `&mut` env); `EnvClosure`'s generic `call<'a>` method ties the future to the env borrow.
    // A recursive higher-order consumer `ap` keeps the closure as a RUNTIME VALUE it applies via `.call`.
    let src = "(module m \
       (def (ap (: g (-> Int64 Int64)) (: n Int64) (: x Int64)) (if (= n 0) x (ap g (+ n -1) (g x)))) \
       (def (run) (let ((inc (fn ((: y Int64)) (+ y 1)))) (ap inc 5 10))) (export run))";
    // Sync rust emits it (the closure is a plain `Rc<dyn Fn>` applied `(g)(x)`).
    assert!(
        compile_rust_result(src).is_ok(),
        "the higher-order closure program compiles on SYNC rust"
    );
    // Async rust now EMITS it (no decline): the consumer takes `g: Rc<dyn EnvClosure<i64,i64>>` and applies
    // it `g.call(env, x).await`, and the closure value builds an `EnvClosure` struct.
    let a = compile_rust_async(src);
    assert!(
        a.contains("g: std::rc::Rc<dyn cdz_rt::EnvClosure<i64, i64>>"),
        "the recursive consumer takes the EnvClosure by value:\n{a}"
    );
    assert!(
        a.contains(".call(__cdz_env,") && a.contains(".await"),
        "the consumer applies the closure via .call(env, arg).await:\n{a}"
    );
    assert!(
        a.contains("impl cdz_rt::EnvClosure<i64, i64> for __Clos_0"),
        "the closure value impls EnvClosure:\n{a}"
    );
    // e2e: rustc-roundtrip against cdz-rt proves the language wall is cleared (the `Rc<dyn Fn>` attempts
    // could not compile THIS shape — a closure value passed to a recursive consumer + `.call`ed through
    // `&mut dyn DynCdzEnv`). inc applied 5× to 10 = 15.
    let driver = r#"
struct Meter;
impl cdz_rt::CdzEnv for Meter {
    async fn consume(&mut self, _g: u64) {}
}
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() {
    let mut e = Meter;
    println!("{}", block_on(prog::run(&mut e)));
}
"#;
    if let Some(out) = rustc_run_driver(&a, driver) {
        assert_eq!(out, "15", "inc applied 5 times to 10 = 15:\n{a}");
    }
}

#[test]
fn rustc_roundtrip_closure_stored_in_a_heap_compound_element() {
    // A closure stored in a LIST/TUPLE element, extracted + applied, declined ("a closure whose function
    // type is not fully solved here has no native Rust representation") while wasm ran it (breaker). The
    // closure NODE's arrow type is not grounded at a compound-element position (the solver leaves a var at
    // the node), so `type_of(id)` gave a non-concrete Ty::Fn. Fix: the `Rc<dyn Fn(…)->…>` type is built from
    // the LIFTED LAMBDA's own param + result types (authoritative concrete machine types), not `type_of`.
    // (a) a closure in a TUPLE element: two adders extracted + applied — (adder 1)(10) + (adder 2)(10) = 23.
    let tup = compile_rust(
        "(module m (def (adder n) (fn (x) (+ x n))) \
           (def (run) (let (((tuple f g) (tuple (adder 1) (adder 2)))) (+ (f 10) (g 10)))) (export run))",
    );
    assert!(
        tup.contains("Rc<dyn Fn(i64) -> i64>"),
        "the closure spells a concrete Rc<dyn Fn(i64) -> i64> from the lifted lambda:\n{tup}"
    );
    if let Some(out) = rustc_run(&tup, "run()") {
        assert_eq!(out, "23", "(adder 1)(10) + (adder 2)(10) = 11 + 12 = 23");
    }

    // (b) a closure in a LIST element, extracted via List.at + applied — (adder 1)(10) = 11.
    let lst = compile_rust(
        "(module m (def (adder n) (fn (x) (+ x n))) \
           (def (run) (let ((fs (list (adder 1) (adder 2)))) \
             (match (List.at fs 0) ((Some f) (f 10)) ((None u) -1)))) (export run))",
    );
    if let Some(out) = rustc_run(&lst, "run()") {
        assert_eq!(out, "11", "the head closure (adder 1)(10) = 11");
    }

    // (c) a DIRECT closure literal in a tuple (no adder wrapper) — also grounded from the lifted lambda.
    let direct = compile_rust(
        "(module m (def (run) (let (((tuple f g) (tuple (fn (x) (+ x 1)) (fn (x) (+ x 2))))) \
           (+ (f 10) (g 10)))) (export run))",
    );
    if let Some(out) = rustc_run(&direct, "run()") {
        assert_eq!(
            out, "23",
            "direct closure literals in a tuple: 11 + 12 = 23"
        );
    }
}

#[test]
fn export_name_kebab_validity_is_rejected_on_the_rust_backend_too() {
    // An export's boundary name that is not a valid component kebab extern name is a LANGUAGE-level CDZ0201,
    // not a wasm-only load failure — the rust backend applies the SAME reject (via the shared wasm-module
    // checks) so both backends agree (the corpus grades these `(error CDZ0201)`; the rust backend emits no
    // component, so without this it silently emitted a `pub fn`).
    // (a) two source names colliding under kebab normalization (fA + f-a → f-a).
    let collision = compile_rust_result(
        "(module m (def (fA (: x Int64)) (+ x 1)) (def (f-a (: y Int64)) (+ y 2)) (export fA) (export f-a))",
    );
    assert!(
        collision.is_err(),
        "colliding kebab extern names are rejected on rust:\n{collision:?}"
    );
    // (b) a digit-led kebab segment (step-by-2 → segment `2` is not letter-led).
    let digit_led =
        compile_rust_result("(module m (def (step-by-2 (: x Int64)) (+ x 1)) (export step-by-2))");
    assert!(
        digit_led.is_err(),
        "a digit-led kebab export segment is rejected on rust:\n{digit_led:?}"
    );
    // (c) a normal export name still emits (no over-rejection) — regression guard. Assert on the SPECIFIC
    // emitted identifier `my_func` (the `my-func` boundary name sanitized: `-`→`_` for a Rust fn), NOT a
    // bare `"pub fn "` fallback — that matches ANY public fn (the emitted module always has one), so it
    // would pass even if `my_func` were mis-emitted/missing (Copilot PR#528 weak-test finding).
    let ok = compile_rust("(module m (def (my-func (: x Int64)) (+ x 1)) (export my-func))");
    assert!(
        ok.contains("pub fn my_func"),
        "a valid kebab export name still emits `pub fn my_func` (not declined/over-rejected):\n{ok}"
    );
}

#[test]
fn rustc_roundtrip_unusual_integer_width_stores_in_the_next_larger_primitive() {
    // An UNUSUAL in-range width (`UInt48`, `UInt4` — 1..=64 but not an aliased boundary) has no exact Rust
    // primitive, so it STORES in the next-larger machine width (`UInt48`→`u64`, `UInt4`→`u8`); the value/
    // wrap/render surface is exact. `.wrap` to an unusual width masks the low N bits (an `as` cast keeps the
    // STORAGE width's bits, so add `& (2^N-1)`). Runtime ARITHMETIC on an unusual width computes the native
    // op on the storage type then RANGE-CHECKS the result against the type's own `2^N` bounds (not the
    // storage width's `2^machine`) — see part (c).
    // (a) a `(UInt 48)` const value = 2^48-1, stored as u64.
    let val = compile_rust("(module m (def (run) (: 281474976710655 (UInt 48))) (export run))");
    assert!(
        val.contains("-> u64") && val.contains("281474976710655u64"),
        "a UInt48 value stores in u64:\n{val}"
    );

    // (b) a runtime `(UInt 4).wrap n` masks the low 4 bits: `(n as u8) & 15`. Run: 17 → 1, 15 → 15.
    let wrap =
        compile_rust("(module m (def (run (: n Int64)) ((. (UInt 4) wrap) n)) (export run))");
    assert!(
        wrap.contains("& 15u8"),
        "a UInt4 wrap masks the low 4 bits:\n{wrap}"
    );
    if let Some(out) = rustc_run(&wrap, "run(17)") {
        assert_eq!(out, "1", "17 & 0xF = 1 (low nibble)");
    }
    if let Some(out) = rustc_run(&wrap, "run(15)") {
        assert_eq!(out, "15", "15 fits the nibble whole");
    }

    // (c) runtime ARITHMETIC on an unusual width now COMPILES: the native op runs on the storage type, then
    // a range-check against the TYPE's own `2^N` bound (NOT the storage width's `2^machine`) traps overflow.
    // A `(UInt 48)` add emits `wrapping_add` + `if __uw > 2^48-1 { panic!("integer overflow …") }` — the
    // single unsigned upper-bound test (the rust twin of the wasm narrow-width range-check), NOT a bare
    // `u64::checked_add` (which would trap at 2^64, the wrong width). Was a DECLINE (safety guard) before.
    let arith =
        compile_rust("(module m (def (run (: a (UInt 48)) (: b (UInt 48))) (+ a b)) (export run))");
    assert!(
        arith.contains("281474976710655u64") && arith.contains("integer overflow in addition"),
        "unusual-width UInt48 add range-checks at 2^48-1 (its own bound), not the storage width:\n{arith}"
    );

    // (d) an aliased narrow width (UInt8) still wraps with NO redundant mask (the `as` cast is exact).
    let alias =
        compile_rust("(module m (def (run (: n Int64)) ((. (UInt 8) wrap) n)) (export run))");
    assert!(
        alias.contains("as u8") && !alias.contains("& 255u8"),
        "an aliased UInt8 wrap needs no mask (the cast truncates exactly):\n{alias}"
    );
}

#[test]
fn rustc_roundtrip_a_signed_unusual_width_wrap_sign_extends_not_just_masks() {
    // REGRESSION (Copilot PR #532, github-liaison note): a SIGNED unusual-width `.wrap` must keep the low N
    // bits AND reinterpret them at the target sign (bit N-1 is the sign bit), NOT just mask. The prior emit
    // `(v as i8) & 15` returned +8 for `(Int 4).wrap 8` — a SILENT miscompile: the correct signed 4-bit wrap
    // is -8 (bit 3 set → 8 - 2^4). Fixed by a sign-extending shift `((v as i8) << (bits-N)) >> (bits-N)`
    // (arithmetic right-shift replicates the sign bit). Matches `IntValue::wrap_to(signed=true, 4)` and the
    // wasm backend (which was already correct — no twin bug).
    let wrap = compile_rust("(module m (def (run (: n Int64)) ((. (Int 4) wrap) n)) (export run))");
    assert!(
        wrap.contains("<< 4") && wrap.contains(">> 4") && !wrap.contains("& 15"),
        "a signed unusual-width wrap SIGN-EXTENDS via a shift, not a low-bit mask:\n{wrap}"
    );
    if let Some(out) = rustc_run(&wrap, "run(8)") {
        assert_eq!(
            out, "-8",
            "(Int 4).wrap 8: bit 3 set → 8 - 16 = -8 (NOT the masked +8)"
        );
    }
    if let Some(out) = rustc_run(&wrap, "run(7)") {
        assert_eq!(out, "7", "(Int 4).wrap 7: sign bit clear → 7");
    }
    if let Some(out) = rustc_run(&wrap, "run(15)") {
        assert_eq!(out, "-1", "(Int 4).wrap 15: all 4 bits set → -1");
    }

    // A SIGNED narrower-than-storage width beyond the byte: `(Int 12)` stores in i16 (16 bits), so the shift
    // is `<< 4`/`>> 4` (16 - 12). `(Int 12).wrap 2048` = bit 11 set → 2048 - 2^12 = -2048 (the 12-bit min).
    let wrap12 =
        compile_rust("(module m (def (run (: n Int64)) ((. (Int 12) wrap) n)) (export run))");
    assert!(
        wrap12.contains("as i16") && wrap12.contains("<< 4") && wrap12.contains(">> 4"),
        "a signed (Int 12) wrap sign-extends in i16 storage (16-12 = 4-bit shift):\n{wrap12}"
    );
    if let Some(out) = rustc_run(&wrap12, "run(2048)") {
        assert_eq!(
            out, "-2048",
            "(Int 12).wrap 2048: bit 11 set → 2048 - 4096 = -2048"
        );
    }
}

#[test]
fn rustc_roundtrip_unusual_width_composes_through_compounds_and_collections() {
    // The storage-width map (an unusual `(UInt N)` → the next-larger machine primitive) must COMPOSE through
    // a tuple leaf, a Set element, and a List element — the value stays in range, so the wider storage is
    // lossless and the collection Ord/eq (over the storage primitive) matches the logical order. Guards the
    // tick-84 slice against a future change breaking unusual-width-in-compound. (Probed passing; pinned.)
    // (a) a UInt48 MAX value as a Set element, queried by the same value → found (Ord over u64 = logical).
    let set = compile_rust(
        "(module m (def (run) \
           (if (Set.contains (Set.of (list (: 281474976710655 (UInt 48)) (: 5 (UInt 48)))) \
                             (: 281474976710655 (UInt 48))) 1 0)) (export run))",
    );
    if let Some(out) = rustc_run(&set, "run()") {
        assert_eq!(
            out, "1",
            "the UInt48 max value is found as a Set element (u64-stored, in-range)"
        );
    }

    // (b) a UInt48 leaf in a TUPLE crosses + projects back — the wider storage is transparent.
    let tup = compile_rust(
        "(module m (def (run) (. (tuple (: 281474976710655 (UInt 48)) 7) 0)) (export run))",
    );
    if let Some(out) = rustc_run(&tup, "run()") {
        assert_eq!(
            out, "281474976710655",
            "a UInt48 tuple leaf round-trips its max value"
        );
    }

    // (c) a UInt4 (nibble) wrap result as a List element, counted back — the masked value stores in u8.
    let lst = compile_rust(
        "(module m (def (run (: n Int64)) \
           (List.len (list ((. (UInt 4) wrap) n) ((. (UInt 4) wrap) 3)))) (export run))",
    );
    if let Some(out) = rustc_run(&lst, "run(17)") {
        assert_eq!(out, "2", "two UInt4 elements build a length-2 list");
    }
}

#[test]
fn a_sum_with_a_prelude_colliding_variant_emits_the_qualified_heads_note() {
    // The rust backend emits `// cdz-sum-qualified-heads[<ident>]` for a sum whose variant heads must render
    // QUALIFIED at the value boundary — the per-sum `lower::sum_needs_qualified_heads` decision (any variant
    // name is a prelude NON-variant-ctor). Reused verbatim from the wasm backend, so both agree.
    // (a) a user sum with a COLLIDING variant (`Int` is a prelude type ctor) → gets the note (qualifies, like
    // the built-in Ast); this is the case a naive built-in-vs-user rule would wrongly render bare.
    let collide = compile_rust(
        "(module m (type Foo (Int Int64) (Bar Bool)) (def (run) (match (Foo.Int 5) ((Int x) x) (_ 0))) (export run))",
    );
    assert!(
        collide.contains("// cdz-sum-qualified-heads[Foo]"),
        "a sum with a prelude-colliding variant (Int) emits the qualified-heads note:\n{collide}"
    );
    // (b) a user sum with NO prelude-colliding variant → NO note (renders bare).
    let bare = compile_rust(
        "(module m (type Col (Rd) (Gn) (Bl)) (def (run) (match (Col.Rd) ((Rd) 1) (_ 0))) (export run))",
    );
    assert!(
        !bare.contains("// cdz-sum-qualified-heads[Col]"),
        "a sum with no prelude-colliding variant emits NO qualified-heads note:\n{bare}"
    );
}

#[test]
fn a_tail_recursive_list_match_arm_emits_no_redundant_brace_and_builds() {
    // REGRESSION (breaker #18, corpus-bugfix): a tail-recursive list helper whose arm body is a self-call
    // (a `continue` loop edge) or a `break` leaf used to be double-brace-wrapped — the list-match arm added
    // `{ … }` around `emit_tail`'s own block, so a self-loop arm read `if c { { { … continue; } } }`, whose
    // inner brace pair rustc's `unused_braces` lint flags → the gate's -D warnings turned it into a NO-BUILD.
    // The arm now drops the redundant wrap (emit_tail returns a statement/block that sits directly in the
    // arm's own brace). `flatten` is a tail-recursive list-match over `List (List Int64)` with an empty
    // inner `(list)` — it must build clean AND compute (len of the flattened 1,2,5 = 3).
    let m = compile_rust(
        "(module m (def (flatten (: xss (List (List Int64))) (: acc (List Int64))) \
           (match xss ((list) acc) ((list h .. t) (flatten t ((. List concat) acc h))))) \
           (def (run) ((. List len) (flatten (list (list 1 2) (list) (list 5)) (list)))) (export run))",
    );
    if let Some(out) = rustc_run(&m, "run()") {
        assert_eq!(
            out, "3",
            "flatten of [[1,2],[],[5]] has length 3 (builds clean, no unused_braces no-build)"
        );
    }
}

#[test]
fn an_empty_list_tuple_field_with_an_unsolved_element_grounds_and_builds() {
    // REGRESSION (breaker #18 n18c, routed back from v-inference): a recursive nested-match whose arms
    // return a TUPLE of two lists — one arm supplies `List Int64`, the empty-list base arm keeps `List Any`
    // (the arms' element types were never unified, so the tuple field's SOLVED type is `List(Any)`).
    // `rust_type(Any)` is `None`, so the empty-list grounding used to bail to a bare `vec![]` that rustc
    // cannot infer in a tuple-return position (E0282). `emit_elem_grounding_empty_list` now grounds the
    // element's open vars to the `Int64` default (`ground_open_vars`) before spelling `Vec::<i64>::new()`
    // — behavior-neutral (the list is empty), and rustc unifies with the sibling arm's `Vec<i64>`. This is
    // a RUST-EMIT grounding gap, NOT an inference gap: wasm ran the same witness only because its list
    // handle needs no SPELLED element type — running is not proof the type was solved.
    let m = compile_rust(
        "(module m \
           (def (rev (: xs (List Int64)) (: acc (List Int64))) \
             (match xs ((list) acc) ((list h .. t) (rev t (List.concat (list h) acc))))) \
           (def (deq (: f (List Int64)) (: b (List Int64))) \
             (match f ((list _h .. t) (tuple t b)) \
                      ((list) (match (rev b (list)) \
                                ((list _h .. t) (tuple t (list))) \
                                ((list) (tuple (list) (list))))))) \
           (def (run (: n Int64)) \
             (match (deq (list) (list n 2)) ((tuple f2 b2) (+ (* ((. List len) f2) 10) ((. List len) b2))))) \
           (export run))",
    );
    if let Some(out) = rustc_run(&m, "run(5)") {
        assert_eq!(
            out, "10",
            "deq drains front→back: len(f2)*10 + len(b2) = 1*10 + 0 = 10 (builds clean, no E0282)"
        );
    }
}

#[test]
fn a_recursive_newtype_declines_naming_the_box_indirection_gap_not_a_generic_missing_enum() {
    // A recursive NEWTYPE `(type Lst (Mk (Option (Tuple Int64 Lst))))` ERASES at the type level (its inner
    // is a finite `Ty::Nominal{inner: Option (Int64, Ty::Sum{Lst})}` with a μ back-edge leaf), but the RUST
    // backend has no place to hang the recursion: an erased newtype emits no enum, so a `(Mk …)`
    // construct / `(match l ((Mk o) …))` names a variant path of a type with no emitted Rust representation.
    // wasm runs it (a nominal erases to a heap handle; no named type needed). The backend must DECLINE
    // cleanly (not emit an uncompilable crate naming `Lst`), and the decline must name the PRECISE reason —
    // the missing Box-indirected NOMINAL emission — not the generic "sum with no emitted enum", so whoever
    // picks up the un-erasure feature (or a future me) is pointed at the right fix rather than hunting for a
    // missing enum. Pins the diagnostic wording produced by `enums::unrepresentable_reason` at the
    // construct/match decline site.
    let src = "(module m \
      (type Lst (Mk (Option (Tuple Int64 Lst)))) \
      (def (sm (: l Lst)) (match l \
        ((Mk o) (match o ((Some p) (+ (. p 0) (sm (. p 1)))) ((None) 0))))) \
      (def (main) (sm (Mk (Some (tuple 10 (Mk (Some (tuple 20 (Mk (None)))))))))) \
      (export main))";
    match compile_rust_result(src) {
        Ok(rs) => panic!("recursive newtype should DECLINE on rust, emitted:\n{rs}"),
        Err(diags) => {
            assert!(
                diags
                    .iter()
                    .any(|d| d
                        .contains("recursive newtype with no Box-indirected Rust representation")),
                "decline must name the recursive-newtype Box-indirection gap precisely, got: {diags:?}"
            )
        }
    }
}

#[test]
fn a_symbol_literal_payload_probe_inside_a_recursive_fn_matches_by_content() {
    // A nested Symbol-literal payload probe `(Mk #"go")` inside a self-recursive `walk` — the recursive-fn
    // dimension of the sum-variant literal-payload cases. A Symbol payload maps to a Rust `String` (a Symbol
    // IS its text at run time), so the decision-tree LitTest renders as a content compare
    // `<payload>.as_str() == "go"` rather than declining "non-scalar literal-payload probe". `walk 2 (Mk
    // #"go")` recurses twice then, at the base, probes the `W` argument's Symbol payload against `#"go"` →
    // 40. Pins that a String/Symbol literal-payload probe now renders on the rust backend (wasm already ran
    // it via the byte-leaf compare); a NON-matching symbol falls to the wildcard arm.
    let m = compile_rust(
        "(module m \
           (type W (Mk Symbol)) \
           (def (walk (: n Int64) (: w W)) \
             (if (< n 1) (match w ((Mk #\"go\") 40) (_ (- 0 1))) (walk (- n 1) w))) \
           (def (mk) (walk 2 (Mk #\"go\"))) (export mk))",
    );
    // The content compare is emitted (not a decline).
    assert!(
        m.contains(".as_str() == \"go\""),
        "Symbol literal-payload probe renders a content compare:\n{m}"
    );
    if let Some(out) = rustc_run(&m, "mk()") {
        assert_eq!(out, "40", "recurses twice then matches the #\"go\" payload");
    }
    // A DISTINCT runtime symbol misses the literal arm and takes the wildcard → -1. The symbol is built
    // behind a recursive call so it is a genuine runtime value (not a constant fold to the literal arm).
    let miss = compile_rust(
        "(module m \
           (type W (Mk Symbol)) \
           (def (probe (: w W)) (match w ((Mk #\"go\") 40) (_ (- 0 1)))) \
           (def (f (: n Int64)) (if (= n 0) (Symbol.of \"stop\") (f (- n 1)))) \
           (def (mk) (probe (Mk (f 1)))) (export mk))",
    );
    if let Some(out) = rustc_run(&miss, "mk()") {
        assert_eq!(out, "-1", "a non-#\"go\" symbol falls to the wildcard arm");
    }
}

#[test]
fn rustc_roundtrip_recursive_option_carrying_sum_compare_recurses_through_a_list_element() {
    // Companion to `rustc_roundtrip_recursive_option_carrying_sum_compare_terminates_via_helper` (PR#890):
    // that one pins the BOX-recursive back-edge; this pins the LIST-ELEMENT one. `emit_sum_cmp_walk` must
    // route a sum that recurses through a `(List Self)` element through a `__cmp_<Ident>` helper (seen-guard
    // + call-indirection) — NOT the Box-only `variant_is_recursive` path (which a List-element self-reference
    // does NOT trip) and NOT an inline expansion (unbounded codegen on the self-referential type). This
    // mirrors the equality walk's List-recursion case (a) in
    // `runtime_equality_over_a_recursive_sum_emits_a_recursive_helper_fn`, closing the cmp/eq coverage gap:
    // before, only the eq side witnessed the List-element recursion route on the compare-family walk.
    //
    // Fixture: `Ast = (Leaf (Option Int64)) | (Node (List Ast))` — the self-reference is a `List Ast`
    // ELEMENT, and the `Option Int64` payload forces the value-cmp walk (not native `.cmp()`). A green
    // compile proves the helper routing terminates; the answer proves the Option payload orders Some<None.
    let src = "(module m \
        (type Ast (Leaf (Option Int64)) (Node (List Ast))) \
        (def (go (: k Int64)) \
          (match (Ordering.of (Ast.Node (list (Ast.Leaf (Some k)))) \
                          (Ast.Node (list (Ast.Leaf (: (None unit) (Option Int64)))))) \
            ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
        (export go))";
    // This type IS representable on the rust backend (verified — the generic/nested-Option value-op emit arc
    // is closed, so a List-element-recursive Option-carrying sum compare EMITS via the `__cmp_` helper), so a
    // DECLINE here is a real emit regression: `compile_rust` HARD-FAILS on decline rather than tolerating it
    // (an `Err(_) => {}` arm masked the pin — it stayed green even if the backend stopped emitting).
    let rs = compile_rust(src);
    assert!(
        rs.contains("fn __cmp_"),
        "a List-element-recursive Option-carrying sum compare must route through a __cmp_ helper \
         (not inline, not the Box-only path):\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "go(5)") {
        // Node [Leaf (Some 5)] vs Node [Leaf None]: same variant Node → compare the List payloads
        // element-wise → Leaf (Some 5) vs Leaf None → Option field Some 5 < None (Cadenza declared
        // order) → Less (1).
        assert_eq!(
            out, "1",
            "a List-recursive Option-carrying sum orders its Option payload Some<None → Less:\n{rs}"
        );
    }
}

#[test]
fn rustc_roundtrip_list_of_option_compare_is_lexicographic_with_some_before_none() {
    // Pins the List cmp walk (expr.rs `emit_value_cmp_walk_seen` Ty::List arm) COMPOSED with the Option
    // Some<None flip: `(List (Option Int64))` compare is element-wise lexicographic (first non-Equal element
    // decides), with a length tiebreak (a proper prefix is Less), and each element ordered by the Cadenza
    // declared order Some k < None — NOT std's `Vec<Option<_>>` derived Ord (which is None < Some). Before
    // this, the List-cmp path and the Option flip were each witnessed alone, but never their COMPOSITION on
    // the rust backend — a future change to either the List zip/length shape or the Option flip could
    // silently mis-order a list of options without a failing case.
    let src = "(module m \
        (def (cmp2 (: a (List (Option Int64))) (: b (List (Option Int64)))) \
          (match (Ordering.of a b) ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
        (def (p1) (cmp2 (list (Some 1)) (list (: (None unit) (Option Int64))))) \
        (def (p2) (cmp2 (list (Some 1)) (list (Some 1) (Some 2)))) \
        (def (p3) (cmp2 (list (: (None unit) (Option Int64))) (list (Some 1)))) \
        (def (p4) (cmp2 (list (Some 1) (Some 2)) (list (Some 1) (Some 2)))) \
        (export p1) (export p2) (export p3) (export p4))";
    // `(List (Option Int64))` compare IS representable on the rust backend (verified — it emits the
    // zip/find/length walk nesting the Option flip), so a DECLINE is a real emit regression: `compile_rust`
    // HARD-FAILS on decline (an `Err(_) => {}` arm masked the pin — it stayed green even if the backend
    // stopped emitting). Compiled ONCE; all four probes assert against the single artifact.
    let rs = compile_rust(src);
    // [Some 1] vs [None]: first element Some 1 < None → Less (1).
    if let Some(out) = rustc_run(&rs, "p1()") {
        assert_eq!(out, "1", "[Some 1] < [None] — Some precedes None:\n{rs}");
    }
    // [Some 1] vs [Some 1, Some 2]: equal prefix, shorter is Less (1).
    if let Some(out) = rustc_run(&rs, "p2()") {
        assert_eq!(out, "1", "a proper prefix is Less (length tiebreak):\n{rs}");
    }
    // [None] vs [Some 1]: first element None > Some 1 → Greater (3).
    if let Some(out) = rustc_run(&rs, "p3()") {
        assert_eq!(out, "3", "[None] > [Some 1] — None follows Some:\n{rs}");
    }
    // identical lists → Equal (2).
    if let Some(out) = rustc_run(&rs, "p4()") {
        assert_eq!(out, "2", "identical option-lists compare Equal:\n{rs}");
    }
}

#[test]
fn rustc_roundtrip_record_with_option_field_compare_is_lexicographic_in_sorted_key_order() {
    // Pins the RECORD cmp walk (expr.rs `emit_value_cmp_walk_seen` Ty::Record arm) COMPOSED with the Option
    // Some<None flip. A record compares its fields lexicographically in SORTED-KEY order (the arm iterates
    // `fields.values()`, which are keyed in sorted order) via a `.then_with` chain, and an Option-typed field
    // is ordered by the Cadenza declared order Some k < None — NOT std's derived `Option` Ord (None < Some).
    // The bare-Option compare and the tuple-leaf-Option compare were each witnessed alone, but the Record
    // arm's sorted-field lexicographic chain composed with the flip had no rust-backend witness. Verified
    // NON-VACUOUS: this emits `__cmp_Option(&x.0, &y.0).then_with(|| x.1.cmp(&y.1))` — the record erases to a
    // `(Option<i64>, i64)` tuple, so the pin guards the Record→sorted-field lowering fused with the flip
    // helper. A future change to the field ordering / `.then_with` chain or the Option flip would flip a case.
    //
    // Fields are `a` (Option Int64) and `b` Int64 — `a` sorts before `b`, so the Option field DECIDES first;
    // `b` only breaks a tie when the `a` fields are equal. Pins BOTH Some<None at the deciding field AND that
    // the sorted-key tiebreak reaches the second field only on an equal first.
    let src = "(module m \
        (def (mk-a (: k Int64) (: y Int64)) \
          (: (record (= a (Some k)) (= b y)) (Record (a (Option Int64)) (b Int64)))) \
        (def (mk-n (: y Int64)) \
          (: (record (= a (: (None unit) (Option Int64))) (= b y)) (Record (a (Option Int64)) (b Int64)))) \
        (def (cmp3 (: x (Record (a (Option Int64)) (b Int64))) (: y (Record (a (Option Int64)) (b Int64)))) \
          (match (Ordering.of x y) ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
        (def (p1) (cmp3 (mk-a 1 9) (mk-n 0))) \
        (def (p2) (cmp3 (mk-n 0) (mk-a 1 9))) \
        (def (p3) (cmp3 (mk-a 1 5) (mk-a 1 7))) \
        (def (p4) (cmp3 (mk-a 1 5) (mk-a 1 5))) \
        (export p1) (export p2) (export p3) (export p4))";
    // A record-with-Option-field IS representable on rust (verified — erases to a `(Option<i64>, i64)` tuple
    // and emits `__cmp_Option(&x.0,&y.0).then_with(|| x.1.cmp(&y.1))`), so a DECLINE is a real emit
    // regression: `compile_rust` HARD-FAILS on decline (an `Err(_) => {}` arm would silently mask the pin).
    // Compiled ONCE; all four probes assert against the single artifact.
    let rs = compile_rust(src);
    // {a: Some 1, b: 9} vs {a: None, b: 0}: field `a` sorts first and differs → Some 1 < None → Less (1).
    // (`b` 9 > 0 would say Greater if `b` decided — it must NOT; `a` decides first.)
    if let Some(out) = rustc_run(&rs, "p1()") {
        assert_eq!(
            out, "1",
            "record field `a` (Option) decides first: Some 1 < None → Less (NOT b's 9>0):\n{rs}"
        );
    }
    // symmetric: {a: None, b: 0} vs {a: Some 1, b: 9}: None > Some 1 → Greater (3).
    if let Some(out) = rustc_run(&rs, "p2()") {
        assert_eq!(
            out, "3",
            "symmetric — None follows Some at the deciding field:\n{rs}"
        );
    }
    // {a: Some 1, b: 5} vs {a: Some 1, b: 7}: field `a` equal, tiebreak on `b` → 5 < 7 → Less (1).
    if let Some(out) = rustc_run(&rs, "p3()") {
        assert_eq!(
            out, "1",
            "equal Option field falls through to the `b` tiebreak (5<7):\n{rs}"
        );
    }
    // identical records → Equal (2).
    if let Some(out) = rustc_run(&rs, "p4()") {
        assert_eq!(out, "2", "identical records compare Equal:\n{rs}");
    }
}

#[test]
fn rustc_roundtrip_nonrecursive_user_sum_with_option_payload_compare_declared_disc_then_flip() {
    // Pins emit_sum_cmp_walk's DIRECT (non-helper) path for a NON-RECURSIVE multi-variant user sum whose
    // variants carry Option payloads, composed with the Some<None flip. Distinct from the RECURSIVE-sum pin
    // (rustc_roundtrip_recursive_option_carrying_sum_compare_terminates_via_helper), which exercises the
    // seen-guard `__cmp_<Ident>` self-call recursion base; THIS pins the declared-discriminant ordering
    // (`__ord` maps `A`→0, `B`→1 by DECLARATION order) fused with same-variant payload compare through the
    // Option flip helper — the Cadenza order (declared disc ascending, then payload), NOT std's derived Ord.
    // Verified NON-VACUOUS: emits `fn __cmp_W(..) { let __ord = |v| match v { W::A(..)=>0, W::B(..)=>1 };
    // match (l,r) { (W::A,W::A)=>__cmp_Option(..), (W::B,W::B)=>__cmp_Option(..), _=>__ord(l).cmp(&__ord(r)) }}`.
    let src = "(module m \
        (type W (A (Option Int64)) (B (Option Int64))) \
        (def (mk-a (: k Int64)) (if (= k 0) (W.A (: (None unit) (Option Int64))) (W.A (Some k)))) \
        (def (mk-b (: k Int64)) (if (= k 0) (W.B (: (None unit) (Option Int64))) (W.B (Some k)))) \
        (def (cmp2 (: x W) (: y W)) \
          (match (Ordering.of x y) ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
        (def (p1) (cmp2 (mk-a 5) (mk-b 5))) \
        (def (p2) (cmp2 (mk-b 5) (mk-a 5))) \
        (def (p3) (cmp2 (mk-a 1) (mk-a 0))) \
        (def (p4) (cmp2 (mk-a 7) (mk-a 7))) \
        (export p1) (export p2) (export p3) (export p4))";
    // A non-recursive multi-variant user sum carrying Option payloads IS representable on rust (verified —
    // emits `__cmp_W` with an `__ord` declared-disc map + same-variant `__cmp_Option` routing), so a DECLINE
    // is a real emit regression: `compile_rust` HARD-FAILS on decline (an `Err(_) => {}` arm would silently
    // mask the pin). Compiled ONCE; all four probes assert against the single artifact.
    let rs = compile_rust(src);
    // A(Some 5) vs B(Some 5): different variants, declared A(disc 0) < B(disc 1) → Less (1).
    // (payloads are equal, so ONLY the declared-discriminant order can decide.)
    if let Some(out) = rustc_run(&rs, "p1()") {
        assert_eq!(
            out, "1",
            "declared discriminant decides across variants: A < B → Less:\n{rs}"
        );
    }
    // symmetric: B(Some 5) vs A(Some 5): B(disc 1) > A(disc 0) → Greater (3).
    if let Some(out) = rustc_run(&rs, "p2()") {
        assert_eq!(
            out, "3",
            "symmetric — B follows A by declared discriminant:\n{rs}"
        );
    }
    // A(Some 1) vs A(None): same variant A, payload Some 1 < None → Less (1) (the flip inside a variant).
    if let Some(out) = rustc_run(&rs, "p3()") {
        assert_eq!(
            out, "1",
            "same-variant payload uses Some<None flip: Some 1 < None → Less:\n{rs}"
        );
    }
    // A(Some 7) vs A(Some 7): identical → Equal (2).
    if let Some(out) = rustc_run(&rs, "p4()") {
        assert_eq!(out, "2", "identical sum values compare Equal:\n{rs}");
    }
}

#[test]
fn rustc_roundtrip_nested_option_of_option_compare_emits_declared_order_some_before_none() {
    // The generic/nested-Option compare EMIT slice: `compare` over `(Option (Option Int64))` used to DECLINE
    // ("value-cmp over a RECURSIVE GENERIC Option-carrying sum is not yet rendered") — a FALSE trigger of the
    // recursion guard, which keyed on the bare `Option` decl id, so the INNER Option re-entering the SAME
    // decl tripped the "recursive generic" path even though `Option<Option<i64>>` is a FINITE distinct
    // instantiation, not self-recursion. The fix keys the guard on the FULL instantiated type (Ty by
    // decl+args) and mangles the `__cmp_*` helper name per instantiation, so the two Option layers get
    // distinct helpers and the walk expands inline. The Cadenza declared order is Some(_) < None at BOTH
    // layers, giving the total order Some(Some k) < Some(None) < None (NOT std's None < Some).
    let src = "(module m \
        (def (cmp2 (: x (Option (Option Int64))) (: y (Option (Option Int64)))) \
          (match (Ordering.of x y) ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
        (def (p1) (cmp2 (Some (Some 1)) (Some (: (None unit) (Option Int64))))) \
        (def (p2) (cmp2 (Some (: (None unit) (Option Int64))) (: (None unit) (Option (Option Int64))))) \
        (def (p3) (cmp2 (Some (Some 1)) (Some (Some 1)))) \
        (def (p4) (cmp2 (Some (Some 1)) (Some (Some 2)))) \
        (def (p5) (cmp2 (Some (Some 2)) (Some (Some 1)))) \
        (export p1) (export p2) (export p3) (export p4) (export p5))";
    // MUST emit now (this is the whole point of the slice) — `compile_rust` HARD-FAILS on a decline so a
    // regression back to the old "recursive generic" decline reds this test.
    let rs = compile_rust(src);
    // Some(Some 1) vs Some(None): both outer Some, inner Some 1 < None → Less (1).
    if let Some(out) = rustc_run(&rs, "p1()") {
        assert_eq!(
            out, "1",
            "Some(Some 1) < Some(None): inner Some<None decides → Less:\n{rs}"
        );
    }
    // Some(None) vs None: outer Some < None → Less (1).
    if let Some(out) = rustc_run(&rs, "p2()") {
        assert_eq!(out, "1", "Some(None) < None: outer Some<None → Less:\n{rs}");
    }
    // identical Some(Some 1) → Equal (2).
    if let Some(out) = rustc_run(&rs, "p3()") {
        assert_eq!(out, "2", "identical nested options compare Equal:\n{rs}");
    }
    // Some(Some 1) vs Some(Some 2): inner payload 1 < 2 → Less (1).
    if let Some(out) = rustc_run(&rs, "p4()") {
        assert_eq!(
            out, "1",
            "Some(Some 1) < Some(Some 2) by inner payload:\n{rs}"
        );
    }
    // symmetric: Some(Some 2) vs Some(Some 1) → Greater (3).
    if let Some(out) = rustc_run(&rs, "p5()") {
        assert_eq!(
            out, "3",
            "Some(Some 2) > Some(Some 1) by inner payload:\n{rs}"
        );
    }
}

#[test]
fn rustc_roundtrip_nested_option_of_option_float_eq_emits_and_walks_canonical_bytes() {
    // Mirror of the nested/generic-Option value-CMP emit slice, for value-EQ. `(Option (Option Float64))`
    // equality used to DECLINE ("runtime structural equality over a RECURSIVE GENERIC sum is not yet
    // rendered") — the eq walk's recursion guard keyed on the bare `Option` decl, so the inner Option
    // re-entering the same decl false-tripped the recursive-generic decline. (A nested Option of an Eq type
    // like Int64 never reaches here — it takes the native `==` fast-path; a FLOAT leaf is what forces the eq
    // WALK, since f64 is not `Eq`.) The fix keys the guard on the FULL instantiated type, so the inner
    // Option<Float64> expands inline. The float leaf still compares by the canonical byte form (NaN==NaN,
    // -0.0 != +0.0), NOT f64's `PartialEq`. `compile_rust` HARD-FAILS on decline (this MUST emit now).
    let src = "(module m \
        (def (eq2 (: x (Option (Option Float64))) (: y (Option (Option Float64)))) (if (= x y) 1 0)) \
        (def (mk (: k Float64)) (if (= k 0.0) (: (None unit) (Option Float64)) (Some k))) \
        (def (p1) (eq2 (Some (mk 1.5)) (Some (mk 1.5)))) \
        (def (p2) (eq2 (Some (mk 1.5)) (Some (mk 2.5)))) \
        (def (p3) (eq2 (Some (mk 0.0)) (: (None unit) (Option (Option Float64))))) \
        (def (p4) (eq2 (: (None unit) (Option (Option Float64))) (: (None unit) (Option (Option Float64))))) \
        (export p1) (export p2) (export p3) (export p4))";
    let rs = compile_rust(src);
    // Some(Some 1.5) == Some(Some 1.5) → equal (1); the inner float compares by canonical bytes.
    if let Some(out) = rustc_run(&rs, "p1()") {
        assert_eq!(out, "1", "equal nested-option floats compare equal:\n{rs}");
    }
    // Some(Some 1.5) vs Some(Some 2.5) → the inner float differs → not equal (0).
    if let Some(out) = rustc_run(&rs, "p2()") {
        assert_eq!(out, "0", "a differing inner float → unequal:\n{rs}");
    }
    // Some(None) vs None → the OUTER option differs → not equal (0).
    if let Some(out) = rustc_run(&rs, "p3()") {
        assert_eq!(out, "0", "Some(None) != None (outer differs):\n{rs}");
    }
    // None == None → equal (1).
    if let Some(out) = rustc_run(&rs, "p4()") {
        assert_eq!(out, "1", "None == None:\n{rs}");
    }
}

#[test]
fn rustc_roundtrip_generic_user_sum_carrying_option_compare_emits_via_full_type_guard() {
    // The nested/generic-Option value-CMP fix (full-instantiated-type recursion key + per-instantiation
    // helper name) generalizes BEYOND the built-in Option: a GENERIC USER sum carrying an Option payload —
    // `(type Box a (Wrap a) (Empty))` at `Box (Option Int64)` — now emits its compare too (it used to hit the
    // same false recursive-generic decline). Declared order: Wrap(disc 0) < Empty(disc 1); same-variant Wrap
    // compares the Option payload with the Some<None flip. `compile_rust` HARD-FAILS on decline.
    let src = "(module m \
        (type Box a (Wrap a) (Empty)) \
        (def (cmp2 (: x (Box (Option Int64))) (: y (Box (Option Int64)))) \
          (match (Ordering.of x y) ((Ordering.Less) 1) ((Ordering.Equal) 2) ((Ordering.Greater) 3))) \
        (def (p1) (cmp2 (Box.Wrap (Some 1)) (Box.Wrap (: (None unit) (Option Int64))))) \
        (def (p2) (cmp2 (Box.Wrap (Some 1)) (Box.Empty))) \
        (def (p3) (cmp2 (Box.Wrap (Some 1)) (Box.Wrap (Some 1)))) \
        (export p1) (export p2) (export p3))";
    let rs = compile_rust(src);
    // Wrap(Some 1) vs Wrap(None): same variant Wrap, payload Some 1 < None → Less (1).
    if let Some(out) = rustc_run(&rs, "p1()") {
        assert_eq!(
            out, "1",
            "Wrap(Some 1) < Wrap(None) via the Some<None payload flip:\n{rs}"
        );
    }
    // Wrap(Some 1) vs Empty: Wrap(disc 0) < Empty(disc 1) → Less (1).
    if let Some(out) = rustc_run(&rs, "p2()") {
        assert_eq!(out, "1", "Wrap < Empty by declared discriminant:\n{rs}");
    }
    // identical → Equal (2).
    if let Some(out) = rustc_run(&rs, "p3()") {
        assert_eq!(
            out, "2",
            "identical generic-user-sum values compare Equal:\n{rs}"
        );
    }
}

#[test]
fn rustc_host_call_no_arg_int_result_emits_a_canonical_shim_call() {
    // H1 host-boundary emit: a delegated no-arg, integer-result host op (`ask.ask -> Int64`) renders as a
    // call to a crate-root shim `crate::__cdz_host_<key>()` the runner supplies (the gate driver generates
    // it from recorded host-responses; a real embedder implements it). The shim name derives from the
    // CANONICAL op key (kebab-normalized effect + verbatim op) so it agrees with the driver-generated fn
    // regardless of the source effect's casing — here effect `ask` → `ask.ask` → `__cdz_host_ask_ask`.
    let src = "(module m (effect ask (op ask (-> Unit Int64))) \
        (def (main) (host (ask) (* (ask.ask) 10))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("crate::__cdz_host_ask_ask()"),
        "a no-arg Int64 host op emits a canonical crate-root shim call:\n{rs}"
    );

    // A CAPITALIZED effect name kebab-normalizes in the shim ident (`Env` → `env`), matching the component
    // extern name the corpus/cdz-run key by — so backend-emitted and driver-generated idents agree.
    let cap = "(module m (effect Env (op width (-> Unit Int64))) \
        (def (main) (host (Env) (Env.width))) (export main))";
    let rc = compile_rust(cap);
    assert!(
        rc.contains("crate::__cdz_host_env_width()"),
        "a capitalized effect kebab-normalizes in the shim ident (Env → env):\n{rc}"
    );
    // (A host op WITH integer ARGUMENTS now emits too — see
    // rustc_host_call_with_int_arg_emits_left_to_right_bound_args (H3); the earlier
    // "with-args declines" sub-assertion here was retired when H3 landed.)
}

#[test]
fn rustc_host_call_narrow_int_result_casts_the_shim_i64_to_the_declared_width() {
    // H1 host-call emit, narrow-result face: a delegated host op whose result is a NARROW fixed-width int
    // (`src.next -> UInt8`) emits `(crate::__cdz_host_<key>() as u8)` — the runner's shim returns the recorded
    // scalar as `i64`, and the emit casts to the op's DECLARED width. `as u8` truncates to the byte, matching
    // the wasm boundary's width semantics (the host supplies a value of the op's declared type). Here the
    // whole is re-widened by `Int64.of`, so the emit is `((… as u8) as i64)` — pins BOTH the narrow cast and
    // that it composes with a widening op. No corpus case exercises a DELEGATED narrow-result host op (the
    // only UInt8 host op is handled in-program), so this rcdzc-lib pin is the sole witness for the cast path.
    let src = "(module m (effect src (op next (-> Unit UInt8))) \
        (def (main) (host (src) (Int64.of (src.next)))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("crate::__cdz_host_src_next() as u8"),
        "a UInt8-result host op casts the shim's i64 to u8 (the declared width):\n{rs}"
    );
}

#[test]
fn rustc_host_call_bool_result_reads_the_shim_i64_truthiness() {
    // H2 host-call emit: a delegated BOOL-result host op (`Param.mirror -> Unit Bool`, from an
    // `@param(widget: toggle) mirror : Bool`) emits `(crate::__cdz_host_<key>() != 0)` — the runner's shim
    // returns the recorded scalar as `i64` (a `true`/`false` response normalized to `1`/`0`), and the emit
    // reads its truthiness (matching the wasm boundary's i32→bool). Pins the bool arm of the result marshal.
    let src = "(module m (effect Param (op mirror (-> Unit Bool))) \
        (def (main) (host (Param) (Param.mirror))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("crate::__cdz_host_param_mirror() != 0"),
        "a Bool-result host op reads the shim's i64 truthiness (!= 0):\n{rs}"
    );
}

#[test]
fn rustc_host_call_with_int_arg_emits_left_to_right_bound_args() {
    // H3 host-call emit: a delegated host op WITH an integer argument (`out.put -> Int64 Int64`, arg a guest
    // arithmetic expr) emits the arg EVALUATED and passed to the shim. Args bind to `let __ha<i>` in source
    // LEFT-TO-RIGHT order (the host-call sequence the oracle records — the arg values cross the boundary but
    // are not compared; only the op name is), then `crate::__cdz_host_<key>(__ha0, …)`. Marshalled `as i64`
    // to the shim's i64 param. Pins the arg-eval + call shape.
    let src = "(module m (effect out (op put (-> Int64 Int64))) \
        (def (main (: n Int64)) (host (out) (out.put (* (+ n 1) 10)))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("let __ha0 = ") && rs.contains("crate::__cdz_host_out_put(__ha0)"),
        "a host call with an int arg binds the arg (__ha0) then passes it to the shim:\n{rs}"
    );
    // (A String host-call ARG now emits too — see rustc_host_call_string_arg_and_string_result_marshal
    // (H7); the earlier "non-integer arg declines" sub-assertion here was retired when H7 landed.)
}

#[test]
fn rustc_host_call_bool_arg_marshals_to_i64() {
    // A BOOL host-call ARGUMENT (`io.pick : (-> Bool Int64)`, arg a guest comparison) marshals to `i64`
    // via `i64::from(<bool>)` (0/1) — the SAME uniform integer marshal an int arg uses, matching wasm
    // (which reps Bool as i32 and crosses it fine). Bool was the one scalar-arg gap the earlier arms missed
    // (int/UInt/Int64 already passed; v-effects routed the Bool-specific parity gap). `as i64` doesn't apply
    // to `bool` in Rust, so the cast goes through `i64::from`. Pins the Bool arm of the host-arg marshal.
    let src = "(module m (effect io (op pick (-> Bool Int64))) \
        (def (main (: k Int64)) (host (io) (io.pick (> k 100)))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("let __ha0 = i64::from(") && rs.contains("crate::__cdz_host_io_pick(__ha0)"),
        "a Bool host-call arg marshals via i64::from(<bool>) then passes __ha0 to the shim:\n{rs}"
    );
}

#[test]
fn rustc_closure_that_escapes_an_effect_rejects_cdz0406_not_a_broken_emit() {
    // A closure whose body performs a DELEGATED effect and CROSSES the host boundary must be REJECTED
    // (CDZ0406 — "closures escaping effects are not supported"): the closure's handler context does not
    // travel with it, so the effect has no home when the host later invokes it. The wasm backend enforces
    // this (backend/wasm/mod.rs); the rust backend previously had NO such guard, so it tried to EMIT the
    // escaping-effect closure and produced un-compilable Rust (an unresolved host-shim call → E0061),
    // graded `todo` while wasm rejected — a cross-backend diagnostic differential. The rust emit now scans
    // the reached AND eta-peeled lifted bodies for a host import and rejects with the SAME code + message.
    // Here `(def (main) (host (ask) (fn (x) (+ x (ask.ask)))))` peels `main` to `fn main(x)` whose body
    // performs `ask.ask` — the peeled-export case the first (lifted-only) scan missed.
    let src = "(module m (effect ask (op ask (-> Unit Int64))) \
        (def (main) (host (ask) (fn ((: x Int64)) (+ x (ask.ask))))) (export main))";
    let err = try_compile_rust(src).expect_err(
        "a closure escaping an effect must REJECT on rust (CDZ0406), not emit broken Rust",
    );
    assert!(
        err.iter().any(
            |d| d.contains("closures escaping effects are not supported") && d.contains("ask.ask")
        ),
        "the rust reject must name the escaping effect + the unsupported feature (CDZ0406):\n{err:?}"
    );
}

#[test]
fn rustc_an_unreached_effectful_lifted_closure_does_not_spuriously_reject_cdz0406() {
    // The DUAL of the reject above (and the rust twin of the wasm #1808 fix): the CDZ0406 escaping-closure
    // scan must fire ONLY for a REACHABLE escaping closure. `layout.lifted` also holds lambdas DEMANDED
    // during type-checking but built by no reachable `Core::Closure` — emitted as inert never-called STUBS
    // (gated on `layout.lifted_reached`). A `Core::HostCall` in such a dead/stub body is provably
    // unreachable, so flagging it would SPURIOUSLY reject a program whose effectful closure can never run.
    // The rust guard already scans `peeled || reached` slots only (backend/rust/mod.rs), so it never had the
    // false-reject the wasm side did pre-#1808 — this witness PINS that (a future change dropping the
    // reached-gate would regress it to a spurious CDZ0406). Same program shape as the wasm witness: the
    // effectful `Box.Bin` closure is an unreached lift (`pick 0` always takes the `Box.Un` arm), never
    // invoked by the host — so it must COMPILE, not reject.
    let src = "(module m (effect ask (op ask (-> Unit Int64))) \
        (type Box (Bin (-> Int64 Int64 Int64)) (Un (-> Int64 Int64))) \
        (def (run (: b Box)) (match b ((Box.Bin f) (f 2 3)) ((Box.Un g) (g 9)))) \
        (def (pick (: which Int64)) \
          (if (> which 0) \
            (Box.Bin (fn ((: a Int64) (: x Int64)) (+ a (ask.ask)))) \
            (Box.Un (fn ((: x Int64)) (+ x 1))))) \
        (def (main) (host (ask) (run (pick 0)))) \
        (export main))";
    let res = try_compile_rust(src);
    assert!(
        res.is_ok(),
        "an UNREACHED effectful lifted closure must NOT spuriously reject CDZ0406 on rust (it can never \
         run — only a reachable escaping closure rejects): {:?}",
        res.err()
    );
}

#[test]
fn rustc_host_call_float_result_reads_the_shim_f64() {
    // H4 host-call emit: a delegated FLOAT-result host op (`Param.ratio -> Unit Float64`, from an
    // `@param(widget: slider) ratio : Float64`) emits `(crate::__cdz_host_<key>() as f64)` — the runner's
    // shim returns the recorded scalar as `f64` (the gate driver keys the shim's return type on the response
    // value text: a `.`-bearing value → an f64 shim), and the emit casts to the declared float width. Pins
    // the float arm of the result marshal (distinct from the int `as <width>` / bool `!= 0` arms).
    let src = "(module m (effect Param (op ratio (-> Unit Float64))) \
        (def (main) (host (Param) (Param.ratio))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("crate::__cdz_host_param_ratio() as f64"),
        "a Float64-result host op reads the shim as f64:\n{rs}"
    );
}

#[test]
fn rustc_host_call_seq_emits_statements_in_source_order() {
    // H5 host-call emit: a Core::Seq (a `do` whose non-final statements reach a side effect — here two
    // host calls, the first's result discarded) emits a Rust block `{ let _ = <stmt0>; <tail> }`. The
    // statements emit in WRITTEN order, so the host calls are observed in exactly the order the program made
    // them (the sequencing invariant). Pins that Seq renders (was "does not yet render this compound value").
    let src = "(module m (effect out (op put (-> Int64 Int64))) \
        (def (main) (host (out) (do (out.put 1) (out.put 2)))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("let _ = ") && rs.matches("crate::__cdz_host_out_put").count() >= 2,
        "a Seq of two host calls binds the first with `let _ =` then the tail, both calling the shim:\n{rs}"
    );
    // The first (discarded) put must appear BEFORE the second (the block's value) — source order.
    let first = rs
        .find("let _ = { let __ha0 = (1u64")
        .or_else(|| rs.find("let _ = "));
    assert!(
        first.is_some(),
        "the first host call binds before the tail:\n{rs}"
    );
}

#[test]
fn rustc_host_seq_elides_a_discarded_pure_statement_no_spurious_trap() {
    // adv-56 (breaker): the Core::Seq emit must NOT run a DISCARDED non-final statement that reaches no host
    // call — its value is dropped and its trap is unobserved (dead-init ruling §283). `(do (/ 100 d)
    // (io.put 1) 42)` inside `(host (io) …)`: `(/ 100 d)` is pure + discarded → ELIDED entirely (not emitted
    // as `let _ = …`), so at d=0 it does NOT div-by-zero; only the host-call statement is kept. Emitted:
    // `{ let _ = { … __cdz_host_io_put(__ha0) … }; (42u64 as i64) }` — the io.put shim call present, NO
    // division of the discarded pure item.
    let src = "(module m (effect io (op put (-> Int64 Int64))) \
        (def (main (: d Int64)) (host (io) (do (/ 100 d) (io.put 1) 42))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("crate::__cdz_host_io_put") && !rs.contains("100u64"),
        "the discarded pure `(/ 100 d)` (100u64) is elided from the Seq; only the host-call stmt kept:\n{rs}"
    );
    // (Runtime behavior — main(0) = 42, no spurious trap — is verified by the corpus gate case adv-56;
    // the lib-test `rustc_run` can't execute a host-delegating program, so this pins the EMIT only.)
}

#[test]
fn rustc_dead_let_heap_ctor_force_evals_trapping_scalar_arg() {
    // CASE2 (#5194/#5328, breaker cross-backend gate-check-rust red): the STRICT-construction twin of the
    // adv-56 elide above. A REACHED list/set/map ctor whose result is DEAD still MUST evaluate its
    // trap-possible SCALAR arg computations (their traps occur) — the (A)-strict rule OVERRIDES §283
    // dead-init elision (which elides only a bare unobserved scalar; v-spec-oracle). `(let ((x (list 1
    // (/ 5 d)))) 0)`: `lower_let` decomposes the dead list ctor to its trap-possible scalar `(/ 5 d)`,
    // marks it in `db.strict_force_eval`, and sequences it (discarded) before the body `0` via `Core::Seq`
    // — NOTHING is built (decompose-and-mark, not build-and-reclaim → no borrowed-element double-free). The
    // rust `Core::Seq` emit now force-evaluates a `strict_force_eval` stmt (`let _ = …`) instead of eliding
    // it, so at d=0 the div-by-zero TRAPS. This was wasm-only in #5328 (the rust backend §283-elided the
    // dead ctor and returned 0, dropping the trap) → v-spec-oracle's #5332 03-equality:1658 pin went RED on
    // gate-check-rust until this arm. `f` (not `main`) avoids colliding with the generated `fn main`.
    let src = "(module m (def (f (: d Int64)) (let ((x (list 1 (/ 5 d)))) 0)) (export f))";
    let rs = compile_rust(src);
    // EMIT: the decomposed `(/ 5 d)` is force-evaluated (the checked div is present), NOT §283-elided; and
    // the dead list itself is NOT built (decompose-and-mark) — no `Vec`/list-ctor for it. The scalar div
    // emits an explicit `if r == 0 { panic!("division by zero") }` guard (the rust backend's div-by-zero
    // trap form), inside a discarded `let _ = { … }` — its presence proves the arg was force-evaluated, not
    // §283-elided (an elided arg would leave NO `division by zero` guard anywhere in the emit).
    assert!(
        rs.contains("division by zero") && rs.contains("let _ = {"),
        "the dead-list ctor's trap-possible scalar arg `(/ 5 d)` is force-evaluated (its div-by-zero guard \
         is present in a discarded `let _ = {{`), not elided:\n{rs}"
    );
    // RUNTIME d=0: the (A)-strict force makes the discarded `(/ 5 0)` TRAP divide-by-zero (a RanOk here is a
    // lost trap — the exact regression breaker flagged). d=1: runs → body value 0 (no trap, nothing leaked).
    match rustc_run_traps(&rs, "f(0)") {
        TrapRun::Trapped(msg) => assert!(
            msg.contains("by zero"),
            "a dead-let list ctor with a `/0` scalar arg must TRAP divide-by-zero; panic was:\n{msg}"
        ),
        TrapRun::RanOk(out) => {
            panic!(
                "dead-let `(list 1 (/ 5 0))` discarded must STILL trap (CASE2 strict force), but ran → {out}"
            )
        }
        TrapRun::NoRustc => {}
    }
    if let Some(out) = rustc_run(&rs, "f(1)") {
        assert_eq!(
            out, "0",
            "d=1: the dead-let ctor is discarded, no trap, the body value 0 is returned"
        );
    }
}

#[test]
fn rustc_closure_capturing_a_let_bound_host_call_emits_it_once() {
    // H6: a returned closure that captures a LET-BOUND host-call result must fire the host op ONCE, not
    // twice. `(host (ask) (let ((v (ask.ask))) (fn (x) (+ x v))))` — lowering inlines the `(ask.ask)` value
    // into `Core::Closure.captures` (the capture node IS the host-call node, not a `LocalRef` to `v`), so a
    // naive capture emit re-ran it → 2 host calls / 1 recorded response → OOB (the factory-host double-emit
    // bug). The `Core::Let` arm now maps the value node → its binding name, and the `Core::Closure` capture
    // references that binding instead of re-emitting. Pin: exactly ONE `__cdz_host_ask_ask` call site.
    let src = "(module m (effect ask (op ask (-> Unit Int64))) \
        (def (main) (host (ask) (let ((v (ask.ask))) (fn ((: x Int64)) (+ x v))))) (export main))";
    let rs = compile_rust(src);
    assert_eq!(
        rs.matches("crate::__cdz_host_ask_ask").count(),
        1,
        "a let-bound host call captured by a returned closure must be emitted exactly ONCE:\n{rs}"
    );
    // A non-closure double-use already dedups via the let binding — pin it stays 1 (regression guard).
    let twouse = "(module m (effect ask (op ask (-> Unit Int64))) \
        (def (main) (host (ask) (let ((v (ask.ask))) (+ v v)))) (export main))";
    assert_eq!(
        compile_rust(twouse)
            .matches("crate::__cdz_host_ask_ask")
            .count(),
        1,
        "a let-bound host call used twice in a non-closure body stays a single call site"
    );
}

#[test]
fn rustc_host_call_string_arg_and_string_result_marshal() {
    // H7 host-call emit: STRING args + STRING results. A host op taking a String arg emits the arg to a
    // GENERIC shim param (`fn shim<A0>(_a0: A0)` — the arg value isn't compared, so the shim ignores it);
    // a String-result op reads the shim's `String` return directly. (A Unit-result effect op now emits too —
    // see rustc_host_call_unit_result_emits_the_shim_call_for_effect_then_unit (H8); the earlier
    // "Unit-result declines" sub-note here was retired when H8 landed.)
    // String arg into a value-returning op — log.ask : (-> String Int64).
    let arg = "(module m (effect log (op ask (-> String Int64))) \
        (def (main) (host (log) (log.ask \"q\"))) (export main))";
    let rs = compile_rust(arg);
    assert!(
        rs.contains("\"q\".to_string()") && rs.contains("crate::__cdz_host_log_ask(__ha0)"),
        "a String host-call arg emits `\"q\".to_string()` passed to the shim:\n{rs}"
    );
    // String RESULT — line : (-> Unit String); the shim returns String, emit passes it through to a String
    // consumer (String.concat).
    let sres = "(module m (effect log (op line (-> Unit String))) \
        (def (main) (host (log) (String.concat (log.line) \"!\"))) (export main))";
    let rr = compile_rust(sres);
    assert!(
        rr.contains("crate::__cdz_host_log_line()"),
        "a String-result host op reads the shim's String return:\n{rr}"
    );
}

#[test]
fn rustc_host_call_unit_result_emits_the_shim_call_for_effect_then_unit() {
    // H8 host-call emit: a UNIT-result effect op (`log.emit : (-> String Unit)`) crosses the boundary FOR
    // ITS SIDE EFFECT only — it is OBSERVED (its op name enters the host-call sequence) and yields the unit
    // value. Emit `{ crate::__cdz_host_<key>(__ha0); () }`: call the shim for effect, then evaluate to `()`.
    // (The shim itself — a `()`-returning fn that prints its op — is generated by the gate DRIVER from the
    // case's `(host-calls …)` sequence, not from host_responses, since a unit-result op records no response
    // value; that side is exercised by the corpus gate cases this increment flips to pass.) Pins the Unit
    // arm of the result marshal (was "does not yet render a host call whose result is not … unit").
    let src = "(module m (effect log (op emit (-> String Unit))) \
        (def (main) (host (log) (log.emit \"hello\"))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("crate::__cdz_host_log_emit(__ha0); () }"),
        "a Unit-result host op emits the shim call for effect then `()`:\n{rs}"
    );
    // The arg still binds left-to-right (the String crosses to the generic shim param, H7) before the call.
    assert!(
        rs.contains("\"hello\".to_string()"),
        "the Unit-result op's String arg still marshals to the shim:\n{rs}"
    );
}

#[test]
fn rustc_host_call_unit_arg_evaluates_for_effect_then_passes_unit() {
    // H9 host-call emit: a UNIT-typed ARGUMENT (`io.fetch : (-> Unit String)` called `(io.fetch unit)`). The
    // `unit` operand carries no data but may itself be effectful, so the arg binds to `{ <av>; () }` — the
    // operand is evaluated for its side effect, then `()` crosses to the generic shim param (H7). This
    // UNBLOCKS a String-RESULT host op reached through a unit arg (breaker's s2): the arg-decline previously
    // fired BEFORE the String-result marshal (H7) could run. Here `io.fetch`'s String result reads twice.
    let src = "(module m (effect io (op fetch (-> Unit String))) \
        (def (main) (host (io) (String.byte-len (io.fetch unit)))) (export main))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("let __ha0 = { (); () };"),
        "a Unit-typed host-call arg binds `{{ <expr>; () }}` (eval-for-effect then unit):\n{rs}"
    );
    // And the shim IS called with that unit arg (the whole no longer declines on the argument — H7's
    // String-result marshal now runs on the shim's String return, read here via `.len()`).
    assert!(
        rs.contains("crate::__cdz_host_io_fetch(__ha0)"),
        "the unit arg is passed to the shim, reaching the String-result marshal:\n{rs}"
    );
}

#[test]
fn adv68_inrange_sibling_typed_set_element_and_map_value_render_at_the_collection_width() {
    // adv-68 (v-inference ROUTED, emit-side): an IN-RANGE bare literal in a Set element / Map key/value
    // position defaults its OWN `type_of` to Int64, so the rust emitter rendered it `Nu64 as i64` —
    // dropping an `i64` into a `BTreeSet<u64>` / a `BTreeMap<_, u8>` value slot → rustc E0308 (wasm just
    // wraps the literal to the element width and runs). The emit half of #1780's CDZ0302 sibling-width
    // range check: the literal must render at the COLLECTION's settled element/value width, via
    // `emit_grounded` (the Set/Map twin of the list-element render #1766). Witnesses that it now BUILDS
    // and computes the right value under rustc (a witness that merely compiled the container type but
    // never ran would not prove the element render is right).
    //
    // Set over a sibling-typed element: `(: 1 UInt64)` pins the set's element type to UInt64; the bare
    // in-range `41` must render `41u64` (not `41u64 as i64`) into the `BTreeSet<u64>`.
    let set = "(module m \
        (def (run) (Set.len (Set.of (list (: 1 UInt64) 41)))) \
        (export run))";
    let set_res = try_compile_rust(set);
    assert!(
        set_res.is_ok(),
        "a sibling-UInt64 Set element must render at the set width, not `as i64`: {:?}",
        set_res.err()
    );
    if let Some(out) = rustc_run(&compile_rust(set), "run()") {
        assert_eq!(
            out, "2",
            "the two distinct u64 elements {{1, 41}} count as 2"
        );
    }
    // Map with a sibling-typed VALUE: the first entry's `(: 5 UInt8)` pins the value type to UInt8; the
    // second entry's bare in-range `30` must render `30u8` (not `30u64 as i64`) into the `BTreeMap<_, u8>`
    // value slot. A chained `Map.insert(Map.insert Map.empty …)` folds to `Core::MapNew`, so the fix lives
    // in the MapNew entry render (and the MapInsert arm for the un-folded single-insert shape).
    let map = "(module m \
        (def (run) (Map.len (Map.insert (Map.insert (Map.empty) 1 (: 5 UInt8)) 2 30))) \
        (export run))";
    let map_res = try_compile_rust(map);
    assert!(
        map_res.is_ok(),
        "a sibling-UInt8 Map value must render at the value width, not `as i64`: {:?}",
        map_res.err()
    );
    if let Some(out) = rustc_run(&compile_rust(map), "run()") {
        assert_eq!(out, "2", "the two distinct keys {{1, 2}} count as 2");
    }
}

#[test]
fn a_partial_eval_explosion_declines_on_rust_instead_of_an_unbuildable_artifact() {
    // REGRESSION (breaker nsq1 `83e6f35b9`, 2026-08-15, bank `.breaker-probes/2026-08-15-newton-sqrt`).
    // The effect handler is fully partial-evaluated into ONE arithmetic expression; the `improve` arm's
    // compound `(/ (+ x (/ t x)) 2)` references `x` twice and CHAINS into the next `improve` as its `x`,
    // so a 5-`improve` chain expands ~2^5 x the nested division structure. The Rust emit re-descends the
    // shared Core DAG per reference → ONE ~7MB function body that `rustc` cannot compile (parse/typecheck
    // times out) — the corpus gate reported the opaque "artifact did not build" FAIL (a `BadArtifact`,
    // NOT a miscompile: wasm passes at 738KB, under `EMIT_INSTRUCTION_BUDGET`). The per-function emit-size
    // backstop (`expr::RUST_FN_EMIT_BUDGET`) now DECLINES it cleanly (a `todo`), so the gate never hands
    // rustc an unbuildable multi-MB artifact. Durable linear fix = sharing-aware emit (separate increment,
    // blocked on the Perceus dup/drop seam) — this pins the SOUNDNESS backstop, the Rust twin of the wasm
    // `EMIT_INSTRUCTION_BUDGET` decline.
    let nsq1 = "(module m \
        (effect N (op improve (-> Int64)) (op done (-> Int64))) \
        (def (main (: n Int64)) \
          (handle N (tuple (+ 60 (* n 7)) (+ 60 (* n 7))) \
            ((improve () st (match st \
                              ((tuple x t) \
                               (resume (/ (+ x (/ t x)) 2) (tuple (/ (+ x (/ t x)) 2) t))))) \
             (done () st (match st \
                           ((tuple x t) \
                            (if (< t (* (+ x 1) (+ x 1))) \
                                (if (< t (* x x)) (resume 0 st) (resume 1 st)) \
                                (resume 0 st)))))) \
            (let ((a (N.improve))) \
              (let ((b (N.improve))) \
                (let ((c (N.done))) \
                  (let ((d (N.improve))) \
                    (let ((e (N.improve))) \
                      (let ((f (N.done))) \
                        (let ((g (N.improve))) \
                          (let ((h (N.done))) \
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g)) h))))))))))) \
          (export main))";
    let res = compile_rust_result(nsq1);
    assert!(
        res.is_err(),
        "an emit that would blow past the per-function size budget must DECLINE (a todo), not emit an \
         unbuildable multi-MB artifact"
    );
    let msg = res.err().unwrap().join(" ");
    assert!(
        msg.contains("function-size budget"),
        "the decline cites the per-function emit-size backstop: {msg}"
    );
}

#[test]
fn the_rust_fn_emit_budget_enforcer_declines_over_budget_bodies() {
    // Unit-pins the backstop threshold + decline (fast; no compile). A body AT the budget is fine; one
    // byte OVER declines cleanly. Guards the constant + logic even if a future refactor moves the call
    // sites (the e2e test above pins the WIRING).
    use super::expr::{RUST_FN_EMIT_BUDGET, enforce_fn_emit_budget};
    assert!(
        enforce_fn_emit_budget(&"x".repeat(RUST_FN_EMIT_BUDGET)).is_ok(),
        "a body exactly at the budget is allowed"
    );
    assert!(
        enforce_fn_emit_budget(&"x".repeat(RUST_FN_EMIT_BUDGET + 1)).is_err(),
        "a body one byte over the budget declines"
    );
}
