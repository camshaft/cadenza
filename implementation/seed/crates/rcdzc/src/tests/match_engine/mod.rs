use crate::backend::Target;
use crate::compile::{compile, compile_component};
use crate::testkit::parse;

fn component(src: &str) -> Vec<u8> {
    compile_component(&crate::codec::encode(&parse(src))).expect("compile")
}

/// The coded rejection a program produces, or `None` if it compiled. Used to pin a well-formedness
/// rejection (CDZ code) rather than a silent miscompile.
fn reject_code(src: &str) -> Option<String> {
    let bytes = crate::codec::encode(&parse(src));
    crate::host::run_with_compiler_stack(|| {
        let out = compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                bytes,
            )],
            &[Target::Wasm],
        );
        if out.artifact(Target::Wasm.artifact_kind()).is_some() {
            return None; // compiled — no rejection
        }
        // SKIP the umbrella CDZ0900 "unsupported construct" decline (seq-286): it is a safe NOT-YET
        // decline, NOT a program-is-wrong reject (diag.rs `Code::UnsupportedConstruct`), and it is
        // commonly a SCAFFOLD artifact here — a test that exports `(def (f (: xs (List/Map/…))) …)` to
        // exercise a MATCH/pattern hits the "non-scalar entry parameter is not supported on this export
        // path" CDZ0900 (backend/wasm/mod.rs, flipped decline()→unsupported() in #6101) regardless of the
        // pattern under test. Before #6101 that decline was code-`None`, so `reject_code` returned `None`
        // for it (invisible); skipping CDZ0900 restores that intent so `reject_code` surfaces the PATTERN/
        // program-error code (CDZ0201/CDZ0210/CDZ0101/…) the callers actually assert, not the boundary
        // not-yet. (No caller asserts a CDZ0900 via `reject_code`; the two CDZ0900 assertions read the
        // diagnostic directly.)
        out.diagnostics
            .iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .find(|d| d.code.as_deref() != Some("CDZ0900"))
            .and_then(|d| d.code.clone())
    })
}

/// The first error `Diagnostic` from compiling `src` (full record, so a test can read the carried
/// fix), or `None` if `src` compiled clean.
fn reject_full(src: &str) -> Option<crate::abi::Diagnostic> {
    let bytes = crate::codec::encode(&parse(src));
    crate::host::run_with_compiler_stack(|| {
        let out = compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                bytes,
            )],
            &[Target::Wasm],
        );
        if out.artifact(Target::Wasm.artifact_kind()).is_some() {
            return None;
        }
        out.diagnostics
            .into_iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
    })
}

/// Whether the component `bytes` imports the runtime op named `op` (a core-module import from the
/// `heap` interface). Used to assert the FBIP fast path emits NO `dup` for a single-use consume.
fn component_imports_op(bytes: &[u8], op: &str) -> bool {
    use wasmparser::{Parser, Payload, TypeRef};
    for payload in Parser::new(0).parse_all(bytes) {
        if let Ok(Payload::ImportSection(reader)) = payload {
            for import in reader.into_iter().flatten() {
                if matches!(import.ty, TypeRef::Func(_)) && import.name == op {
                    return true;
                }
            }
        }
    }
    false
}

/// A runtime `Qty` return over a MID-WIDTH Int inner (a width in 33..=63, here `(Int 40)` produced at
/// run time by a `.wrap` of an Int64 param) crosses the scalar-erased resource-escape as VALID wasm.
/// The `scalar_box` narrow-int i32→i64 extend must gate on the actual machine SLOT (`int_valtype`: ground
/// width ≤ 32 → I32), NOT `< 64`: a 33..63-bit inner is already an i64 slot, so extending it emitted
/// `i64.extend_i32_*` on an i64 → "type mismatch: expected i32, found i64" at `Component::new` (the Qty
/// slice-2 gate bug; a mid-width inner is reachable because every width `1..=64` is a valid `(Int N)`).
#[test]
fn a_runtime_mid_width_int_qty_return_crosses_as_valid_wasm() {
    let bytes = component(
        "(module m (def (main (: v Int64)) (Qty.of ((Int 40).wrap v) (Unit.base #\"meter\"))) (export main))",
    );
    let valid = wasmparser::validate(&bytes);
    assert!(
        valid.is_ok(),
        "mid-width Qty return must be valid wasm: {:?}",
        valid.err()
    );
    // A NARROW inner (≤ 32, an i32 slot) still crosses valid — the extend correctly fires there.
    let narrow = component(
        "(module m (def (main (: v Int64)) (Qty.of ((Int 16).wrap v) (Unit.base #\"meter\"))) (export main))",
    );
    let narrow_valid = wasmparser::validate(&narrow);
    assert!(
        narrow_valid.is_ok(),
        "narrow Qty return stays valid: {:?}",
        narrow_valid.err()
    );
}

#[test]
fn a_recursive_match_binder_materializes_its_scrutinee_once_not_per_use() {
    // REGRESSION (perf, S2-twin) — WHITE-BOX emit-count: a match/pattern BINDER used more than once must
    // NOT re-emit its whole scrutinee per use. When the scrutinee is a RECURSIVE CALL, a binder used K
    // times re-runs that call K times per recursion level → 2^depth runtime recompute (the pattern-binder
    // twin of the inline-tuple fall-through exponential). `f` recurses to `(Mk 1 1)` at n=0; each recursive
    // arm matches `(f (+ n 987654321))`, binds `a`, and uses it TWICE in `(Mk a a)`. FIX
    // (`scrutinee_reaches_recursive_call`, lower.rs): the single-arm `Leaf` fold keeps the `Core::MatchSum`
    // wrapper (materializing the scrutinee into ONE slot) when the scrutinee reaches a recursive call, so
    // it runs once per level — LINEAR. Without the wrapper the fold drops it and each payload binder (`a`)
    // resolves to a `Core::SumPayload` EMBEDDING the scrutinee, re-emitting the whole recursive call per
    // use → 2^depth. This pins the fix STRUCTURALLY: the recursive call's DISTINCTIVE argument constant
    // `987654321` (which lands at the scrutinee's emit site) must appear EXACTLY ONCE in the emitted module
    // — the recursive scrutinee is materialized once, not re-emitted per binder use. (Restores the perf
    // catch of the old run-based `..._is_materialized_once` deadline-trap guard, retired with the wasmtime
    // dev-dep during delanguaging; the run VALUE stays corpus-covered @09-functions. Verified this catches
    // the regression: neutralizing `scrutinee_reaches_recursive_call` → false makes the count 2 → fails.
    // wasm target; the Rust backend's `emit_sum_match` twin still re-emits — tracked separately.)
    let bytes = component(
        "(module m (type P (Mk Int64 Int64)) \
               (def (f (: n Int64)) (if (= n 0) (Mk 1 1) (match (f (+ n 987654321)) ((Mk a _) (Mk a a))))) \
               (def (main) (match (f -60) ((Mk x _) x))) (export main))",
    );
    let occurrences = super::count_opcode(
        &bytes,
        |op| matches!(op, wasmparser::Operator::I64Const { value } if *value == 987_654_321),
    );
    assert_eq!(
        occurrences, 1,
        "a recursive match binder used twice must materialize its recursive-call scrutinee ONCE, not \
             re-emit it per use (2^depth) — the scrutinee's distinctive constant 987654321 must emit exactly \
             once (found {occurrences}); a regression to per-use re-emission would duplicate it"
    );
}

// (a_char_literal_pattern_type_mismatch_and_non_exhaustion_reject migrated to corpus 13-strings, the
// Char-LITERAL patterns section: "a char-literal pattern over an Int scrutinee is a shape error" (CDZ0201)
// + "a wildcard-less char match is non-exhaustive (Char is an open type)" (CDZ0210) — the two rejects the
// section intro promised. --case grades both reject codes.)

#[test]
fn a_deeply_nested_option_pattern_lowers_in_bounded_time() {
    // REGRESSION (perf): the match decision-tree builder (`lower::build_tree`) threads a `PathTypes`
    // map (path → the sub-value's `Ty`) that `extend_path_types` CLONED whole at every nesting level so
    // sibling arms don't share a mutation. A deeply-NESTED pattern `(Some (Some … x))` descends `depth`
    // levels with a map that grows one entry per level, each value a `Ty` itself O(depth) deep — so the
    // per-level clone was O(depth²) and the whole build O(depth³) (depth-400: 7.5s, 52% in `Ty::clone`).
    // Two fixes: (1) `PathTypes` values are `Rc<Ty>` (the per-level map clone is a pointer-bump per
    // entry, not a deep `Ty` copy); (2) `const_at_path`'s per-step nominal-newtype check reads only the
    // type KIND via `infer::type_is_nominal` instead of cloning the whole `Ty`. Depth 60 would have been
    // well into the superlinear regime; that it lowers AND evaluates correctly is the gate.
    let mut val = String::from("0");
    let mut pat = String::from("x");
    for _ in 0..60 {
        val = format!("(Some {val})");
        pat = format!("(Some {pat})");
    }
    let src = format!("(module m (def (main) (match {val} ({pat} x) (_ -1))) (export main))");
    // The pattern matches the value exactly (60 `Some` layers around `0`), binding `x` to the innermost
    // `0` — so the result is `0`, NOT the `-1` fallback. Diagnostics must be clean and return quickly.
    // Through the host-stack guard the bin uses (`host.rs`): the decision-tree/fold walk recurses ~per
    // nesting level (60 deep), which SIGABRTs a default `cargo test` worker's ≈2 MB stack (EXIT=101,
    // 0 FAILED) even though it TERMINATES — deep-but-finite, not a loop (`RUST_MIN_STACK=64M` passes).
    // This test pins the COMPILE-PERF fix (bounded-time lowering of the deeply-nested pattern, no error
    // diagnostics) only; the value parity — a nested `(Some (Some x))` pattern binds the innermost —
    // is corpus-covered at shallow depth by spec/semantics/02-binding-and-control.sexp
    // "(match (Some (Some 5)) ((Some (Some x)) x) …)", so the run half is not needed here.
    let diags = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
    });
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a deeply-nested Option match lowers with no error diagnostics: {diags:?}"
    );
}

#[test]
fn a_generic_sum_types_args_are_rc_shared_so_a_clone_is_a_refcount_bump() {
    // REGRESSION (perf): `Ty::Sum { args }` (and `Ty::Nominal { args }`) held their type-ARGUMENTS in a
    // `Vec<Ty>`, so cloning the type deep-copied every arg. A GENERIC sum NESTED in another —
    // `(Option (Option … Int64))`, whose inner `Option` sits in the outer's `args` — thus deep-copied
    // the WHOLE nesting on each `Ty::clone`, and `payload_ty_at_instantiation`/`ty_at_path` clone once
    // PER match level → an O(depth) copy done O(depth) times = O(depth³) (a deep-Option-match param:
    // depth 800 was 610ms, ~3.9×/dbl; now 77ms, 8× faster). FIX: `args` is an `Rc<[Ty]>` (the sibling
    // of `Ty::Tuple`/`Ty::Record`/`Ty::Nominal::inner`), so a clone shares the slice — a refcount bump,
    // not a deep copy.
    //
    // Lock the REPRESENTATION directly: a clone of a `Ty::Sum` shares the SAME `args` allocation as the
    // original (`Rc::ptr_eq`). A revert to `Vec<Ty>` makes the clone a fresh allocation — `ptr_eq`
    // false — so this test fails. Deterministic, noise-free (no timing).
    use crate::ty::Ty;
    let inner = Ty::Sum {
        decl: crate::ast::StructId(7),
        args: std::rc::Rc::from([Ty::int64()]),
    };
    let outer = Ty::Sum {
        decl: crate::ast::StructId(7),
        args: std::rc::Rc::from([inner]),
    };
    let cloned = outer.clone();
    let (Ty::Sum { args: a0, .. }, Ty::Sum { args: a1, .. }) = (&outer, &cloned) else {
        panic!("both are Ty::Sum");
    };
    assert!(
        std::rc::Rc::ptr_eq(a0, a1),
        "cloning a Ty::Sum must SHARE its `args` Rc (a refcount bump), not deep-copy the Vec — the \
             `Rc<[Ty]>` representation that makes a nested-generic-sum clone O(1) instead of O(depth)"
    );

    // And end-to-end: a deeply-nested generic-Option PARAM match (the path through
    // `payload_ty_at_instantiation` → `unify` → `subst.apply`, which drove the O(depth³) deep-clone)
    // compiles cleanly and quickly. Depth 200 was well into the super-cubic regime before the fix.
    let depth = 200usize;
    let ty = {
        let mut t = String::from("Int64");
        for _ in 0..depth {
            t = format!("(Option {t})");
        }
        t
    };
    let pat = {
        let mut p = String::from("n");
        for _ in 0..depth {
            p = format!("(Some {p})");
        }
        p
    };
    let src = format!(
        "(module m (def (f (: o {ty})) (match o ({pat} n) (_ 0))) (def (main) 0) (export main))"
    );
    let diags = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
    });
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a deeply-nested generic-Option param match compiles with no error diagnostics: {diags:?}"
    );
}

#[test]
fn a_deeply_nested_match_lowers_without_a_cubic_constraint_reclone() {
    // REGRESSION (perf): even after the `PathTypes` `Rc<Ty>` fix (see
    // `a_deeply_nested_option_pattern_lowers_in_bounded_time`), `lower::build_tree` stayed O(depth³) on
    // a deeply-nested pattern via TWO further per-level whole-structure re-clones: (a) the PARTITION
    // loop rebuilt every surviving row's `constraints` list — each a `Vec<PathStep>` path — at every
    // one of `depth` levels (an O(depth)-long path deep-copied `depth` times = O(depth³)); (b)
    // `extend_path_types` CLONED THE WHOLE growing `path_types` map per arm per level. Three fixes:
    // the constraint/lit-test PATH is now `Rc<[PathStep]>` (per-level clone = pointer bump, like
    // `PathTypes`' `Rc<Ty>`), `build_tree` threads ONE shared `&mut PathTypes` with scoped
    // insert/restore instead of a per-arm map clone, and `shallowest_path` selects by reference.
    //
    // The guard is the GROWTH RATIO across a depth doubling, not an absolute wall-clock bound — a ratio
    // tests the complexity CLASS and is independent of profile (dev vs release) and machine speed, where
    // an absolute ceiling is not (the cubic factor only dominates at large depth, so a shallow absolute
    // bound catches nothing). Cubic lowering grows ~8× per doubling; the fixed quadratic-or-better grows
    // ~2–4×. The two depths are timed PAIRED and back-to-back, and we take the MIN ratio across several
    // pairs: measuring both depths in the same instant means transient CPU contention (under the
    // parallel test harness) hits them EQUALLY, so it cancels in the ratio — a single starved window
    // can't inflate one depth without the other. Threshold 6.0 sits between the two classes with margin
    // (fixed ~2–4×, cubic ~8×). Depths are large enough that the timed work dominates measurement noise,
    // but shallow enough to avoid the deep-recursion stack limit.
    fn build_src(depth: usize) -> String {
        let mut val = String::from("(None)");
        let mut pat = String::from("x");
        for _ in 0..depth {
            val = format!("(Some {val})");
            pat = format!("(Some {pat})");
        }
        format!(
            "(module m (type Opt (a) (Some Opt) (None)) \
                   (def (f (: o Opt)) (match o ({pat} 1) (_ 0))) \
                   (def (main) (f {val})) (export main))"
        )
    }
    fn lower_ms(src: &str, depth: usize) -> f64 {
        let start = std::time::Instant::now();
        let diags = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        assert!(
            diags
                .iter()
                .all(|d| d.severity != crate::abi::Severity::Error),
            "a {depth}-deep nested Some match lowers with no error diagnostics: {diags:?}"
        );
        ms
    }
    let (src200, src400) = (build_src(200), build_src(400));
    // Deep-but-finite recursion (per nesting level) → run under the compiler stack guard, like the
    // depth-60 test, so a default worker's ~2 MB stack does not SIGABRT on a terminating walk.
    let ratio = crate::host::run_with_compiler_stack(|| {
        lower_ms(&src200, 200); // warm lazy one-time init before the first timed pair
        let mut best = f64::INFINITY;
        for _ in 0..6 {
            let t200 = lower_ms(&src200, 200);
            let t400 = lower_ms(&src400, 400);
            best = best.min(t400 / t200.max(0.1));
        }
        best
    });
    assert!(
        ratio < 6.0,
        "a nested match's lowering must grow sub-cubically with depth (was O(depth³) via per-level \
             constraint/path-map re-clone): 200→400 grew {ratio:.1}× (min paired ratio); \
             cubic would be ~8×, the fix is ~2–4×"
    );
}

#[test]
fn a_recursive_fn_self_calling_in_a_do_def_rhs_is_detected_as_recursive_not_deep_beta_reduced() {
    // A recursive fn whose ONLY self-call sits in a do-local VALUE def's RHS — `(def hh (f …))` inside a
    // `(do …)` — must be detected as RECURSIVE by `is_recursive` so the caller types it by its def-scheme,
    // NOT by β-reducing REDUCE_DEPTH_LIMIT levels deep (which mis-types a heap-scalar BigInt recursive
    // result as kind `Type` → bogus CDZ0201 "member access requires a record, found Type" at the call
    // site; the result-used-twice spelling drove that descent to a >100s HANG). Root: `collect_callees`
    // did not descend into a do-def's value-RHS (the `def` form resolves to a declaration, treated as a
    // leaf), so the self-call edge was missed. Fixed by descending via `do_value_def_value`. This is a
    // BigInt-result program (heap-scalar — the shape that triggered it; scalar-Int64 β-reduces fine), so
    // the run-host needs the bigint runtime; the bug was a COMPILE-time decline, so the guard is that the
    // module VALIDATES (before the fix `component` panicked on the CDZ0201; the hang spelling never
    // returned). `(f 3 4 5)` with e:4→2→1→0 returns base=3.
    let bytes = component(
        "(module m \
               (def (f (: base BigInt) (: e Int64) (: md BigInt)) \
                 (if (= e 0) base (do (def hh (f base (/ e 2) md)) (% hh md)))) \
               (def (main) (f (BigInt.of 3) 4 (BigInt.of 5))) (export main))",
    );
    let valid = wasmparser::validate(&bytes);
    assert!(
        valid.is_ok(),
        "a recursive fn self-calling in a do-def RHS must type by its scheme (valid wasm), not mis-type via deep β-reduce: {:?}",
        valid.err()
    );
}

#[test]
fn a_constant_bigint_newtype_passed_as_a_recursive_call_arg_emits_a_handle_not_a_raw_i64() {
    // FACE-B of the nonzero-BigInt-recursive miscompile (an INVALID-MODULE bug, so the guard is that the
    // emitted bytes VALIDATE): `(type W (Mk BigInt))` is a single-variant single-payload NEWTYPE, so it
    // erases to `Ty::Nominal { inner: BigInt }`. A CONSTANT `(Mk 7)` passed as a RUNTIME call arg is a
    // `Core::ConstInt` typed `Nominal{BigInt}`; a BigInt sum is a BOXED i32 HANDLE, so the arg must
    // materialize via `bigint-of-i64`, NOT a raw `i64.const` (which mismatched the i32 handle param →
    // `expected i32, found i64` at the call → invalid module). `is_bigint_valued` now peels the nominal
    // (`peel_qty_ty`) so the `Core::ConstInt` emit routes through the leaf-handle path. Before the fix,
    // `component` (which asserts the backend validates) panicked on the func-validation failure.
    let bytes = component(
        "(module m (type W (Mk BigInt)) \
               (def (walk (: n Int64) (: w W)) (match w ((Mk v) (if (>= n 0) (walk (- n 1) w) v)))) \
               (def (main) (walk 0 (Mk 7))) (export main))",
    );
    let valid = wasmparser::validate(&bytes);
    assert!(
        valid.is_ok(),
        "a constant BigInt-newtype passed as a recursive call arg must emit a boxed handle (valid wasm): {:?}",
        valid.err()
    );
}

#[test]
fn file_of_binary_search_maps_every_files_nodes_to_the_right_file() {
    // REGRESSION (perf + correctness): `FileScopeTable::file_of` maps a node id to its package file,
    // consulted on EVERY file-scoped resolution (`file_scoped_def`/`_type`/`_variant_ctor`). It was a
    // linear `files.iter().position(|f| f.contains(id))` → O(files) per lookup → O(N²) over a package
    // of N files. Files are appended sequentially at link (each `struct_base = structure.len()` at its
    // turn), so the ranges are ascending + non-overlapping and `file_of` is now a BINARY SEARCH
    // (`partition_point` on `struct_base`, then confirm `contains`). This pins the binary search returns
    // the SAME file the linear scan did for a node in EACH file (a broken search would misattribute a
    // node to the wrong file → cross-file privacy / resolution breaks), and correctly returns `None` for
    // a node in no file (the link-synthesized `(do …)` root, which sits outside every file's range).
    let n = 40;
    let files: Vec<(String, crate::ast::Arenas)> = (0..n)
        .map(|i| {
            (
                format!("f{i}"),
                parse(&format!("(do (def (g{i}) {i}) (export g{i}))")),
            )
        })
        .collect();
    let linked = crate::link::link(&files, "f0").expect("package links");
    let db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
    // Each file's FIRST and LAST node id must map back to that file's own index — a broken binary
    // search (wrong bound / off-by-one) would misattribute a boundary node to a neighbouring file.
    for (fi, fs) in linked.files.iter().enumerate() {
        let base = crate::ast::StructId(fs.struct_base);
        let last = crate::ast::StructId(fs.struct_base + fs.struct_count - 1);
        assert_eq!(
            db.file_of(base),
            Some(fi),
            "file {fi}'s base node maps to {fi}"
        );
        assert_eq!(
            db.file_of(last),
            Some(fi),
            "file {fi}'s last node maps to {fi}"
        );
    }
}

#[test]
fn a_lowercase_unit_variant_payload_is_ty_unit_not_a_spurious_type_param() {
    use crate::testkit::parse;
    // The pervasive nullary-variant idiom `(None unit)` / `(Nil unit)` writes a unit-typed payload with
    // the lowercase `unit` VALUE (prelude empty product). `collect_type_params` must NOT harvest it as a
    // type param, and `typeval_of` maps a type-position `unit` → `Ty::Unit` (ruling A) — so
    // `(type (Box a) (Full a) (Nil unit))` has EXACTLY ONE param `a` (was 2: `[a, unit]`, whose unfilled
    // phantom left a stray Var making the sum non-Eq/non-Ord → a Set/Map of it DECLINED on the rust
    // backend). Value side: a match over both variants runs, WITH the lowercase idiom intact (no
    // migration to capital `Unit`).
    // (The value-side run — matching `(Full 6)` over both variants with the lowercase idiom intact,
    // exactly one param → 7 — is corpus 05 "a user generic sum whose nullary variant carries a
    // lowercase-unit payload has exactly one type param"; this test keeps the rust-EMIT witness below.)
    // The rust-decline witness: a Set of the generic lowercase-unit-payload sum must EMIT rust (the
    // spurious stray Var previously left it non-Ord → "Set with a non-Ord element"). Exactly the shape
    // the v-rust-backend ask flagged + the guide/playground idiom the (B) attempt's reject exposed; a
    // HARD emit check on the RUST target (storeless-safe — no run).
    let src = "(module m (type (Box a) (Full a) (Nil unit)) \
                   (def (main (: k Int64)) (Set.len (Set.of (list (Full 1) (Full k) (Nil unit))))) (export main))";
    let mut dbr = crate::db::Db::load(parse(src));
    let layr = crate::layout::compute(&mut dbr).expect("layout");
    crate::backend::emit(crate::backend::Target::Rust, &mut dbr, &layr, None, None).expect(
            "a Set of a generic lowercase-unit-payload sum must emit rust — no spurious stray-Var non-Ord decline",
        );
}

// (a_wrong_arity_generic_in_a_variant_payload_is_cdz0203_at_the_declaration migrated to corpus
// 07-type-system, next to the annotation-position wrong-arity generic block: a wrong-arity user generic
// `(Wrap (Box Int64 Bool))` / built-in `(Wrap (Option Int64 Bool))` in a variant payload → CDZ0203
// (message "`<Name>` takes 1 type argument")(message "but 2 were supplied") AT THE DECLARATION (was
// silently accepted — the user generic reduces to a Ty::Sum dropping the extra arg — surfacing only later
// as a confusing construction-site CDZ0201) + 2 running controls: a right-arity payload constructs +
// matches (→ 5), a param-parameterized `(Box a)` payload inside another generic is valid (→ 0). --case
// grades the reject codes + messages + run values (all 4 PASS).)

// (a_wrong_arity_generic_type_application_in_an_annotation_is_cdz0203_for_user_and_builtin migrated to
// corpus 07-type-system, the OVER/WRONG-arity generic block: built-in Option over-applied (2 args),
// user Box over-applied (2 args), multi-param Pair under-applied (1 of 2, plural "arguments"), and Box
// applied with 0 args — each CDZ0203 with (message "`<Name>` takes N type argument(s)")(message "but M
// was/were supplied"). --case grades the code + both message substrings (all 4 verified PASS); the arity
// check is uniform for a user generic and a built-in, the #1683 path.)

#[test]
fn a_parenthesized_head_generic_type_name_is_visible_to_the_export_reader() {
    // #1683 review gap (2): sibling `(type …)`-tail name-readers (the linker's `top_item_defined_name`,
    // used for export/import name resolution) must decode the parenthesized `(type (Box a) …)` head via
    // the shared `Arenas::type_decl_head_name`, not a bare `tail.first().as_name()` (which returns None
    // for the `(Box a)` list head → the type was invisible/un-exported). A bare `(export Box)` of the
    // generic type NAME now RESOLVES the name (it reaches the abstract-type-handle export path, whose
    // single-module message names `Box` as a TYPE — proving the name resolved — rather than an
    // "unknown"/absent). The message naming `Box` is the witness that the export reader saw the name.
    let msg = compile_component(&crate::codec::encode(&crate::testkit::parse(
        "(module m (type (Box a) (Mk a)) (def (main) 0) (export main) (export Box))",
    )))
    .expect_err("a bare type-handle export in a single module reports the no-importer message")
    .message;
    assert!(
        msg.contains("`Box` names a TYPE") || msg.contains("abstract-type"),
        "the export reader resolved the parenthesized-head generic name `Box`, got: {msg}"
    );
}

#[test]
fn comparing_a_newtype_to_its_underlying_type_is_a_nominal_boundary_error() {
    // The bare CDZ0202 comparison rejects (newtype vs its erased inner, either operand order, and a
    // generic newtype vs its bare instantiated inner) migrated to corpus 05-compound-types (the newtype
    // NOMINAL-BOUNDARY reject block). What STAYS here is the ACTIONABLE-FIX half, which carries a novel
    // unwrap `(match … ((Mk n) n))` Wrap fix + its no-fix negative + a round-trip compile — facets the
    // corpus (error …) surface grades only via the nix corpus-grade, kept in rust as the fix-quality pin.
    // ACTIONABLE FIX (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix): the
    // message says "unwrap the nominal", and for an ERASABLE SINGLE-VARIANT newtype the unwrap is the
    // total, unambiguous `(match <it> ((<Variant> n) n))` — so it now carries that WRAP fix on the
    // newtype operand. Wraps whichever operand IS the newtype, either order.
    let d = reject_full(
        "(module m (type UserId (Mk Int64)) (def (f (: u UserId)) (= u 5)) (export f))",
    )
    .expect("newtype-vs-inner comparison rejects");
    assert_eq!(d.code.as_deref(), Some("CDZ0202"), "got: {}", d.message);
    let fix = d.fix.as_ref().expect("the unwrap fix is carried");
    assert_eq!(fix.kind, crate::abi::FixKind::Wrap);
    assert!(
        fix.replacement.contains("match") && fix.replacement.contains("((Mk n) n)"),
        "the fix unwraps via a `(match … ((Mk n) n))`: {:?}",
        fix.replacement
    );
    // The unwrap-applied program compiles (the unwrap is total for a single-variant newtype).
    assert!(
            crate::compile::compile_component(&crate::codec::encode(&parse(
                "(module m (type UserId (Mk Int64)) (def (f (: u UserId)) (= (match u ((Mk n) n)) 5)) (export f))"
            )))
            .is_ok(),
            "the suggested unwrap type-checks"
        );
    // NO FIX for a MULTI-variant sum vs a bare value — no single, unambiguous unwrap exists (and it is
    // not an erasable newtype), so it stays the generic mismatch with no misleading unwrap.
    let multi =
        reject_full("(module m (type W (A Int64) (B Int64)) (def (f (: w W)) (= w 5)) (export f))")
            .expect("a multi-variant sum vs a bare value rejects");
    assert!(
        multi.fix.is_none(),
        "a multi-variant sum offers no unwrap fix: {} fix={:?}",
        multi.message,
        multi.fix
    );
}

// (an_absent_record_field_access_is_cdz0212_like_record_project migrated to corpus 15-rows-and-open-sums,
// next to "projecting a record onto an absent field is rejected": a `.`-access of an absent field on a
// genuine record is CDZ0212 (the Record.project twin — same user error, same code, not the generic
// CDZ0201), for both a direct and a let-bound record; the narrow-scope guard is the two contrasts — a
// module MEMBER miss and a sum-type VARIANT miss stay CDZ0201 (their own category word). --case grades
// all 4 reject codes.)

// (a_generic_newtype_at_two_instantiations_stays_distinct migrated to corpus 05-compound-types, in the
// newtype nominal-boundary block: "a generic newtype at two DIFFERENT instantiations stays distinct"
// ((type Box (Mk a)), (= (Mk 1) (Mk true)) → CDZ0203 — Box Int64 ≠ Box Bool, per-instantiation inner keeps
// them apart; a nominal-vs-nominal clash stays CDZ0203, vs the newtype-vs-untagged-inner CDZ0202 sibling).)

#[test]
fn a_newtype_over_a_sum_erases_to_the_same_component_as_the_bare_sum() {
    // The proof there is NO double-box: a newtype-over-Option compiles to the BYTE-IDENTICAL component
    // as the bare Option it wraps — the `Mk` tag erased to nothing. (A constant fold makes both a
    // single resource; the point is they are indistinguishable, i.e. the wrapper added zero.)
    let bare = component(
        "(module m (def (main) (match (Some 5) ((Some n) n) ((None _) 0))) (export main))",
    );
    let wrapped = component(
        "(module m (type Cached (Mk (Option Int64))) \
               (def (main) (match (Cached.Mk (Some 5)) ((Mk o) (match o ((Some n) n) ((None _) 0))))) (export main))",
    );
    assert_eq!(
        bare, wrapped,
        "the newtype-over-sum wrapper must erase to nothing"
    );
}

// (a_newtype_over_a_record_still_rejects_a_missing_field + a_newtype_wrong_constructor_pattern_is_a_type_error
// migrated to corpus 05-compound-types, the newtype NOMINAL-BOUNDARY reject block: "a newtype over a
// record still rejects a missing field through the tag" (`.z` on a newtype over (Record (x …)) → CDZ0212)
// + "a wrong-constructor pattern over a newtype scrutinee is a type error" ((Some n) over UserId → CDZ0203).
// --case grades the reject codes (both PASS).)

// (a_multi_payload_pattern_of_wrong_arity_is_rejected migrated to corpus 02-binding-and-control, as the
// CONSTRUCTOR twin of the tuple-pattern-arity family: "a constructor pattern of the wrong arity is a type
// error naming the field count" ((Pair.Mk a b c) on a 2-field Mk → CDZ0201 (message "this pattern binds 3
// elements for `Mk`, but `Mk` carries 2 fields")) + "a multi-binder pattern on a single-value constructor
// points at the one-sub-pattern form" ((Mk x y) on (Mk Int64) → CDZ0201 (message "`Mk` carries a single
// value of type Int64 — bind it with one sub-pattern `(Mk x)`")). --case grades code + message (both PASS).
// The NOT-"payload" negative is the corpus-inexpressible remainder, covered by the positive field-count message.)

// (an_exported_closure_body_is_type_checked migrated to corpus 21-host-closures, the exported-closure
// body-type-check block: annotation-mismatch body `(fn ((: x Int64)) (: x Bool))` → CDZ0203, narrow-arg
// wide-result body `(fn ((: x Int8)) (: (+ x 100) Int64))` → CDZ0203, and the arithmetic non-numeric-operand
// body `(fn ((: x Int64)) (+ x true))` → CDZ0203 (all three exercise the same closure-export body
// `type_errors`-before-emit soundness path). The NO-OVER-REJECTION positive control is covered by the many
// well-typed exported-closure running cases in 21-host-closures (make/call over `(+ x 1)`, `(* x 3)`, …).)

#[test]
fn a_bakeable_type_valued_export_crosses_the_boundary() {
    // A Type is a FIRST-CLASS value that can be returned and inspected at run time (core-semantics.md
    // §Types Are First-Class Values). A NULLARY export whose type-value reduces to a concrete type —
    // `(def (main) Int64)` — CROSSES the boundary via the constant value-form escape: `constant_value_form`
    // bakes `(: Int64 Type)` from the reduced type (the type is fully compile-time-known, its runtime
    // footprint nil). So it compiles CLEAN — no error, no residual no-runtime-form decline (the cascade
    // that once fired is dropped by `dedup_faults`'s bakeable-type-export gate, since the escape, not a
    // reject, is the answer). It runs to `(: Int64 Type)` (see the `07-type-system` corpus case).
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse("(module m (def (main) Int64) (export main))")),
        )],
        &[crate::backend::Target::Wasm],
    );
    let errors: Vec<&crate::abi::Diagnostic> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "a bakeable (nullary, concrete) type export compiles clean, got: {:?}",
        out.diagnostics
    );
    // No residual no-runtime-form declines leak either (the bakeable-export gate drops them).
    assert!(
        !out.diagnostics.iter().any(|d| {
            matches!(
                d.message.as_str(),
                crate::diag::TYPE_VALUE_NO_RUNTIME_DECLINE
                    | crate::diag::NULLARY_LAMBDA_NO_CLOSURE_DECLINE
                    | crate::diag::PRIM_AS_VALUE_DECLINE
            )
        }),
        "no no-runtime-form decline accompanies a bakeable type export: {:?}",
        out.diagnostics
    );
}

#[test]
fn an_exported_unannotated_param_surfaces_in_check_with_an_annotate_fix() {
    // An exported def with an unannotated param `(def (f x) …)` has an ambiguous boundary parameter —
    // it MUST be reported by the always-run `Diagnostics` set (`cdz check`), not only the emit path
    // (where `layout::export_params` declined it, invisible to `check` — the check-vs-emit gap). It
    // now carries the rustc-gold "add a type annotation" fix: WRAP `x` → `(: x Int64)`.
    // Read the always-run `Diagnostics` set (what `cdz check` runs) — this fault is surfaced there
    // (collect_faults), whereas the EMIT path reports layout's coarser decline first, so use the check
    // path to see the coded CDZ0201.
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f x) (+ x 1)) (export f))",
    )));
    let d = diags
        .iter()
        .find(|d| d.code.as_deref() == Some("CDZ0201"))
        .expect("an exported unannotated param must be reported in check");
    assert!(
        d.message.contains("parameter type is ambiguous"),
        "names the ambiguity: {}",
        d.message
    );
    let fix = d.fix.as_ref().expect("carries an annotate fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Wrap);
    assert_eq!(
        fix.replacement,
        format!("(: {} Int64)", crate::abi::WRAP_HOLE),
        "wraps the bare param in a type annotation"
    );
    assert!(!fix.verified, "the concrete type is a heuristic guess");
    // NO OVER-REPORT: a NON-exported unannotated param (it inlines at call sites) is NOT flagged.
    let clean = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (helper x) (+ x 1)) (def (main) (helper 5)) (export main))",
    )));
    assert!(
        !clean.iter().any(|d| d.code.as_deref() == Some("CDZ0201")),
        "a non-exported unannotated param inlines — not a boundary ambiguity: {clean:?}"
    );
}

#[test]
fn an_exported_annotated_char_param_names_the_no_boundary_representation_not_ambiguous() {
    // DIAGNOSTIC QUALITY (v-property-testing's scalar-Char gap): an exported param annotated with a
    // type that HAS no component-boundary representation (`Char`) must NOT report "ambiguous — annotate
    // it" — the param IS annotated, so that advice is misleading (sends the author to add an annotation
    // that is already present). The message must instead NAME the type and say it has no boundary
    // representation. The unannotated-`Any` case still says "ambiguous" (the sibling test above).
    let msg = compile_component(&crate::codec::encode(&parse(
        "(module m (def (f (: c Char)) 1) (export f))",
    )))
    .expect_err("a Char boundary param must decline")
    .message;
    assert!(
        msg.contains("Char") && msg.contains("no component-boundary representation"),
        "an annotated Char export param names the type + the boundary-rep cause, not ambiguity: {msg}"
    );
    assert!(
        !msg.contains("ambiguous"),
        "an ANNOTATED param must not be called ambiguous — the annotation is present: {msg}"
    );
}

#[test]
fn a_non_recursive_scalar_match_scrutinee_param_is_grounded_not_declined_heap_walk() {
    // #6426's dedup-unmask exposed CDZ0900 "matching a compound value needs a heap walk" on an
    // unannotated SCALAR-match parameter of a NON-recursive def: the param stayed `Any` (a non-recursive
    // param is normally left to inline at its call site, not solved from its body), so `is_scalar` failed
    // at lowering and the scalar match declined as if compound. A standalone (non-inlined) body needs the
    // scalar type: `infer::nonrec_scalar_scrutinee_ty` grounds a param used as a scalar-literal match
    // scrutinee — `(match n (0 …) …)` ⇒ Int64 — so the scalar probe-chain routes it. Compiles clean now.
    fn heap_walk_declined(src: &str) -> bool {
        crate::host::run_with_compiler_stack(|| {
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(src)),
                )],
                &[Target::Wasm],
            );
            out.diagnostics.iter().any(|d| {
                d.message
                    .contains("matching a compound value needs a heap walk")
            })
        })
    }
    assert!(
        !heap_walk_declined(
            "(module m (def (g n) (match n (0 100) (_ 1))) (def (main) (g 0)) (export main))"
        ),
        "an integer-literal scalar match on a non-recursive param grounds Int64 (no heap-walk decline)"
    );
    // A GUARD-only scalar match: `(guard v (>= v 60))` binds the whole scrutinee and the guard's `>=`
    // pins it Int — grounded even with NO literal-patterned arm (the guard-bound-scrutinee case).
    assert!(
        !heap_walk_declined(
            "(module m (def (grade s) (match s ((guard x (>= x 60)) 1) (_ 0))) (def (main) (grade 90)) (export main))"
        ),
        "a guard-only scalar match grounds Int from the guard's comparison (no heap-walk decline)"
    );
    // NEGATIVE (the narrowness that avoids the Ast-reflection over-ground regression): a genuine COMPOUND
    // (tuple-pattern) match is NOT scalar-grounded — its exported param stays ambiguous (CDZ0201), never
    // silently pinned to a concrete type that would mis-route a sum/compound match's decision tree.
    assert_eq!(
        reject_code("(module m (def (f p) (match p ((tuple 1 2) 10) (_ 0))) (export f))")
            .as_deref(),
        Some("CDZ0201"),
        "a tuple-pattern match must NOT be scalar-grounded — the boundary param stays ambiguous"
    );
}

#[test]
fn a_misspelled_export_does_not_also_flag_its_intended_target_unused() {
    // A near-miss export typo (`(export mian)` for `(def (main) …)`) has ONE real defect — the export
    // names no definition (CDZ0101, "did you mean `main`?"). The intended target `main` must NOT ALSO
    // draw a CDZ0306 "unused definition" — the author clearly meant to export it (they misspelled the
    // export), so "unused" is consequent, misleading noise. Suppressed: a def a MISSING export names as
    // its nearest match counts as intended-for-export.
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (main) 1) (export mian))",
    )));
    assert!(
        diags
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0101")
                && d.message.contains("did you mean `main`?")),
        "the export typo is still reported: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        !diags
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0306") && d.message.contains("`main`")),
        "no spurious 'unused definition `main`' — it is the intended export target: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // NO OVER-SUPPRESSION: a FAR-miss export (no near def) still lets a genuinely-unused def warn — the
    // suppression fires only when the export offers that exact def as its "did you mean?".
    let far = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (main) 1) (export zzzzz))",
    )));
    assert!(
        far.iter()
            .any(|d| d.code.as_deref() == Some("CDZ0306") && d.message.contains("`main`")),
        "a far-miss export does not suppress the genuinely-unused def: {:?}",
        far.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // And a def unrelated to any export error still warns unused (baseline unaffected).
    let plain = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (helper) 1) (def (main) 2) (export main))",
    )));
    assert!(
        plain
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0306") && d.message.contains("`helper`")),
        "an ordinary unused def still warns: {:?}",
        plain.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// (a_bare_declaration_keyword_form_declares_nothing_is_rejected migrated to corpus 11-modules: "a bare
// {def,type,effect} declaration keyword form declares nothing and is rejected" — each bare (def)/(type)/
// (effect) → CDZ0201 (message "declares nothing")(message "`(<kw>)`"). --case grades the code + both
// message substrings (all 3 PASS). The no-false-positive controls (a well-formed def/type/effect is not
// flagged) are covered vacuously by the corpus at large — every case carries well-formed declarations.)

// (a_malformed_top_level_annotation_names_the_annotation_shape_not_an_unbound_at migrated to corpus
// 09-functions, next to "an unrecognized annotation leaves its wrapped definition in effect": the five
// malformed top-level `(@ …)` shapes (name-only, empty, non-form target, non-def list target, malformed
// inner def) each → CDZ0201 (message "annotation wraps no definition")(message "`(@ <name> (def …))`").
// --case grades the code + both message substrings (all 5 PASS; the malformed-inner-def case uses the
// main-valid form so the annotation CDZ0201 is the sole fault, not an unbound-export CDZ0101). The NOT-
// "unbound name `@`" negative is the corpus-inexpressible remainder, covered by the positive shape message;
// the no-false-positive controls are covered by the transparent-unknown-annotation case above.)

/// Verification Inc-b b4a: `@requires(pred)`/`@ensures(pred)` are RECORDED (their predicate occurrences
/// keyed by the def's body occ, `Db::requires_of`/`ensures_of`) — the `@requires`/`@ensures`→node
/// channel the proof-guided-elision oracle needs. The recording is unchanged by the (D) enforcement:
/// `verify_enforce` rewrites the def BODY but leaves the `(@ (requires …) …)` wrapper in place, so
/// `strip_annotations` still records the predicate.
///
/// This unit pins only the RECORDING (a pure-fn invariant: `db.requires_of`/`ensures_of` see the
/// predicates). The (D) run-time ENFORCEMENT behavior — a violated `@requires` traps, a satisfying input
/// is value-transparent, and enforcement descends through a stacked `@ensures` wrapper — is pinned
/// end-to-end in the corpus (`26-program-conditions.sexp`: "@requires stacked OVER @ensures: the
/// precondition is still enforced …" for `(f -5)` -> trap, and the @requires/@ensures satisfying family
/// for the value-transparent path), so it no longer needs a wasmtime run here.
#[test]
fn requires_ensures_predicates_are_recorded() {
    use crate::testkit::parse;
    // A def carrying a @requires and a @ensures. The predicates are ordinary forms over the param `x`
    // (and, for @ensures, the implicit result binder `ret`); b4a just records their StructIds.
    let src = "(module m (@ (requires (> x 0)) (@ (ensures (> ret 0)) (def (f (: x Int64)) (+ x 1)))) (export f))";
    let db = crate::db::Db::load(parse(src));
    // The annotations are RECORDED against f's def (unchanged by the (D) enforcement — the wrapper stays).
    let f = db.def_by_name("f").expect("def f");
    assert_eq!(
        db.requires_of(f).len(),
        1,
        "the @requires(> x 0) predicate is recorded for f"
    );
    assert_eq!(
        db.ensures_of(f).len(),
        1,
        "the @ensures(> ret 0) predicate is recorded for f"
    );
}

/// Verification Inc-b @invariant (design §10, DATA-level family member): `@invariant(pred)` on a `(type …)`
/// declaration is RECORDED (keyed by the type decl occ, over the value binder `self`) and read by
/// `Db::invariant_of` — v-property-testing's `gen<T>` seam. Unlike `@requires`/`@ensures` it annotates a
/// TYPE, not a def, so it must NOT trip the "annotation wraps no definition" reject; the type still takes
/// effect (a value of it constructs + runs). Behavior-neutral recording (the establish/preserve enforcement
/// + elision are later slices).
#[test]
fn an_invariant_on_a_type_is_recorded_and_the_type_still_works() {
    use crate::testkit::parse;
    // `Percent` carries an `@invariant(and (>= it 0) (<= it 100))`; a value constructs + the program runs.
    let src = "(module m \
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64))) \
            (def (mk (: v Int64)) (Percent.Pct v)) \
            (def (unwrap (: p Percent)) (match p ((Percent.Pct n) n))) \
            (def (main) (unwrap (mk 42))) (export main))";
    let db = crate::db::Db::load(parse(src));
    // The invariant is RECORDED against the `Percent` type decl occ (not a def) — read by invariant_of.
    let percent = db
        .type_decls
        .iter()
        .find(|t| t.name == "Percent")
        .expect("type Percent")
        .occ;
    assert!(
        db.invariant_of(percent).is_some(),
        "the @invariant predicate is recorded for the Percent type declaration"
    );
    // A type WITHOUT an @invariant records nothing.
    let no_inv = "(module m (type Plain (P Int64)) (def (main) 0) (export main))";
    let db2 = crate::db::Db::load(parse(no_inv));
    let plain_ty = db2
        .type_decls
        .iter()
        .find(|t| t.name == "Plain")
        .expect("type Plain")
        .occ;
    assert!(
        db2.invariant_of(plain_ty).is_none(),
        "a type with no @invariant records no predicate"
    );
    // BEHAVIOR-NEUTRAL (the @invariant-annotated Percent type still constructs + unwraps + runs — the
    // wrapper is consumed at strip, the type declaration survives) is corpus 14b "an @invariant newtype
    // is constructed from PERFORM results" (identical `@invariant Percent` + `mk`/`unwrap`, runs to 85)
    // and 14 §effects. This keeps only the white-box recording witness (`invariant_of`, no runtime).
}

/// Verification Inc-b @invariant ESTABLISH Part 1: `invariant_establish::synthesize` emits a typed checker
/// def per @invariant type, so the predicate is TYPE-CHECKED with `it : T`. For a SINGLE-PAYLOAD NEWTYPE it
/// AUTO-UNWRAPS — the bare `(>= it 0)` (which would hit the nominal boundary on `it : Percent`) type-checks
/// because the checker binds the payload and rewrites `self` to it. A predicate that ALREADY destructures
/// `self` is used as-is (NOT double-unwrapped). Both forms compile clean (no CDZ0202/CDZ0203).
#[test]
fn an_invariant_on_a_newtype_auto_unwraps_so_a_bare_scalar_predicate_type_checks() {
    use crate::testkit::parse;
    // BARE `(>= it 0)` on `it : Percent` (a single-payload newtype) — auto-unwrapped, type-checks clean.
    let bare = "(module m (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64))) \
            (def (mk (: v Int64)) (Percent.Pct v)) (def (main) 0) (export main))";
    let ds = crate::diagnostics(&mut crate::db::Db::load(parse(bare)));
    assert!(
        !ds.iter()
            .any(|d| matches!(d.code.as_deref(), Some("CDZ0202") | Some("CDZ0203"))),
        "a bare scalar @invariant on a newtype auto-unwraps — no nominal-boundary fault: {ds:?}"
    );
    // A SELF-DESTRUCTURE `(match self ((Percent.Pct v) …))` predicate is used as-is (not double-unwrapped —
    // that would try to match the payload Int64 as a Percent and fail CDZ0203). Also clean.
    let destr = "(module m (@ (invariant (match self (((. Percent Pct) v) (and (>= v 0) (<= v 100))))) \
            (type Percent (Pct Int64))) (def (main) 0) (export main))";
    let ds2 = crate::diagnostics(&mut crate::db::Db::load(parse(destr)));
    assert!(
        !ds2.iter()
            .any(|d| matches!(d.code.as_deref(), Some("CDZ0202") | Some("CDZ0203"))),
        "a self-destructuring @invariant is used as-is (not double-unwrapped) — clean: {ds2:?}"
    );
}

/// Verification Inc-b @invariant ESTABLISH over a MULTI-VARIANT sum: `invariant_establish` synthesizes a
/// per-variant checked constructor (`__invariant_construct_Shape__d<disc>`, keyed by discriminant) for a
/// ≥2-variant sum carrying an `@invariant`, and both are indexed by name (the callee the boxed-construction
/// divert routes to). This pins the SYNTHESIS + name-indexing at the unit level; the run-time establish
/// behavior (a satisfying value constructs, a violating value traps, per variant, including the 2-payload
/// arm) is a heap-constructing run pinned in the corpus (`26-program-conditions.sexp`), which links the
/// value-heap runtime the boxed `Core::SumNew` needs.
#[test]
fn an_invariant_on_a_multi_variant_sum_synthesizes_a_per_variant_checked_constructor() {
    use crate::testkit::parse;
    let src = "(module m \
            (@ (invariant (match self (((. Shape Circle) r) (> r 0)) \
                (((. Shape Square) w h) (and (> w 0) (> h 0))))) \
             (type Shape (Circle Int64) (Square Int64 Int64))) \
            (def (main) 0) (export main))";
    let db = crate::db::Db::load(parse(src));
    // One checked constructor per variant, keyed by discriminant (Circle=0, Square=1).
    assert!(
        db.def_by_name("__invariant_construct_Shape__d0").is_some(),
        "the Circle (disc 0) variant gets a checked constructor"
    );
    assert!(
        db.def_by_name("__invariant_construct_Shape__d1").is_some(),
        "the Square (disc 1) variant gets a checked constructor"
    );
    // The whole-value checker (Part 1) is the callee both construct-defs invoke.
    assert!(
        db.def_by_name("__invariant_check_Shape").is_some(),
        "the whole-value checker is synthesized for the multi-variant sum"
    );
    // A multi-variant sum is NOT a single-payload newtype, so no bare `__invariant_construct_Shape`.
    assert!(
        db.def_by_name("__invariant_construct_Shape").is_none(),
        "a multi-variant sum has per-variant construct-defs, not a single bare one"
    );
}

/// Verification Inc-b @invariant ESTABLISH over a SINGLE-VARIANT MULTI-PAYLOAD newtype (`(type T (Mk A B))`).
/// Such a type erases to a `Ty::Tuple` (not a single-payload value), so it takes neither the bare
/// `__invariant_construct_T` (that is only the single-PAYLOAD newtype) nor is it a ≥2-variant sum — it is the
/// third shape. `invariant_establish` synthesizes its sole variant's checked constructor as `__d0` (the
/// per-variant path now fires for any non-sole-payload-newtype, including a single-variant multi-payload
/// one), and the tuple-erase arm of `lower_sum_new` diverts through it. This pins the SYNTHESIS; the run
/// behavior (a satisfying pair constructs, a violating one traps) is a corpus case.
#[test]
fn an_invariant_on_a_single_variant_multi_payload_newtype_synthesizes_a_checked_constructor() {
    use crate::testkit::parse;
    let src = "(module m \
            (@ (invariant (match self (((. Range Mk) lo hi) (<= lo hi)))) (type Range (Mk Int64 Int64))) \
            (def (main) 0) (export main))";
    let db = crate::db::Db::load(parse(src));
    // The sole variant (disc 0) gets a per-variant checked constructor — the tuple-erase divert's callee.
    assert!(
        db.def_by_name("__invariant_construct_Range__d0").is_some(),
        "the single (2-payload) variant gets a checked constructor keyed by disc 0"
    );
    assert!(
        db.def_by_name("__invariant_check_Range").is_some(),
        "the whole-value checker is synthesized"
    );
    // It is NOT a single-PAYLOAD newtype, so no bare `__invariant_construct_Range`.
    assert!(
        db.def_by_name("__invariant_construct_Range").is_none(),
        "a multi-payload newtype uses the per-variant `__d0`, not the bare single-payload construct-def"
    );
}

/// Verification Inc-b @invariant ESTABLISH over a NULLARY variant: a nullary variant carries no payload but
/// is still a VALUE of the type, so its construction must satisfy the invariant. An `@invariant` that
/// rejects the nullary variant (`(match it (((. T A)) false) …)`, making `A` uninhabitable) must TRAP when
/// `A` is constructed. `invariant_establish` synthesizes a no-arg checked constructor for it
/// (`__invariant_construct_T__d0`), and the nullary-unit path of `lower_sum_new` diverts through it. This
/// pins the SYNTHESIS (a nullary variant gets a construct-def now, no longer skipped); the run behavior (the
/// rejected nullary traps, the accepted payload variant constructs) is a corpus case. This closes the LAST
/// ESTABLISH shape — every variant kind (single/multi-payload newtype, multi-variant, nullary) now
/// establishes at construction.
#[test]
fn an_invariant_rejecting_a_nullary_variant_synthesizes_a_checked_constructor_for_it() {
    use crate::testkit::parse;
    let src = "(module m \
            (@ (invariant (match self (((. T A)) false) (((. T B) x) (> x 0)))) (type T (A) (B Int64))) \
            (def (main) 0) (export main))";
    let db = crate::db::Db::load(parse(src));
    // The nullary variant A (disc 0) gets a no-arg checked constructor — no longer skipped.
    assert!(
        db.def_by_name("__invariant_construct_T__d0").is_some(),
        "the nullary variant A (disc 0) gets a checked constructor"
    );
    // The payload variant B (disc 1) also gets one.
    assert!(
        db.def_by_name("__invariant_construct_T__d1").is_some(),
        "the payload variant B (disc 1) gets a checked constructor"
    );
}

/// Verification Inc-b predicate NAME-RESOLUTION (@requires LIST-REST arm): the predicate binder collector
/// (`resolve::arm_pattern_binders`) binds BOTH a rest pattern's leaf + rest names without over-collecting
/// the `list` head / `..` marker, and still catches a genuinely-stray name → CDZ0101. (The @invariant
/// flat/destructure name-resolution halves migrated to corpus 26-program-conditions.)
#[test]
fn a_requires_predicate_list_rest_pattern_arm_binds_both_names_and_still_catches_a_stray() {
    use crate::testkit::parse;
    // The @invariant flat/destructure name-resolution halves of this test migrated to corpus
    // 26-program-conditions (unbound-flat 2840, accepted-flat 2863, destructure-binder-in-scope 3022,
    // and the destructure-arm-stray-name reject added alongside 2840). This keeps the @requires
    // LIST-REST-pattern binder-scope pin below, whose rest-pattern-in-a-predicate shape has no corpus
    // form yet (it asserts a predicate-local binder scope + no-spurious-secondary, corpus-inexpressible).
    // PR#562: the predicate binder collector now delegates to `resolve::arm_pattern_binders` (the
    // canonical, well-scoped one), replacing a local walk that pushed EVERY bare name — including a
    // separator `..`/`_` or a compound pattern's HEAD. A REST pattern `(P.Ps (list a .. rest))` must
    // bind BOTH the leaf `a` AND the rest name `rest` while NOT over-collecting the `list` head or the
    // `..` marker into scope; the body may use `a`/`rest`, and a stray name is still caught. This pins
    // that the well-scoped collector binds a nested rest pattern's names correctly in a predicate.
    let rest_ok = "(module m \
            (@ (requires (match xs ((P.Ps (list a .. rest)) (>= a 0)) (P.Empty true))) \
              (def (f (: xs P)) 5)) \
            (type P (Ps (List Int64)) (Empty)) (export f))";
    assert!(
        !crate::diagnostics(&mut crate::db::Db::load(parse(rest_ok)))
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0101")),
        "a predicate whose arm is a list-rest pattern binds its leaf + rest names (no false unbound)"
    );
    // ...and a stray name in that same rest-pattern arm is STILL caught — the collector binds `a`/`rest`
    // but not a typo'd reference, so the `@requires` gate does not let a genuine unbound slip.
    let rest_bad = "(module m \
            (@ (requires (match xs ((P.Ps (list a .. rest)) (>= a stray)) (P.Empty true))) \
              (def (f (: xs P)) 5)) \
            (type P (Ps (List Int64)) (Empty)) (export f))";
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(rest_bad)))
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0101") && d.message.contains("stray")),
        "a stray name in a list-rest predicate arm is still CDZ0101 (binders don't mask it)"
    );
}

// (a_malformed_requires_ensures_arity_is_rejected_not_silently_dropped migrated to corpus
// 26-program-conditions, the "@requires/@ensures ARITY discipline" block: zero-arg and two-arg
// @requires/@ensures → CDZ0201 (message "takes exactly one PREDICATE argument"), plus the two valid
// one-predicate controls that run (c → 3). --case grades the code + message (all 6 PASS). Arity is a
// strip-time shape check; name-resolution/boolean-typedness is checked later at denotation, see
// requires_ensures_predicate_unbound_name_is_cdz0101_valid_names_ok below.)

/// Verification Inc-b a1: the compiler-bundled verification KERNEL asset (`verify_kernel.cdz`) READS as
/// a well-formed s-expression module and declares the pieces a1/a3 need — the ABSTRACT `Thm` sequent
/// and the `licenses` match predicate. This is the parse-level validation of the asset the compiler will
/// `include_str!` at a1 (design §9): a malformed/truncated kernel source is caught here.
///
/// ROOT SHAPE: the asset is a BARE `(do …)`, NOT a `(module "verify-kernel" (do …))` wrapper — the
/// link path supplies the module NAME externally (`VERIFY_KERNEL_NAME`), exactly as the corpus package
/// driver keys a library file by its FILENAME. A doubly-wrapped `(module … (do …))` root makes
/// `link::top_items` unwrap only the outer module to a single `[(do …)]` item, hiding every
/// `type`/`def`/`export` from top-level linking (imports resolve to nothing; the kernel's own type
/// annotations go unbound) — see the a3 root-cause probe. Bare `(do …)` is the fix.
///
/// NOTE: full semantic validation (the module's types in scope, `Thm` unforgeable) requires the LINKED-
/// package load a1 wires — the bare-`(do …)` kernel is linked as a package member under
/// `VERIFY_KERNEL_NAME`, which puts its types in scope for its defs and gives `Thm` its opacity. That
/// end-to-end check is part of a3 proper; the module shape's opacity is already pinned by the
/// 25-verification corpus (63 unforgeability cases over this same `Thm`-sequent shape).
#[test]
fn bundled_verify_kernel_asset_reads_and_declares_thm_and_licenses() {
    // The bundled kernel asset — the same source the compiler will include_str! at a1.
    const KERNEL_SRC: &str = include_str!("../../verify_kernel.cdz");
    // It READS as a well-formed s-expression (the reader the compiler uses at a1).
    let arenas = cadenza_syntax::sexpr::read(KERNEL_SRC)
        .expect("the bundled verify_kernel.cdz reads as well-formed s-expression");
    assert!(!arenas.structure.is_empty(), "non-empty kernel arena");
    // It declares the pieces a1 links + a3 compile-time-evals: the abstract Thm sequent + licenses.
    assert!(
        KERNEL_SRC.contains("(type Thm (Seq (List Term) Term))"),
        "the kernel declares the abstract Thm sequent"
    );
    assert!(
        KERNEL_SRC.contains("(def (licenses"),
        "the kernel declares the licenses match predicate (the trusted elision surface)"
    );
    // The root is a BARE `(do …)`, NOT a `(module "verify-kernel" …)` wrapper — the link path supplies
    // the module name externally, and a re-wrap would hide the declarations from `link::top_items`.
    // Check the CODE (comment lines dropped — the header comment explains the module wrapper it avoids).
    let code: String = KERNEL_SRC
        .lines()
        .filter(|l| !l.trim_start().starts_with(';'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.trim_start().starts_with("(do"),
        "the kernel root is a bare (do …) so link::top_items sees each top-level declaration"
    );
    assert!(
        !code.contains("(module"),
        "the kernel must NOT re-wrap in (module …) — the link path names it externally"
    );
}

/// A SHAPE-valid constructor-export `(export (. T A))` / `(export (. T *))` must ALSO be SEMANTICALLY
/// valid: `T` a declared sum, `A` one of its variants. The linker's `as_ctor_export` recorded the
/// (type, ctor) names WITHOUT checking they exist, so `(export (. T Nonesuch))` (a ctor `T` lacks),
/// `(export (. foo A))` (`foo` a value def), and `(export (. Undeclared A))` were SILENTLY ACCEPTED.
/// `collect_faults` now validates each: an unknown ctor of a real sum names it + a did-you-mean over
/// the variants (with a replace fix); a non-sum head names its category; the wildcard `*` skips the
/// per-ctor check.
#[test]
fn a_constructor_export_is_semantically_validated() {
    use crate::testkit::parse;
    // (a) an unknown ctor of a real sum → names the ctor + type; a near-miss carries a replace fix.
    let near = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (type T (Alpha) (Beta)) (export (. T Alph)) (def (main) 1) (export main))",
    )))
    .into_iter()
    .find(|d| {
        d.message
            .contains("is not a constructor of the sum type `T`")
    })
    .expect("a bad ctor-export ctor is rejected");
    assert_eq!(
        near.code.as_deref(),
        Some("CDZ0201"),
        "got: {}",
        near.message
    );
    assert!(
        near.message.contains("did you mean `Alpha`?"),
        "names the near ctor: {}",
        near.message
    );
    assert_eq!(
        near.fix.as_ref().map(|f| (f.kind, f.replacement.as_str())),
        Some((crate::abi::FixKind::Replace, "Alpha")),
        "carries a replace-with-the-variant fix: {:?}",
        near.fix
    );
    // (b) a non-sum head (a value def) → names the category.
    let val = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def foo 5) (export (. foo A)) (def (main) 1) (export main))",
    )));
    assert!(
        val.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("`foo` to be a sum type")
            && d.message.contains("a value definition")),
        "a ctor-export of a value def names the category: {:?}",
        val.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // (c) an undeclared type head → "not a declared type".
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (export (. Undeclared A)) (def (main) 1) (export main))"
        )))
        .iter()
        .any(|d| d.message.contains("not a declared type")),
        "a ctor-export of an undeclared type is rejected"
    );
    // NO FALSE POSITIVE: a real ctor and the wildcard are clean.
    for ok in [
        "(module m (type T (Alpha) (Beta)) (export (. T Alpha)) (def (main) 1) (export main))",
        "(module m (type T (Alpha) (Beta)) (export (. T *)) (def (main) 1) (export main))",
    ] {
        assert!(
            !crate::diagnostics(&mut crate::db::Db::load(parse(ok)))
                .iter()
                .any(|d| d.message.contains("is not a constructor")
                    || d.message.contains("to be a sum type")),
            "a valid ctor-export is not flagged: {ok}"
        );
    }
}

#[test]
fn a_mistyped_top_level_keyword_suggests_the_keyword_and_carries_a_replace_fix() {
    // A top-level `(head …)` form whose head is a near-miss for a DECLARATION KEYWORD (`exprot`→
    // `export`, `deff`→`def`) is a mistyped keyword — the likeliest intent in a declaration position.
    // It now names the intended keyword AND carries a REPLACE fix on the head occurrence, the same
    // closed-set "did you mean?"-with-fix the export-name / pragma-key sites give (the candidate pool
    // is `TOP_LEVEL_KEYWORDS`, so the suggestion can never name a keyword the grammar rejects). This is
    // a code-less DECLINE (a top-level unbound head declines the whole program), so it is found by
    // message, not code.
    let find = |src: &str| {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.message.contains("at the top level"))
            .expect("a top-level unknown head is reported")
    };
    let d = find("(module m (def (f) 1) (exprot f))");
    assert!(
        d.message.contains("did you mean `export`?"),
        "names the keyword, not a value def: {}",
        d.message
    );
    let fix = d.fix.as_ref().expect("carries a replace fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Replace);
    assert_eq!(fix.replacement, "export", "swaps the head to the keyword");
    // `deff` → `def` (the head is the FIRST form, not the export).
    let d2 = find("(module m (deff (f) 1) (export f))");
    assert!(d2.message.contains("did you mean `def`?"), "{}", d2.message);
    assert_eq!(d2.fix.as_ref().map(|f| f.replacement.as_str()), Some("def"));
    // NO OVERREACH: an unknown top-level head that is NOT close to any keyword (`improt`, `frobnicate`)
    // keeps the defined-name hint and carries NO keyword fix — a baseless keyword swap is worse than
    // none.
    let far = find("(module m (improt x) (def (f) 1) (export f))");
    assert!(
        !far.message.contains("did you mean `"),
        "no baseless keyword suggestion: {}",
        far.message
    );
    assert!(
        far.fix.is_none(),
        "no fix for a non-keyword head: {:?}",
        far.fix
    );
}

#[test]
fn a_bare_unbound_name_top_level_item_is_the_same_unbound_error_as_its_application_twin() {
    use crate::testkit::parse;
    // A bare NAME atom top-level item resolving to NOTHING — `(module m nonesuch …)` — is the
    // paren-less twin of the `(nonesuch)` APPLICATION and MUST behave identically. `head_name` is
    // `None` for an atom, so `unknown_top_forms` never saw it and it was SILENTLY ACCEPTED; a bare name
    // naming no binding is broken under any reading of the grammar. It now gets the SAME code-less
    // "unbound name at the top level" decline (found by message) the application form gives.
    let find = |src: &str| {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.message.contains("at the top level"))
    };
    // The bare-name twin and the application both report the SAME message.
    let bare = find("(module m nonesuch (def (main) 0) (export main))")
        .expect("a bare unbound name is now reported");
    let app = find("(module m (nonesuch) (def (main) 0) (export main))")
        .expect("the application twin is reported");
    assert!(
        bare.message
            .contains("unbound name `nonesuch` at the top level"),
        "the bare name is named unbound: {}",
        bare.message
    );
    assert_eq!(
        bare.message, app.message,
        "the bare name and its application twin report identically"
    );
    // A near-miss to a def name carries the confident did-you-mean hint.
    let near = find("(module m maim (def (main) 0) (export main))")
        .expect("a near-miss bare name is reported");
    assert!(
        near.message.contains("did you mean `main`?"),
        "names the near def: {}",
        near.message
    );
    // NO false positive: a LITERAL, a BOUND bare name, a grammar head, a prelude name, and a TYPE name
    // are all left to the (pending) bare-expression-legality ruling — not flagged as unbound.
    for ok in [
        "(module m 5 (def (main) 0) (export main))",    // literal
        "(module m main (def (main) 0) (export main))", // bound name
        "(module m if (def (main) 0) (export main))",   // grammar head
        "(module m unit (def (main) 0) (export main))", // prelude
        "(module m Int64 (def (main) 0) (export main))", // type name
    ] {
        assert!(
            find(ok).is_none(),
            "a resolvable / literal bare item is not flagged unbound: {ok}"
        );
    }
}

/// A CDZ0101 raised on a SYNTHESIZED β-reduction name copy (the whole-program-monomorphization path,
/// where an inlined callee body's name re-resolves — v-compiler-ml's mutual-recursion-cycle unbound
/// param) must carry a SOURCE LOCATION, not read as a bare, unanchored "unbound name `x`". The
/// per-file `cdz check` path reports such a name at its user occurrence (and suppresses the synth
/// copy as an inference artifact — `infer::collect_node`'s `is_user_node` gate); the whole-program
/// reached-poison walk instead surfaces the copy, whose id is past `user_node_count` (no span), so
/// `sanitize_origin` used to null the anchor. The fix records each β-copied name's SOURCE occurrence
/// (`synth_name_origin`, in `eval::copy_structural`) and RELOCATES a synth anchor to it
/// (`Db::source_of_synth`) rather than dropping it. This pins the mechanism directly: β-copy a real
/// def body via `copy_structural_pub`, then assert every copied NAME atom traces back to a USER node.
/// A regression (a copy site not recording provenance, or the relocation reverting to null) re-buries
/// this whole class of whole-program CDZ0101 as location-less — the exact "very hard to debug"
/// symptom the report filed.
///
/// Tested at the MECHANISM level (not end-to-end on a program) BY DESIGN: after v-inference's SCC fix
/// (trunk 8a044187a), a GENUINELY-unbound name anchors at its own user occurrence via the per-body
/// `type_errors` walk BEFORE the reached-poison synth path surfaces it — so no stable whole-program
/// input produces a synth-anchored CDZ0101 without reintroducing a bug. The relocation is thus a
/// DEFENSIVE guarantee (any future synth-anchored poison gets located, never dropped), and the honest
/// pin is here: the provenance record + `source_of_synth` chain, plus the null-fallback for a
/// sourceless synth node.
#[test]
fn a_beta_reduced_name_copy_traces_back_to_its_source_user_node_for_diagnostics() {
    use crate::db::Db;
    // A def whose body references a param (`n`) and a sibling (`g`) by name — the shapes a
    // monomorphization inline copies fresh. `copy_structural_pub` β-copies the body with no
    // substitution (an empty `arg_of`), producing fresh name occurrences past the user-node ceiling.
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (g (: x Int64)) x) (def (f (: n Int64)) (+ (g n) n)) (export f))",
    ));
    let d = db.def_by_name("f").expect("def f");
    let body = db.defs[d].body.expect("f has a body");
    let params = db.defs[d].params.clone();
    // The ceiling is the arena length BEFORE the copy — every node appended by the copy is synth.
    let ceiling = db.ast.structure.len() as u32;
    // The body node itself is a USER node — `source_of_synth` returns None (nothing to relocate).
    assert!(db.is_user_node(body), "the source body is a user node");
    assert_eq!(
        db.source_of_synth(body),
        None,
        "a user node has no synth provenance to relocate"
    );
    let arg_of = crate::fxhash::FxHashMap::default();
    let copy = crate::eval::copy_structural_pub(&mut db, body, &params, &arg_of);
    assert!(
        !db.is_user_node(copy),
        "the copy root is a synthesized node (past the user-node ceiling)"
    );
    // EVERY synthesized NAME atom the copy recorded provenance for must trace back to a USER node —
    // so a CDZ0101 raised on any of them (an unbound re-resolution) anchors at the author's source
    // reference, not nowhere.
    let mut checked_a_name = false;
    for i in ceiling..(db.ast.structure.len() as u32) {
        let id = crate::ast::StructId(i);
        let Some(src) = db.source_of_synth(id) else {
            continue;
        };
        assert!(
            db.is_user_node(src),
            "the relocated anchor is a genuine user node with a span"
        );
        // The traced source names the SAME identifier — the relocation points at the right token.
        assert_eq!(
            db.ast.as_name(id),
            db.ast.as_name(src),
            "the copy and its source occurrence are the same name"
        );
        checked_a_name = true;
    }
    assert!(
        checked_a_name,
        "the β-copy produced at least one provenance-tracked name atom to verify"
    );
    // NULL-FALLBACK branch (the other half of `sanitize_origin`'s decision): a synthesized node with
    // NO recorded provenance — a freshly PUSHED atom, like the constant-atom / built-`(Int W)` nodes
    // the evaluator appends — has no source to relocate to, so `source_of_synth` returns None and
    // `sanitize_origin` still nulls the unmappable anchor (the pre-fix behavior, preserved). This is
    // what keeps the fix strictly ADDITIVE: it relocates ONLY when a real source is on record, never
    // inventing a bogus anchor for a genuinely-sourceless synth node.
    let bare_synth = db.push_atom(crate::ast::Leaf::Int {
        value: crate::ast::IntValue::from_i64(0),
        radix: crate::ast::Radix::Dec,
    });
    assert!(
        !db.is_user_node(bare_synth),
        "the pushed atom is synthesized"
    );
    assert_eq!(
        db.source_of_synth(bare_synth),
        None,
        "a synth node with no recorded provenance has nothing to relocate to (null-fallback)"
    );
}

#[test]
fn an_import_form_is_named_as_unmodeled_not_a_typo_of_export() {
    // `import` is a KNOWN surface keyword (the ML reader parses `import { … } from "…"` → an
    // `(import …)` top-level form) that the MODULE LINKER resolves, not this single-module compile path
    // (a structural boundary — cross-module imports ARE realized via the linker; concierge seq-286
    // ruling). Because `import`→`export` is only 2 edits, the generic keyword-typo path would suggest
    // "did you mean `export`?" — an actively MISLEADING fix (an author who wrote `import` never meant its
    // opposite). It now gets a specific linker-boundary message with NO export swap.
    let find = |src: &str| {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.message.contains("import"))
            .expect("the import form is reported")
    };
    for src in [
        "(module m (import \"lib\" (foo)) (def (main) 1) (export main))",
        "(do (import \"lib\" (foo)) (def (main) 1) (export main))",
    ] {
        let d = find(src);
        assert!(
            d.message.contains("module linker") && d.message.contains("`import`"),
            "names import as a linker-boundary form: {}",
            d.message
        );
        assert!(
            !d.message.contains("did you mean `export`?"),
            "no misleading export suggestion: {}",
            d.message
        );
        assert!(d.fix.is_none(), "no export swap fix: {:?}", d.fix);
    }
}

#[test]
fn an_unannotated_context_typed_closure_param_carries_its_narrow_width_to_the_const_fold() {
    // WRONG-VALUE regression: an UNANNOTATED closure param typed narrow from its storage context's
    // arrow (`app : ((-> Int8 Int8)) -> Int8` applied `(app (fn (n) …))`) recovered the arrow's param
    // type for the runtime path, but the body's CONST-FOLD ran on the still-`Any` param → a const arg
    // folded at Int64, MISSING the Int8 overflow. `(app (fn (n) (+ n 1)))` @ (g 127) yielded 128 (Int64)
    // instead of the CDZ0304 the explicit `(fn ((: n Int8)) …)` gives. Fixed by recovering the param's
    // context arrow at `type_of` (so the body types narrow) AND wrapping the substituted arg in the
    // recovered `(: arg (Int N))` at β-reduction (so the fit-check fires + travels through copies).
    // A constant arithmetic OPERATION that overflows the recovered narrow width is a provable trap →
    // CDZ0304 (ConstTrap), like the wide `(+ Int64.max 1)`. (The context-typing fix — carrying the
    // arrow's Int8 into the body const-fold — is what makes the overflow VISIBLE at compile time; the
    // CODE is CDZ0304 because it is an operation with no value, not a literal that fails to fit.)
    let overflows = "(module m (def (app (: g (-> Int8 Int8))) (g 127)) (def (main) (app (fn (n) (+ n 1)))) (export main))";
    assert_eq!(
        reject_code(overflows).as_deref(),
        Some("CDZ0304"),
        "an unannotated context-Int8 param overflows a const arg like an explicit Int8 param"
    );
    // The `*` variant: 12*12 = 144 > Int8.max 127.
    let mul = "(module m (def (app (: g (-> Int8 Int8))) (g 12)) (def (main) (app (fn (n) (* n n)))) (export main))";
    assert_eq!(reject_code(mul).as_deref(), Some("CDZ0304"));
    // UInt8: 255 + 1 = 256 overflows.
    let uint = "(module m (def (app (: g (-> UInt8 UInt8))) (g 255)) (def (main) (app (fn (n) (+ n 1)))) (export main))";
    assert_eq!(reject_code(uint).as_deref(), Some("CDZ0304"));
    // NO OVER-REJECTION: an IN-RANGE const still compiles + runs (g 5 → 5+1 = 6, fits Int8).
    let in_range = "(module m (def (app (: g (-> Int8 Int8))) (g 5)) (def (main) (app (fn (n) (+ n 1)))) (export main))";
    assert_eq!(
        reject_code(in_range),
        None,
        "an in-range const through a context-Int8 param must still compile"
    );
    // A WIDE (Int64) context param must NOT be false-rejected — 128 fits Int64.
    let wide = "(module m (def (app (: g (-> Int64 Int64))) (g 127)) (def (main) (app (fn (n) (+ n 1)))) (export main))";
    assert_eq!(
        reject_code(wide),
        None,
        "a wide-Int64 context param has no narrow width to overflow"
    );
}

// (a_quantity_whose_reference_scaled_magnitude_overflows_its_inner_int_declines migrated to corpus
// 18-units-of-measure: the reject `(Qty.of 9223372036854776 kilometer)` → CDZ0304 (×1000 scaled magnitude
// overflows Int64) is the "a quantity whose reference-scaled magnitude overflows its inner Int is rejected"
// case; the two NO-OVER-REJECTION boundary controls are the added "a prefixed-unit magnitude whose
// reference-scaled value JUST fits Int64 is not rejected" (9223372036854775 km → 9223372036854775000 m,
// one step below the ceiling) and "a reference-unit magnitude at Int64 max is not rejected (no scale to
// overflow)" (a scale-1 reference unit has nothing to overflow). The fitting-scaled-value render path is
// also covered by the Float display case "a prefixed quantity displays scaled to its reference unit".)

#[test]
fn an_explosive_self_application_const_fold_declines_instead_of_overflowing_the_stack() {
    // v-cdz-smith seed 14281198340853570680 (`selfapp-typeinfer-overflow` escape): an EXPLOSIVE
    // self-application drove the const-fold recursion `const_eval` <-> `const_eval_apply` past the
    // native `rcdzc-compile` worker stack and HARD-ABORTED the process (SIGABRT, bypassing
    // `catch_unwind`). `const_eval`'s `budget` bounds cumulative WORK but not native call DEPTH; the
    // added `db.descent_depth` guard (shared with `core_of`/`type_of`, the stack-sizing policy) now
    // DECLINES the fold at the depth limit, so the ill-formed program is REJECTED cleanly. This test
    // running to completion (no process abort) IS the core assertion — a companion to the deep-recursion
    // robustness tests `host.rs` sizes the worker stack for.
    let src = "(do (def (main) (let ((v0 (fn (v1) (v1 (. (v1 v1) 2))))) (. (v0 (fn (v2) (v2 v0))) f0))) (export main))";
    let out = crate::host::run_with_compiler_stack(|| {
        compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            )],
            &[Target::Wasm],
        )
    });
    assert!(
        out.has_error(),
        "the ill-formed self-application must DECLINE (not compile, not crash): {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    assert!(
        out.artifact(Target::Wasm.artifact_kind()).is_none(),
        "no wasm artifact for a declined self-application"
    );
}

#[test]
fn const_eval_matches_native_compound_patterns_so_the_const_trap_surfaces() {
    // SOUNDNESS guard (28-compiler-primitives dc02, a MISCOMPILE the M2 native-compound migration exposed):
    // `const_pattern_matches` (the const-evaluator's arm selector) recognized only the NAME-alias
    // `(tuple …)`/`(list …)` pattern heads (`as_form`), NOT the native `#tuple(…)`/`#list(…)` ctor-leaf head.
    // So a `(const (: t (Tuple …)))` recursion whose match arm uses a NATIVE tuple pattern silently DECLINED
    // the whole const-eval (an "undecidable shape") → the const-fold countdown never ran → its `(trap …)`
    // was LOST (compiled clean / `None`, not CDZ0304). `corpus_roundtrip` (structural) could not catch this —
    // only the behavioral grade. Now reads `compound_form_of` (native + alias), so the const trap surfaces
    // in BOTH spellings.
    for (label, src) in [
        (
            "native #tuple const-param countdown",
            "(module m (def (f (const (: t (Tuple Int64 Int64)))) (match t (#tuple(a b) (if (= a b) (trap \"met\") (f #tuple((- a 1) b)))))) (def (main) (f #tuple(3 1))) (export main))",
        ),
        (
            "name-alias tuple const-param countdown",
            "(module m (def (f (const (: t (Tuple Int64 Int64)))) (match t ((tuple a b) (if (= a b) (trap \"met\") (f (tuple (- a 1) b)))))) (def (main) (f (tuple 3 1))) (export main))",
        ),
        (
            "native #list const-param countdown",
            "(module m (def (f (const (: t (List Int64)))) (match t (#list() (trap \"met\")) (#list(h .. r) (f r)))) (def (main) (f #list(1 2))) (export main))",
        ),
    ] {
        assert_eq!(
            reject_code(src).as_deref(),
            Some("CDZ0304"),
            "{label}: the const-param countdown's trap must surface CDZ0304 (const-eval must match the native compound pattern), not be silently lost"
        );
    }
}

#[test]
fn native_compound_recognition_matches_the_alias_across_lingering_behavior_paths() {
    // dc02-class hardening: three rcdzc behavior-path recognizers still read the NAME/STRING alias only
    // (eval.rs record-field-key β-immunity via compound_ctor_either; lower.rs scalar-replacement +
    // ctor-payload-irrefutability via as_form(_,"tuple")). After M3 nativized the corpus, a NATIVE form
    // hitting these paths was mis-handled (a native #record field key matching a param could be
    // β-substituted → corrupted; a native #tuple payload mis-classified refutable). All three now read
    // compound_form_of (native + name + string). Pin native ≡ alias (both compile clean):
    for (label, src) in [
        (
            "native #record field key is β-immune (not substituted → corrupted)",
            "(module m (def (f (: x Int64)) #record((= x 5))) (def (main) (f 7)) (export main))",
        ),
        (
            "name-alias record field key β-immune (control)",
            "(module m (def (f (: x Int64)) (record (= x 5))) (def (main) (f 7)) (export main))",
        ),
        (
            "native #tuple ctor-payload is an irrefutable match binder",
            "(module m (def (g (: p (Option (Tuple Int64 Int64)))) (match p ((Some #tuple(a b)) (+ a b)) (_ 0))) (def (main) (g (Some #tuple(3 4)))) (export main))",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "{label}: must compile clean (native compound recognition must match the alias), got {:?}",
            reject_code(src)
        );
    }
}

#[test]
fn native_compound_match_patterns_compile_over_an_untyped_scrutinee_like_the_alias() {
    // M3 blocker (v-guide-infra): a native #-form compound MATCH pattern over a scrutinee whose type is
    // NOT definitely a compound (an UNTYPED/Any param — the guide's shape) rejected CDZ0201 "a compound-
    // constructor head leaf is not a value", while the name-alias compiled. lower_match's scalar-path
    // decline (when classify_probe returns None) lowered the pattern HEAD to propagate an unbound-ctor
    // poison; a native ctor-LEAF head spuriously poisoned CDZ0201 there. Now a recognized compound
    // pattern (compound_form_of, any spelling) is exempt from that head-poison probe → it declines
    // cleanly like the alias, and the type-directed lowering (once inference solves the scrutinee) handles
    // it. Native ≡ alias, TYPED and UNTYPED:
    for (label, src) in [
        (
            "native #tuple match, UNTYPED param",
            "(module m (def (f p) (match p (#tuple(a b) a))) (def (main) (f #tuple(3 4))) (export main))",
        ),
        (
            "native #record match, UNTYPED param",
            "(module m (def (f p) (match p (#record((= x xv) (= y yv)) xv))) (def (main) (f #record((= x 1) (= y 2)))) (export main))",
        ),
        (
            "name-alias tuple match, UNTYPED param (control)",
            "(module m (def (f p) (match p ((tuple a b) a))) (def (main) (f (tuple 3 4))) (export main))",
        ),
        (
            "native #tuple match, TYPED param",
            "(module m (def (f (: p (Tuple Int64 Int64))) (match p (#tuple(a b) a))) (def (main) (f #tuple(3 4))) (export main))",
        ),
        (
            "native #list match, TYPED param (rest + empty arms)",
            "(module m (def (f (: xs (List Int64))) (match xs (#list() 0) (#list(h .. t) h))) (def (main) (f #list(1 2))) (export main))",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "{label}: a native compound match pattern must compile like the alias, got {:?}",
            reject_code(src)
        );
    }
}

#[test]
fn a_native_list_pattern_shapes_its_scrutinee_type_like_the_alias() {
    // M3 native-recognition parity (infer.rs pattern_implied_ty): a native `#list` MATCH pattern must
    // SHAPE an otherwise-untyped scrutinee `List <elem>` exactly like the `(list …)` alias — so a recursive
    // list consumer whose param type is implied ONLY by the list pattern (no call-site type/annotation)
    // solves its scheme. `pattern_implied_ty`'s list arm read `as_form` (name-alias only), so a native
    // `#list` recursive consumer left its param a free var → grounded Any → the scheme DECLINED (CDZ0201)
    // while the alias compiled. Now reads `compound_form_of`. Native ≡ alias:
    for (label, src) in [
        (
            "native #list recursive consumer (type only from pattern)",
            "(module m (def (sum (: acc Int64) xs) (match xs (#list() acc) (#list(h .. t) (sum (+ acc h) t)))) (export sum))",
        ),
        (
            "name-alias list recursive consumer (control)",
            "(module m (def (sum (: acc Int64) xs) (match xs ((list) acc) ((list h .. t) (sum (+ acc h) t)))) (export sum))",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "{label}: a native list-pattern must shape its scrutinee type like the alias, got {:?}",
            reject_code(src)
        );
    }
}

#[test]
fn a_native_compound_match_arm_compiles_under_polymorphic_instantiation_like_the_alias() {
    // M3 native-recognition parity (v-guide-infra #51311): a native `#list`/`#tuple`/`#record` MATCH-ARM
    // pattern in a GENERIC def, instantiated at >= 2 distinct element types (so the def monomorphizes more
    // than once), must compile exactly like the `(list …)`/`(tuple …)`/`(record …)` alias. The historical
    // bug: the recognizer resolved the native ctor-LEAF head per-instantiation and re-read it as a VALUE on
    // the 2nd monomorphization → CDZ0201 "a compound-constructor head leaf is not a value", while the alias
    // (whose head resolves cleanly) compiled. Fixed upstream by exempting a recognized compound pattern from
    // the head-poison probe (compound_form_of, #5429) + native `#list` scrutinee type-shaping (#5436); this
    // pins the polymorphic facet so it can't regress. Native ≡ alias, both compile:
    for (label, src) in [
        (
            "native #list poly len (Int + String elems)",
            "(module m (def (len xs) (match xs (#list() 0) (#list(h .. t) (+ 1 (len t))))) (def (main) (+ (len #list(1 2 3)) (len #list(\"a\" \"b\")))) (export main))",
        ),
        (
            "name-alias (list …) poly len (control)",
            "(module m (def (len xs) (match xs ((list) 0) ((list h .. t) (+ 1 (len t))))) (def (main) (+ (len (list 1 2 3)) (len (list \"a\" \"b\")))) (export main))",
        ),
        (
            "native #tuple poly fst (Int,Int + Bool,Bool)",
            "(module m (def (fst p) (match p (#tuple(a b) a))) (def (main) (if (fst #tuple(true false)) (fst #tuple(1 2)) 0)) (export main))",
        ),
        (
            "name-alias (tuple …) poly fst (control)",
            "(module m (def (fst p) (match p ((tuple a b) a))) (def (main) (if (fst (tuple true false)) (fst (tuple 1 2)) 0)) (export main))",
        ),
        (
            "native #record poly getk (Int + Bool)",
            "(module m (def (getk r) (match r (#record((= k kv) (= v _vv)) kv))) (def (main) (if (getk #record((= k true) (= v false))) (getk #record((= k 1) (= v 2))) 0)) (export main))",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "{label}: a native compound match-arm must compile under polymorphic instantiation like the alias, got {:?}",
            reject_code(src)
        );
    }
}

#[test]
fn a_nested_map_list_element_with_an_absent_const_key_over_a_runtime_value_map_compiles() {
    // breaker mf3 (the const-path sibling of #5450's runtime top-level fall-through): a nested `#map`
    // list-arm element `(list (map (5 v)) _r)` whose head map has a CONSTANT KEY but a RUNTIME VALUE
    // (`(map (1 n))`, `n` a param). The refutable-map-element desugar builds a key-PRESENCE guard; because
    // the map value is runtime the guard is a runtime `Map.lookup` test, so the guarded arm's body is kept
    // in the `MatchList` and its value-binder Core is lowered EAGERLY. That binder read folded through the
    // const-structured list to the const-keyed `MapNew`, found the pattern key PROVABLY absent, and emitted
    // a HARD Poison ("a map pattern value binder's key is absent from the constant map (arm mis-selected)")
    // — a compile failure on a valid program (plus a Poison→cadenza-backend cascade). The arm is DEAD (the
    // runtime presence guard gates it false), so the read now lowers to a divergent `Core::Trap` instead of
    // a Poison. Both the native `#map` and name-alias `(map …)` spellings, both single- and two-map-arm
    // shapes, must COMPILE (behavioral fall-through/binding pinned by the corpus grade):
    for (label, src) in [
        (
            "native #map, one map arm, absent key, runtime value",
            "(module m (def (f xs) (match xs (#list(#map((= 5 v)) _r) v) (_ (- 0 1)))) (def (main (: n Int64)) (f #list(#map((= 1 n)) #map((= 2 20))))) (export main))",
        ),
        (
            "name-alias (list (map …)), one map arm, absent key, runtime value",
            "(module m (def (f xs) (match xs ((list #map((= 5 v)) _r) v) (_ (- 0 1)))) (def (main (: n Int64)) (f (list #map((= 1 n)) #map((= 2 20))))) (export main))",
        ),
        (
            "two map arms, first absent + second present key, runtime value",
            "(module m (def (f xs) (match xs (#list(#map((= 5 v)) _r) v) (#list(#map((= 1 w)) _r) (* w 10)) (_ (- 0 1)))) (def (main (: n Int64)) (f #list(#map((= 1 n)) #map((= 2 20))))) (export main))",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "{label}: a nested map list-element with an absent const key over a runtime-value map must compile, got {:?}",
            reject_code(src)
        );
    }
}

#[test]
fn a_native_compound_equality_compiles_like_the_alias() {
    // M3 parity: an equality `(= a b)` over NATIVE compound literals compiles exactly like the
    // name-alias, across every kind — so `Eq`/`const_compound_eq`'s structural walk (order-independent
    // for record/map/set, positional for tuple, ordered for list, recursive for nesting) reads the
    // native ctor-leaf heads + FieldPair entries. (Behavioral truth — order-independence, ordered lists,
    // nesting — is pinned by the corpus grade; this guards that the recognizers stay native-aware through
    // Phase-2's reader/recognizer changes.) Native ≡ alias where an alias exists (sets have only the
    // `#set`/`("set" …)` spelling, no bare-name alias, so the native form is checked alone):
    for (label, src) in [
        (
            "native #record equality",
            "(module m (def (main) (= #record((= x 1) (= y 2)) #record((= y 2) (= x 1)))) (export main))",
        ),
        (
            "name-alias record equality (control)",
            "(module m (def (main) (= (record (= x 1) (= y 2)) (record (= y 2) (= x 1)))) (export main))",
        ),
        (
            "native #map equality",
            "(module m (def (main) (= #map((= 1 10) (= 2 20)) #map((= 2 20) (= 1 10)))) (export main))",
        ),
        (
            "native #set equality",
            "(module m (def (main) (= #set(1 2 3) #set(3 2 1))) (export main))",
        ),
        (
            "native #list equality",
            "(module m (def (main) (= #list(1 2) #list(2 1))) (export main))",
        ),
        (
            "native #tuple equality",
            "(module m (def (main) (= #tuple(1 2) #tuple(1 2))) (export main))",
        ),
        (
            "native nested #record-in-#list equality",
            "(module m (def (main) (= #list(#record((= a 1))) #list(#record((= a 1))))) (export main))",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "{label}: native compound equality must compile like the alias, got {:?}",
            reject_code(src)
        );
    }
}

#[test]
fn a_refutable_list_element_dead_arm_with_a_runtime_leaf_compiles_across_the_family() {
    // Hazard-class boundary for #5450/#5472: a REFUTABLE list-arm element desugars to a guard + a body
    // re-match, and over a const-STRUCTURED scrutinee carrying a RUNTIME leaf a mismatching (dead) arm's
    // body binder is lowered EAGERLY. The MAP element was vulnerable — `desugar_runtime_map_match` routes
    // on WHOLE-map constness, so a runtime VALUE made the presence guard runtime → the dead arm was kept
    // and its value-binder folded through to the const-keyed `MapNew` and hard-Poisoned on the absent key
    // (#5472 lowers it to `Core::Trap` instead). The CTOR-element and NESTED-LIST-element siblings are NOT
    // vulnerable: their guards fold on the DISCRIMINANT / LENGTH, which is const here independently of the
    // runtime payload/elements, so the dead arm is PRUNED (never eagerly lowered). Pin the whole family +
    // the map REST-binder companion so a future desugar change can't reintroduce the class:
    for (label, src) in [
        (
            "ctor element, wrong-variant dead arm, runtime payload",
            "(module m (def (f xs) (match xs (#list((None) .. r) 0) (#list((Some x) .. r) x) (_ (- 0 1)))) (def (main (: n Int64)) (f #list((Some n) (Some 2)))) (export main))",
        ),
        (
            "nested-list element, wrong-length dead arm, runtime element",
            "(module m (def (f xs) (match xs (#list(#list(a b) .. r) a) (#list(#list(a) .. r) a) (_ (- 0 1)))) (def (main (: n Int64)) (f #list(#list(n) #list(9)))) (export main))",
        ),
        (
            "map REST binder, absent-key dead arm, runtime value",
            "(module m (def (f xs) (match xs (#list(#map((= 5 v) .. rest) _r) v) (_ (- 0 1)))) (def (main (: n Int64)) (f #list(#map((= 1 n)) #map((= 2 20))))) (export main))",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "{label}: a dead refutable-element arm with a runtime leaf must compile (fall through), got {:?}",
            reject_code(src)
        );
    }
}

#[test]
fn a_native_structural_pattern_over_a_wrong_kind_scrutinee_is_cdz0203() {
    // ALIAS-SPELLING RESIDUE. The NATIVE faces of this soundness guard migrated to corpus
    // 05-compound-types: "a tuple pattern over an Int64 scrutinee" (`#tuple` over Int64), "a list pattern
    // over an Int64 scrutinee naming both kinds" (`#list` over Int64), and "a tuple pattern over a Map
    // scrutinee" (`#tuple` over Map) — each a CDZ0203 wrong-kind reject. The scrutinee-KIND check
    // (lower_match) reads `compound_form_of`, which recognizes BOTH the native `#tuple`/`#list`/`#map` head
    // AND the NAME-ALIAS `(tuple …)`/`(list …)`/`(map …)` head — after M3 nativized corpus patterns, a
    // native head that read only `head_name` slipped the check into a misleading CDZ0201, so both spellings
    // must now surface the SAME CDZ0203. The corpus surface is native-only (nativize-check forbids a
    // `(tuple …)` alias INPUT), so the alias-recognition arm below is the irreducible white-box residue the
    // corpus cannot express; the native twins are graded there.
    assert_eq!(
        reject_code("(module m (def (main) (match 5 ((tuple a b) a) (_ 0))) (export main))")
            .as_deref(),
        Some("CDZ0203"),
        "a name-alias `(tuple …)` pattern over a wrong-kind (Int64) scrutinee rejects CDZ0203, like its native `#tuple` twin"
    );
}

#[test]
fn a_native_record_destructuring_binding_param_binds_like_the_classic_spelling() {
    // M3-canonical equivalence (breaker flag, the def-param twin of the #5340/#5346 match-pattern
    // hardening): a NATIVE `#record(…)` destructuring PARAM must bind its irrefutable fields exactly as the
    // classic `(record …)` spelling — `check_binding_pattern`'s record arm read `as_form` (name-only), so a
    // native `#record` param fell through to the ctor classifier's CDZ0201 "not a tuple, record, or
    // constructor" while classic `(record …)` + native `#tuple` params compiled. Now reads `compound_form_of`.
    for (label, src) in [
        (
            "native #record param",
            "(module m (def (get #record((= x a))) a) (def (main) (get #record((= x 9)))) (export main))",
        ),
        (
            "classic (record …) param control",
            "(module m (def (get (record (= x a))) a) (def (main) (get (record (= x 9)))) (export main))",
        ),
        (
            "native #tuple param control",
            "(module m (def (get #tuple(a b)) (+ a b)) (def (main) (get #tuple(3 4))) (export main))",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "{label}: an irrefutable record/tuple destructuring param must compile (got {:?})",
            reject_code(src)
        );
    }
}

#[test]
fn a_narrow_width_overflow_in_a_native_map_or_set_literal_element_is_rejected_cdz0302() {
    // SOUNDNESS guard (06-numeric-model): a literal MAP VALUE / MAP KEY / SET element that overflows its
    // annotated narrow width must be rejected CDZ0302, not silently truncated. The width-fit annotation
    // DESCENT (`nested_literal_width_faults_against`) stopped reaching the M2 native ctor leaves: a
    // name-alias `(map (= k v))` resolves to `Apply(MapNew)` whose entry is a native `FieldPair` `(= k v)`
    // (3-element, not the legacy 2-element `(k v)` the descent read), and a `#set(e…)` literal resolves to
    // `Resolved::Set` (which had no descent arm at all) — so `(: (map (= 1 999)) (Map Int64 Int8))`,
    // `(: (map (= 999 1)) (Map Int8 Int64))`, and `(: #set(200) (Set Int8))` COMPILED and truncated
    // (999→-25, 200→-56). The descent now reads FieldPair map entries + descends the native Set literal.
    for (label, src) in [
        (
            "map value",
            "(module m (def (main) (: (map (= 1 999)) (Map Int64 Int8))) (export main))",
        ),
        (
            "map key",
            "(module m (def (main) (: (map (= 999 1)) (Map Int8 Int64))) (export main))",
        ),
        (
            "native #map value",
            "(module m (def (main) (: #map((= 1 999)) (Map Int64 Int8))) (export main))",
        ),
        (
            "set literal element",
            "(module m (def (main) (: #set(200) (Set Int8))) (export main))",
        ),
    ] {
        assert_eq!(
            reject_code(src).as_deref(),
            Some("CDZ0302"),
            "{label}: an out-of-range native map/set literal element must reject CDZ0302, not truncate"
        );
    }
    // A FITTING native map/set literal still compiles (no false reject).
    assert!(
        reject_code("(module m (def (main) (: (map (= 1 5)) (Map Int64 Int8))) (export main))")
            .is_none(),
        "a fitting map value is accepted"
    );
    assert!(
        reject_code("(module m (def (main) (: #set(5) (Set Int8))) (export main))").is_none(),
        "a fitting set element is accepted"
    );
}

#[test]
fn native_compound_patterns_tuple_list_map_resolve_and_bind() {
    // M3 pattern-spelling guard: `#name(…)` is the sole compound CTOR + PATTERN form (operator ruling).
    // A native ctor-leaf-headed match PATTERN (`#tuple(a b)`, `#list(h .. t)`, `#map((= k v))`) must
    // destructure + bind exactly like the name-head `(tuple a b)` / `(list …)` / `(map (= k v))` alias.
    // Before this, the pattern routers (is_tuple/list/map_pattern in resolve + lower), the map-match
    // dispatch, and `map_pattern_of`'s entry extraction recognized only the name/string head (and 2-element
    // `(k v)` map entries), so a native head/FieldPair-entry leaked to value resolution → CDZ0201
    // "a compound-constructor head leaf is not a value on its own". Now routed through `compound_form_of`
    // + `field_pair_parts` — all three destructure. (Native tuple + map were the broken ones; list already
    // worked via the compound_form_of binder-descent.)
    for (label, src) in [
        (
            "native tuple pattern",
            "(module m (def (f (: p (Tuple Int64 Int64))) (match p (#tuple(a b) (+ a b)))) (def (main) (f #tuple(3 4))) (export main))",
        ),
        (
            "native list pattern",
            "(module m (def (sum (: xs (List Int64))) (match xs (#list() 0) (#list(h .. t) (+ h (sum t))))) (def (main) (sum #list(1 2 3))) (export main))",
        ),
        (
            "native map pattern",
            "(module m (def (f (: mp (Map Int64 Int64))) (match mp (#map((= 1 v)) v) (_ 0))) (def (main) (f #map((= 1 7)))) (export main))",
        ),
    ] {
        assert!(
            reject_code(src).is_none(),
            "{label}: a native compound pattern must destructure like its name-head alias (no CDZ0201)"
        );
    }
}

#[test]
fn a_namespaced_ctor_pattern_binds_its_payload_and_a_record_eq_pattern_resolves() {
    // M3 regression guard (consolidation #5158 dropped rcdzc's `as_name` MIGRATION BRIDGE when it deleted
    // rcdzc/ast.rs + unified onto cadenza-ast, whose `as_name` did not bridge). Without the bridge, the
    // native `Member`/`FieldPair` leaf heads report `None` from `as_name`, so every `as_form(head, ".")` /
    // `as_name(head) == Some("=")` recognizer broke on native leaves:
    //   - a NAMESPACED ctor pattern `((. Option Some) v)` no longer bound its payload `v` (Case-6
    //     `find_binder_in_pattern` needs `as_form(head, ".")` to see the member-headed constructor) →
    //     CDZ0101 unbound `v`, while the BARE `(Some v)` still worked.
    //   - a record `(= k v)` PATTERN's field key/binder recognition broke → CDZ0201.
    // The bridge is restored in cadenza-ast's `as_name`; both forms resolve again.
    // Namespaced ctor pattern binds its payload (`v` → 7):
    assert!(
            reject_code(
                "(module m (def (f (: x (Option Int64))) (match x ((Option.Some v) v) ((Option.None) 0))) \
                 (def (main) (f (Option.Some 7))) (export main))"
            )
            .is_none(),
            "a namespaced ctor pattern (Option.Some v) binds its payload v — no CDZ0101 unbound"
        );
    // Bare ctor pattern still works (control):
    assert!(
        reject_code(
            "(module m (def (f (: x (Option Int64))) (match x ((Some v) v) ((None) 0))) \
                 (def (main) (f (Some 7))) (export main))"
        )
        .is_none(),
        "a bare ctor pattern (Some v) still binds its payload"
    );
    // Record `(= k v)` pattern resolves its field binders:
    assert!(
        reject_code(
            "(module m (def (g (: r (Record (x Int64) (y Int64)))) \
                   (match r ((record (= x a) (= y b)) (+ a b)))) (export g))"
        )
        .is_none(),
        "a record (= k v) pattern resolves its field binders — no CDZ0201"
    );
}

#[test]
fn adding_two_rationals_type_checks_without_a_phantom_int64() {
    // `(+ r s)` with BOTH operands `Rational` — Rational arithmetic IS wired (B4-1, `@431d7833`:
    // `apply_type` gives a Rational-operand `+`/`-`/`*`/`/` the result `Ty::Rational`). So it type-checks
    // cleanly, WITHOUT the operator's `∀a. (Int a) → …` scheme defaulting the first operand to `Int64`
    // and reporting the second as a numeric MIX (a phantom `Int64` the author never wrote). Before the
    // wiring this DECLINED honestly (`adding_two_rationals_declines_honestly_…`); now it is well-typed.
    // (A runtime-Rational operand still DECLINES at LOWERING — the constant fold is wired, the runtime
    // rational compound is a later slice — but the TYPE side is clean, which is what this pins.)
    assert!(
            reject_code("(module m (def (f (: r Rational) (: s Rational)) (+ r s)) (def (main) 5) (export main))")
                .is_none(),
            "Rational + Rational type-checks (no phantom Int64 mix)"
        );

    // A CONSTANT Rational sum FOLDS end-to-end (the wired path): `(+ (Rational.of 1 3) (Rational.of 1
    // 6))` = 1/2, a real value, not a decline.
    assert!(
        reject_code(
            "(module m (def (main) (+ (Rational.of 1 3) (Rational.of 1 6))) (export main))"
        )
        .is_none(),
        "constant Rational arithmetic folds"
    );

    // CONTRAST — equality over two Rationals COMPILES (∀a. a→a→Bool, no Int forcing).
    assert!(
            reject_code("(module m (def (f (: r Rational) (: s Rational)) (= r s)) (def (main) 5) (export main))")
                .is_none(),
            "equality of two Rationals still compiles"
        );
    // A GENUINE Rational/int MIX keeps CDZ0301 — there the Int64 operand IS present (the `1` literal).
    assert_eq!(
        reject_code("(module m (def (f (: r Rational)) (+ r 1)) (def (main) 5) (export main))")
            .as_deref(),
        Some("CDZ0301"),
        "a Rational/int mix is a genuine numeric-promotion error"
    );
    // BigInt arithmetic IS wired — two BigInt operands still compile (the sibling generic-numeric shape).
    assert!(
        reject_code(
            "(module m (def (f (: b BigInt) (: c BigInt)) (+ b c)) (def (main) 5) (export main))"
        )
        .is_none(),
        "BigInt arithmetic still compiles"
    );
}

#[test]
fn a_long_chained_rational_sum_folds_in_bounded_time() {
    // REGRESSION (perf): constant `Rational` arithmetic folds via `normalized_rational`, which reduces
    // the result to lowest terms with `IntValue::gcd`. `gcd` WAS Euclidean over the bit-serial
    // `divmod_mag` (`8·len(a)` iterations per call regardless of quotient size), so a chained exact sum
    // of fractions with DISTINCT denominators — which MULTIPLY without cancellation, so the magnitude
    // grows unbounded — was super-cubic: a 160-term sum took ~1.8s, a 320-term ~30s (99% in
    // `divmod_mag`), an effective HANG of `cdz check` on a small program. The fix makes `gcd_mag`
    // BINARY GCD (Stein's — shift/subtract only, never trial division), dropping it to O(bits²): the
    // 320-term sum now folds in ~200ms, the realistic 10-50-term case in single-digit ms. This 200-term
    // chain would be seconds pre-fix; that `diagnostics` returns quickly is the gate.
    let mut primes: Vec<u64> = Vec::new();
    let mut x = 2u64;
    while primes.len() < 200 {
        if primes.iter().all(|&p| !x.is_multiple_of(p)) {
            primes.push(x);
        }
        x += 1;
    }
    let mut expr = format!("(Rational.of 1 {})", primes[0]);
    for p in &primes[1..] {
        expr = format!("(+ {expr} (Rational.of 1 {p}))");
    }
    // Compare to a constant so the whole thing folds to a scalar `Bool` (a bare Rational has no host
    // render); the equality also folds, exercising the `cmp` after the reducing adds.
    let src = format!("(module m (def (main) (= {expr} (Rational.of 0 1))) (export main))");
    // Through the host-stack guard the bin uses (`host.rs`): the fold/reached-poison walk over a
    // 200-term chain recurses ~per term, which SIGABRTs a default `cargo test` worker's ≈2 MB stack
    // (EXIT=101, 0 FAILED) even though it TERMINATES — deep-but-finite frame bloat, not a loop
    // (`RUST_MIN_STACK=64M` passes). Sizing the stack from `DESCENT_DEPTH_LIMIT` bounds it by depth.
    let diags = crate::host::run_with_compiler_stack(move || {
        crate::diagnostics(&mut crate::db::Db::load(parse(&src)))
    });
    assert!(
        diags
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "a long chained Rational sum folds with no error diagnostics: {diags:?}"
    );
}

#[test]
fn a_bool_match_missing_a_literal_offers_the_specific_missing_arm() {
    // A Bool scrutinee is a FINITE gap: missing `false` → name it AND insert exactly
    // `(false (trap "TODO: false"))` (not a generic wildcard), the same precision as a missing sum
    // variant. The `trap` body type-checks against the sibling `Int64` arm (a `unit` body would clash).
    let d = reject_full("(module m (def (main (: b Bool)) (match b (true 1))) (export main))")
        .expect("non-exhaustive must reject");
    assert_eq!(d.code.as_deref(), Some("CDZ0210"), "got: {}", d.message);
    assert!(
        d.message.contains("`false`") && d.message.contains("not covered"),
        "names the missing literal: {}",
        d.message
    );
    assert_eq!(
        d.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("(false (trap \"TODO: false\"))"),
        "inserts the specific missing arm: {}",
        d.message
    );
    // Symmetric: missing `true` → `(true (trap "TODO: true"))`.
    let d2 = reject_full("(module m (def (main (: b Bool)) (match b (false 2))) (export main))")
        .expect("reject");
    assert_eq!(
        d2.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("(true (trap \"TODO: true\"))"),
        "message: {}",
        d2.message
    );
}

#[test]
fn the_exhaustiveness_add_arm_fix_type_checks_in_one_shot_against_int_arms() {
    // The key property of the `trap`-bodied add-arm fix over the old `unit` body: the covering arm it
    // suggests type-checks in ONE shot even when the sibling arms return a non-Unit type. A `unit` body
    // traded the CDZ0210 for a fresh CDZ0203 "match arms differ: Int64 vs Unit" (a cascade — the fix
    // did not verify); the diverging `trap` (∀a. String → a) unifies with the `Int64` arms, so the
    // repaired program compiles clean. These are the exact arm shapes the fix inserts (verified by the
    // `fix.replacement` assertions in the sibling tests) — here we confirm the RESULT compiles.
    fn compiles_clean(src: &str) {
        assert!(
            reject_full(src).is_none(),
            "the add-arm fix's covering arm must type-check against the Int64 sibling arms (no cascade): \
                 {:?}\nsrc: {src}",
            reject_full(src).map(|d| (d.code, d.message))
        );
    }
    // Each is the ORIGINAL non-exhaustive match with the fix's exact covering arm spliced in. Sibling
    // arms return Int64 — the old `unit` body clashed (CDZ0203); the `trap` body does not.
    compiles_clean(
        "(module m (type C (A) (B) (Cc)) (def (main) (match (C.A) ((A) 1) ((B) 2) (Cc (trap \"TODO: Cc\")))) (export main))",
    );
    compiles_clean(
        "(module m (def (main) (match true (true 1) (false (trap \"TODO: false\")))) (export main))",
    );
    compiles_clean(
        "(module m (def (main) (match (Some 1) ((None) 0) ((Some _p0) (trap \"TODO: Some\")))) (export main))",
    );
    compiles_clean(
        "(module m (def (main) (match 5 (1 10) (2 20) (_ (trap \"TODO\")))) (export main))",
    );
    // And the OLD `unit` body genuinely DID cascade — pin the regression so a revert is caught.
    let with_unit = reject_full(
        "(module m (type C (A) (B) (Cc)) (def (main) (match (C.A) ((A) 1) ((B) 2) (Cc unit))) (export main))",
    );
    assert_eq!(
        with_unit.and_then(|d| d.code).as_deref(),
        Some("CDZ0203"),
        "a `unit` arm body among Int64 arms is the cascade the trap body avoids"
    );
}

#[test]
fn a_non_exhaustive_match_in_a_called_function_is_reported_once_not_duplicated() {
    // A non-exhaustive match in a function that is also CALLED was reported TWICE: once at the def (the
    // def-body check, with the insert-arms fix) and once re-anchored to the call site (the lowering walk
    // inlines the callee and re-reaches the same poison; its fix targets a SYNTHESIZED node). An agent
    // saw one defect as two errors, the second worse (points at the caller, its fix stripped). Now
    // `dedup_faults` drops the copy whose fix targets a non-user node when the same (code, message) is
    // reported with a fix editing a USER node. Exactly ONE CDZ0210 survives, at the match, with its fix.
    let diags = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (type C R G B) (def (f (: c C)) (match c ((R) 1) ((G) 2))) \
               (def (main) (f (R))) (export main))",
    )));
    let ne: Vec<_> = diags
        .iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0210"))
        .collect();
    assert_eq!(
        ne.len(),
        1,
        "a called function's non-exhaustive match reports ONCE, not per call: {ne:?}"
    );
    assert!(
        ne[0].fix.is_some(),
        "the surviving copy is the authoritative one, with its insert-arms fix"
    );
    // SAFETY (the M7 concern): TWO GENUINELY-DISTINCT matches — same missing variant, different defs —
    // must BOTH survive (each fix edits its own user node, so neither is the dropped "non-user" copy).
    let two = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (type C R G B) (def (f (: c C)) (match c ((R) 1) ((G) 2))) \
               (def (g (: c C)) (match c ((R) 9) ((G) 8))) (export f) (export g))",
    )));
    assert_eq!(
        two.iter()
            .filter(|d| d.code.as_deref() == Some("CDZ0210"))
            .count(),
        2,
        "two distinct non-exhaustive matches are NOT merged: {two:?}"
    );
}

// (an_operator_arg_wrap_in_variant_uses_the_readable_lead / an_operator_arg_structural_mismatch_names_the_delta /
// the_wrap_variant_is_derived_generically_from_the_user_sum_not_hardcoded / an_annotation_mismatch_with_no_fitting_variant_carries_no_wrap /
// using_an_option_where_its_payload_is_expected_says_to_match_it — all migrated to corpus 07-type-system, the
// "value-vs-sum / operator-arg mismatch: readable lead + wrap fix + match-it hint" block (9 cases: op-arg
// wrap-in-Some readable lead, op-arg record field-set/field-type + tuple-arity deltas, generic (Wrap …) fix,
// no-fitting-variant no-wrap, Option-payload-expected match-it hint at annotation + binop sites, and the
// unrelated-payload no-hint control). All PASS wasm.)

// (a_record_type_mismatch_is_not_reported_as_a_field_set_difference migrated to corpus 07-type-system: a
// same-field-set record whose field TYPE differs names the field ("field `x` should be Int64, but this one is
// Bool") with (not "missing field") + (not "no such field") — not a field-set difference. PASS wasm.)

// (a_tuple_arity_mismatch_names_the_element_counts migrated to corpus 07-type-system: too-few / too-many
// tuple arities name the element-count delta ("expected a tuple with N elements, but this one has M"), and a
// same-arity element-type mismatch names the specific position ("element 1 should be Bool, but this one is
// Int64") not an arity delta. All 3 PASS wasm.)

#[test]
fn a_collection_element_mismatch_across_kinds_names_no_axis() {
    // RESIDUAL of a_collection_element_mismatch_names_the_differing_axis — its per-axis message faces
    // (list-element / map-key / map-value / both-axes-leftmost, + the no-mechanical-fix control) migrated to
    // corpus 07-type-system. This keeps the cross-KIND no-hint control the corpus grades only as a todo: a
    // `(List Int64)` where a `(Set Int64)` is annotated has agreeing element types but differing KINDS, so no
    // single "axis" is named (no spurious "its elements should be …" hint across collection kinds).
    let kinds =
        reject_full("(module m (def (h (: s (Set Int64))) s) (def (g) (h (list 1))) (export g))")
            .expect("a List where a Set is wanted rejects");
    assert!(
        !kinds.message.contains("its elements should be"),
        "no axis hint across different collection kinds: {}",
        kinds.message
    );
}

// (a_function_type_mismatch_names_the_differing_result_or_arity migrated to corpus 07-type-system: a wrong
// RESULT type ("its result should be Bool, but this one returns Int64"), a wrong ARITY ("expected a function
// taking 1 argument, but this one takes 2"), a same-arity PARAMETER difference resolving at the inner argument
// (no fn-signature tail), the value-annotation-site result face, and the identical-fn no-fault control. All PASS.)

// (a_mismatch_between_two_same_named_distinct_types_disambiguates_the_shared_name migrated to corpus
// 07-type-system: a user `(type Int64 …)` shadowing the prelude makes a prelude-Int64-vs-user-Int64 mismatch
// append "two DIFFERENT types printed with the same name … shadows a built-in" (argument + value-annotation
// sites); an ordinary distinct-name mismatch adds no such tail. All 3 PASS wasm.)

// (an_unsolved_type_variable_renders_as_underscore_not_an_internal_number migrated to corpus 07-type-system:
// an unsolved type var renders as `_` (not the internal `?N`) — `(Result Int64 _)` — at a list-element clash,
// a call argument, and an if-branch join, each pinned with (not "?"). All 3 PASS wasm.)

#[test]
fn a_join_site_scalar_clash_keeps_its_retype_fix_and_no_structural_delta() {
    // RESIDUAL of a_join_site_names_the_structural_delta_not_two_full_renders — its per-member delta faces
    // (list-literal / if-branch / match-arm record-field / tuple-position / tuple-arity) migrated to corpus
    // 07-type-system. This keeps the scalar-clash NO-delta control the corpus grades only as a todo (its
    // fix-only quality assertion): a SCALAR clash at a join gets NO structural-delta tail (the delta fires
    // only for same-kind compounds that differ inside) AND still carries the int-literal->float retype fix.
    let scalar =
        reject_full("(module m (def (g) (list 1 2.0)) (export g))").expect("(list 1 2.0) rejects");
    assert!(
        !scalar.message.contains("should be") && !scalar.message.contains("field"),
        "a scalar clash gets no structural-delta tail: {}",
        scalar.message
    );
    assert!(
        scalar.fix.is_some(),
        "the int-literal->float retype fix still rides along: {:?}",
        scalar.fix
    );
}

#[test]
fn an_annotated_let_binder_mismatch_does_not_cascade_a_contradictory_body_diagnostic() {
    // CASCADE SUPPRESSION (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix —
    // the fix must resolve in ONE shot, not spawn a contradictory follow-on). An annotated let-binder
    // whose initializer disagrees with its annotation reports ONCE at the binder (CDZ0203). A body use
    // types against the DECLARED type (annotation-wins, like an annotated PARAMETER), so it does NOT
    // emit a SECOND diagnostic whose fix undoes the first — exactly as rustc binds `let x: T = e` at `T`
    // for the body.

    // A RECORD field typo: the binder fix says "keep the annotation `foo`, rename the value's `fooo`".
    // The body read `(. r foo)` used to ALSO fault ("record has no field `foo` — did you mean `fooo`?"),
    // a fix that would UNDO the binder fix. Now it is a single diagnostic: the binder mismatch, carrying
    // the value-side rename.
    let rec = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (main) \
                   (let (((: r (Record (foo Int64))) (record (= fooo 1)))) (. r foo))) (export main))",
        )))
    });
    let errs: Vec<_> = rec
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errs.len(),
        1,
        "exactly one diagnostic (the binder mismatch), no contradictory body cascade: {rec:?}"
    );
    assert_eq!(errs[0].code.as_deref(), Some("CDZ0203"), "got: {rec:?}");
    assert!(
        errs[0].message.contains("binder annotated"),
        "the sole diagnostic is the binder mismatch: {}",
        errs[0].message
    );

    // A SCALAR mismatch: `(: n Int64)` bound to `true`, body `(+ n 1)`. The body used to fault a second
    // time ("a Bool and an Int64 are different types"); now it types `n` as the declared `Int64`.
    let scalar = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (main) (let (((: n Int64) true)) (+ n 1))) (export main))",
        )))
    });
    let serrs: Vec<_> = scalar
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        serrs.len(),
        1,
        "a scalar binder mismatch reports once, no arithmetic cascade in the body: {scalar:?}"
    );
    assert!(
        serrs[0].message.contains("binder annotated"),
        "the sole diagnostic is the binder mismatch: {}",
        serrs[0].message
    );

    // NO false suppression: a GENUINE bad field on a WELL-TYPED annotated binder STILL faults (the
    // annotation and value agree; the body simply names a field neither has).
    let genuine = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (main) \
                   (let (((: r (Record (foo Int64))) (record (= foo 1)))) (. r bar))) (export main))",
        )))
    });
    assert!(
        genuine
            .iter()
            .any(|d| d.code.as_deref() == Some("CDZ0212")
                && d.message.contains("has no field `bar`")),
        "a genuine absent field on a well-typed binder still faults: {genuine:?}"
    );
}

#[test]
fn a_join_site_option_or_unapplied_fn_clash_carries_the_annotation_sites_hint() {
    // FIX-PARITY: the annotation site `(: v T)` names an OPTION-vs-payload clash ("match it to handle
    // `None`") and an UNAPPLIED-FUNCTION clash ("apply it to N more arguments"); the PEER-JOIN sites —
    // `if` branches, `match` arms, `list` elements — carried only the structural-delta hints, leaving
    // these two shapes as bare "X vs Y" renders. `peer_type_delta_hint` now tries both hints in BOTH
    // orderings (a peer clash is symmetric — either side may be the Option / the unfinished call).
    let option_note = "the value is optional; match it to handle the absent (`None`) case";
    // `if`: an Option branch against its payload — in EITHER order.
    let if_opt_first = reject_full("(module m (def (f (: b Bool)) (if b (Some 5) 5)) (export f))")
        .expect("if (Option Int64) vs Int64 rejects");
    assert!(
        if_opt_first.message.contains(option_note),
        "if-branch Option-first carries the match-it hint: {}",
        if_opt_first.message
    );
    let if_opt_second = reject_full("(module m (def (f (: b Bool)) (if b 5 (Some 5))) (export f))")
        .expect("if Int64 vs (Option Int64) rejects");
    assert!(
        if_opt_second.message.contains(option_note),
        "if-branch Option-SECOND still carries the hint (symmetric): {}",
        if_opt_second.message
    );
    // `match`: an Option arm body against a payload arm body.
    let m = reject_full("(module m (def (f (: n Int64)) (match n (0 (Some 5)) (_ 5))) (export f))")
        .expect("match (Option Int64) vs Int64 rejects");
    assert!(
        m.message.contains(option_note),
        "match-arm Option clash carries the hint: {}",
        m.message
    );
    // `list`: an Option element against a payload element.
    let list = reject_full("(module m (def (g) (list (Some 5) 5)) (export g))")
        .expect("list (Option Int64) and Int64 rejects");
    assert!(
        list.message.contains(option_note),
        "list-element Option clash carries the hint: {}",
        list.message
    );
    // An UNAPPLIED FUNCTION branch against a scalar — names the missing application.
    let fn_clash = reject_full(
        "(module m (def (h x y) (+ x y)) (def (f (: b Bool)) (if b (h 1) 5)) (export f))",
    )
    .expect("if (-> _ Int64) vs Int64 rejects");
    assert!(
        fn_clash
            .message
            .contains("a function that hasn't been fully applied"),
        "if-branch unapplied-fn clash names the missing application: {}",
        fn_clash.message
    );
    // NO false hint on a plain cross-kind scalar clash (Int vs String) — the peer hint only fires for
    // the Option/fn shapes, exactly as at the annotation site.
    let plain = reject_full("(module m (def (f (: b Bool)) (if b 1 \"x\")) (export f))")
        .expect("(if b 1 \"x\") rejects");
    assert!(
        !plain.message.contains(option_note) && !plain.message.contains("fully applied"),
        "a plain scalar clash gets no Option/fn hint: {}",
        plain.message
    );
}

// (a_nested_compound_mismatch_drills_to_the_exact_leaf_path migrated to corpus 07-type-system: a differing
// nested field/position drills to the dotted leaf path ("field `a.b.c` should be Int64, but this one is
// Bool"; "field `pt.1`…"; "element 0.x…") across 2/3-level records, record-of-tuple and tuple-of-record; the
// drill stops at a deeper field-SET difference (names the immediate field, not a leaf path). All 5 PASS wasm.)

// (a_wrong_type_argument_to_a_prelude_member_op_names_the_operation migrated to corpus 07-type-system: a
// wrong-element-type `List.push` (CDZ0201) / conversion `Int64.of` (CDZ0203) names "`Op` expects an argument
// of type T" + the actual type; a bare operator keeps the generic message (not "expects an argument of type");
// a structurally-mismatched List.push element appends the field-level delta. All 4 PASS wasm.)

#[test]
fn over_applying_a_prelude_member_op_dedupes_the_emit_path_decline() {
    // RESIDUAL of over_applying_a_prelude_member_op_names_the_operation_and_arity — its op+arity message
    // faces (`List.push` takes 2 / `Map.len` takes 1 argument, but N given + delete fix) migrated to corpus
    // 07-type-system. This keeps the two corpus-inexpressible controls: (a) the emit-path wrong-arity decline
    // is DEDUPED so an over-applied member op reports exactly ONE error (a diagnostic COUNT), and (b) a bare
    // operator over-application keeps its own message, not a `.`-member phrasing (its CDZ0201 message has no
    // positive substring the corpus grades cleanly).
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (def (main) ((. Map len) (map (= 1 2)) 99)) (export main))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    );
    let errs: Vec<_> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errs.len(),
        1,
        "one error, decline deduped: {:?}",
        out.diagnostics
    );
    // NO regression: a bare operator over-application keeps its own message (not a `.`-member phrasing).
    let bare = reject_full("(module m (def (g) (+ 1 2 3)) (export g))").expect("(+ 1 2 3) rejects");
    assert!(
        !bare.message.contains("were given"),
        "a bare operator over-application keeps its own message: {}",
        bare.message
    );
}

// (over_applying_a_bare_variant_constructor_names_it migrated to corpus 07-type-system: a bare ctor `(Mk 1 2 3)`
// → "`Mk` takes 2 arguments, but 3 were given" + delete-surplus fix; the member spelling `((. P Mk) 1 2 3)` →
// "`P.Mk` takes 2 arguments, but 3 were given"; an ordinary over-applied fn keeps the anonymous "function of
// arity 1" message (not "were given"). All 3 PASS wasm.)

// (an_unapplied_function_value_names_the_forgotten_call migrated to corpus 07-type-system: a partial
// application where a scalar is expected / as an operator operand names "hasn't been fully applied; apply it
// to N more argument(s)" (+ polished "function value" operator lead + (not "must be the same type here")),
// pluralized for 2+; and the two no-hint controls — an applied result that still differs (positive "Bool") +
// a fn-vs-fn mismatch (positive fn-arity message), each with (not "hasn't been fully applied"). All PASS wasm.)

#[test]
fn a_function_valued_operator_operand_names_the_function_not_the_raw_unify() {
    // A bare arithmetic/comparison operator with a FUNCTION-VALUED operand — a partially-applied prelude
    // op whose fully-applied result is NOT the other operand's type — used to leak the raw scheme-unify
    // "type mismatch: Int64 and (-> Int64 (-> Int64 (Option String))) must be the same type here, but
    // differ" (an internal-clash read that buries the real cause). `String.slice` is `(-> String (->
    // Int64 (-> Int64 (Option String))))`, so `(String.slice s)` is a two-argument-short function; `(+
    // (String.slice s) 1)` puts it where a number is wanted. A function has no arithmetic/order, so this
    // is a genuine kind boundary — now named CDZ0203 "this operation is not defined on a function value".
    // The full application would yield `(Option String)`, not `Int64`, so NO "apply it" hint (honest —
    // calling it would not produce a number); the base message stands alone.
    for src in [
        "(module m (def (f (: s String)) (+ (String.slice s) 1)) (export f))",
        "(module m (def (f (: s String)) (+ 1 (String.slice s))) (export f))",
        "(module m (def (f (: s String)) (if (< (String.slice s) 1) 1 2)) (export f))",
    ] {
        let d = reject_full(src).expect("a function-valued operator operand rejects");
        assert_eq!(d.code.as_deref(), Some("CDZ0203"), "got: {}", d.message);
        assert!(
            d.message
                .contains("this operation is not defined on a function value")
                && !d.message.contains("must be the same type here"),
            "names the function operand, not the raw unify clash: {}",
            d.message
        );
        assert!(
            !d.message.contains("hasn't been fully applied"),
            "no 'apply it' hint when the applied result would not match the other operand: {}",
            d.message
        );
    }
    // When the function fully applied WOULD yield the other operand's type — `(+ (h 1) 2)` for a 2-ary
    // `h : Int64 -> Int64 -> Int64` — the same message appends the actionable "apply it to N more"
    // hint (the forgotten-call story), so both the anonymous kind-boundary cause AND the fix are named.
    let call = reject_full(
        "(module m (def (h (: a Int64) (: b Int64)) (+ a b)) (def (g) (+ (h 1) 2)) (export g))",
    )
    .expect("a partial user-fn operand rejects");
    assert!(
        call.message
            .contains("this operation is not defined on a function value")
            && call
                .message
                .contains("apply it to 1 more argument to get an Int64"),
        "names the function AND the forgotten-call fix: {}",
        call.message
    );
    // NO regression: well-typed arithmetic and a genuine numeric mix are untouched by the fn-operand arm.
    assert!(
        reject_full("(module m (def (f (: a Int64) (: b Int64)) (+ a b)) (export f))").is_none(),
        "well-typed arithmetic on two Int64s still compiles"
    );
    let mix = reject_full("(module m (def (f (: a Int64)) (+ a 2.0)) (export f))")
        .expect("an int/float mix still rejects");
    assert_eq!(
        mix.code.as_deref(),
        Some("CDZ0301"),
        "a numeric mix keeps its CDZ0301, not the fn-operand message: {}",
        mix.message
    );
}

#[test]
fn a_wrong_typed_call_argument_reads_as_an_argument_not_an_annotation() {
    // A wrong-typed argument to a user FUNCTION — `(h true)` where `h`'s parameter is `Int64` — is
    // checked via the parameter's SYNTHESIZED `(: arg paramtype)` wrap, so it shared the value-
    // annotation site's "annotation type Int64 does not match value type Bool" message. But the author
    // wrote NO annotation on `true`; they passed a wrong-typed argument. Now a call argument reads
    // "this argument is a Bool, but a value of type Int64 is expected here" (rustc's "expected `Int64`,
    // found `Bool`" at an argument), while a GENUINE `(: value T)` keeps the annotation wording. The
    // discriminator: the synthesized wrap is a non-user node whose `expr` is the user-written argument.
    let call = reject_full("(module m (def (h (: a Int64)) a) (def (g) (h true)) (export g))")
        .expect("a wrong-typed call argument rejects");
    assert_eq!(
        call.code.as_deref(),
        Some("CDZ0203"),
        "got: {}",
        call.message
    );
    assert!(
        call.message
            .contains("this argument is a Bool, but a value of type Int64 is expected here"),
        "a call argument reads as an argument, not an annotation: {}",
        call.message
    );
    assert!(
        !call.message.contains("annotation type"),
        "no misleading 'annotation' wording for a call argument: {}",
        call.message
    );
    // The coercion fix still rides along — `(h 3.0)` where `a : Int64` keeps the drop-`.0` retype.
    let coerce = reject_full("(module m (def (h (: a Int64)) a) (def (g) (h 3.0)) (export g))")
        .expect("a coercible wrong-typed argument rejects");
    assert!(
        coerce.message.contains("this argument is a Float64")
            && coerce.message.contains("drop the fractional form"),
        "the call-argument wording keeps the coercion fix: {}",
        coerce.message
    );
    assert!(
        coerce.fix.is_some(),
        "the retype fix rides along: {:?}",
        coerce.fix
    );
    // A GENUINE value annotation `(: value T)` KEEPS the "annotation type" wording (the author DID
    // write an annotation there).
    let annot = reject_full("(module m (def (g) (: true Int64)) (export g))")
        .expect("a genuine annotation mismatch rejects");
    assert!(
        annot
            .message
            .contains("annotation type Int64 does not match value type Bool"),
        "a genuine annotation keeps its wording: {}",
        annot.message
    );
    // The per-member structural hint still fires through the call-argument path — a wrong tuple element
    // in an argument names the position AND reads as an argument.
    let compound = reject_full(
        "(module m (def (h (: t (Tuple Int64 Int64))) t) (def (g) (h (tuple 1 true))) (export g))",
    )
    .expect("a wrong-typed compound argument rejects");
    assert!(
        compound
            .message
            .contains("this argument is a (Tuple Int64 Bool)")
            && compound
                .message
                .contains("element 1 should be Int64, but this one is Bool"),
        "the call-argument wording composes with the per-member hint: {}",
        compound.message
    );
}

#[test]
fn an_unreferenced_or_recursive_callees_wrong_argument_names_the_function_and_parameter() {
    // A wrong-typed argument whose parameter the body does NOT reference (`(h true)` where `h`
    // ignores `a`), or whose callee is RECURSIVE (its reduction declines), is reported by the
    // CALL-SITE unify (step 1), not the synthesized-annotation path (step 2, which is silent for
    // these). Step 1 used to emit the raw "type mismatch: Int64 and Bool must be the same type here"
    // — the same defect a referenced-param arg reads as "this argument is a Bool…" (M106). Now step 1
    // gives the SAME argument phrasing AND, having the call head + parameter in hand (step 2 does
    // not), names them: "the argument for `h`'s parameter `a` is a Bool, but a value of type Int64 is
    // expected here".
    // UNREFERENCED parameter — the body `0` never uses `a`.
    let unref = reject_full("(module m (def (h (: a Int64)) 0) (def (g) (h true)) (export g))")
        .expect("a wrong argument to an unreferenced parameter rejects");
    assert!(
            unref
                .message
                .contains("the argument for `h`'s parameter `a` is a Bool, but a value of type Int64 is expected here"),
            "names the function + parameter, argument phrasing: {}",
            unref.message
        );
    assert!(
        !unref.message.contains("must be the same type here"),
        "no raw unify wording: {}",
        unref.message
    );
    // RECURSIVE callee — the reduction declines, so step 2 never runs; step 1 is the sole reporter.
    let rec = reject_full(
        "(module m (def (h (: a Int64)) (if (< a 0) 0 (h (- a 1)))) (def (g) (h true)) (export g))",
    )
    .expect("a wrong argument to a recursive callee rejects");
    assert!(
        rec.message
            .contains("the argument for `h`'s parameter `a` is a Bool"),
        "a recursive callee's wrong argument is named too: {}",
        rec.message
    );
    // The sum-wrap fix still rides along at the call site (an unreferenced Option parameter).
    let wrap =
        reject_full("(module m (def (h (: o (Option Int64))) 0) (def (g) (h 5)) (export g))")
            .expect("a bare payload to an unreferenced Option parameter rejects");
    assert!(
        wrap.message
            .contains("the argument for `h`'s parameter `o`")
            && wrap.fix.as_ref().map(|f| f.kind) == Some(crate::abi::FixKind::Wrap),
        "the sum-wrap fix rides along the named call-site message: {} / {:?}",
        wrap.message,
        wrap.fix
    );
    // A SHARED type variable an EARLIER argument already solved renders CONCRETELY, not as `_`. In
    // `(def (pair (: t Type) (: x t) (: y t)) x)` the two `t`-annotated params share one var; typing the
    // arg for `x` binds it, so the mismatch report for `y` must show that SOLVED type — the call-site
    // unify now applies the accumulated substitution before rendering the expected type. `(pair Int64 1
    // true)`: `x = 1` pins `t`'s var to Int64, so `y`'s expected type is Int64, not the unsolved-`Ty::Var`
    // "_" it used to print ("a value of type `_` is expected here").
    let shared = reject_full(
            "(module m (def (pair (: t Type) (: x t) (: y t)) x) (def (g) (pair Int64 1 true)) (export g))",
        )
        .expect("a sibling-param mismatch on a shared type var rejects");
    assert!(
        shared
            .message
            .contains("parameter `y` is a Bool, but a value of type Int64 is expected here"),
        "the shared var renders the type the sibling arg solved (Int64), not `_`: {}",
        shared.message
    );
    assert!(
        !shared.message.contains("value of type `_`")
            && !shared.message.contains("value of type _"),
        "no unsolved-var `_` in the expected-type render: {}",
        shared.message
    );
}

#[test]
fn a_bitwise_operator_on_a_non_integer_operand_names_the_integer_requirement() {
    use crate::testkit::parse;
    // The bitwise/shift operators `& | ^ << >>` carry the `∀a. (Int a) → (Int a) → (Int a)` scheme, so
    // a non-Int operand made the generic scheme-unify ground the var to `Int64` and report the opaque
    // "type mismatch: Int64 and Bool must be the same type here" (an internal-clash read). They are NOT
    // in the arith/comparison cross-kind list (those share numeric-coercion hints a bitwise op lacks),
    // so this had no specific message. Now named: a bitwise/shift op is integer-only. A BOOL operand
    // also gets the likely-intent hint (`and`/`or` are the boolean connectives — the C/Python habit).
    // `want_fix` is the boolean-connective REPLACE the operator head should carry: `&`→`and`,
    // `|`→`or` on Bool operands; `^`/shifts have no boolean twin so no fix, and a non-Bool operand
    // (Char/String) gets the message-only hint with no fix.
    for (src, ty, want_hint, want_fix) in [
        (
            "(module m (def (f (: a Bool) (: b Bool)) (& a b)) (def (main) 0) (export main))",
            "Bool",
            true,
            Some("and"),
        ),
        (
            "(module m (def (f (: a Bool) (: b Bool)) (| a b)) (def (main) 0) (export main))",
            "Bool",
            true,
            Some("or"),
        ),
        (
            "(module m (def (f (: a Bool) (: b Bool)) (^ a b)) (def (main) 0) (export main))",
            "Bool",
            true,
            None, // xor has no boolean connective twin — hint only, no fix
        ),
        (
            "(module m (def (f (: c Char)) (<< c 1)) (def (main) 0) (export main))",
            "Char",
            false,
            None,
        ),
        (
            "(module m (def (f (: a String)) (| a a)) (def (main) 0) (export main))",
            "String",
            false,
            None,
        ),
    ] {
        let d = crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| {
                d.message
                    .contains("a bitwise/shift operator needs integer operands")
            })
            .unwrap_or_else(|| panic!("a bitwise op on a non-int operand must be named: {src}"));
        assert_eq!(d.code.as_deref(), Some("CDZ0203"), "got: {}", d.message);
        assert!(
            d.message
                .contains(&format!("a value of type {ty} was given"))
                && !d.message.contains("must be the same type here"),
            "names the bad operand type, not a phantom clash: {}",
            d.message
        );
        assert_eq!(
            d.message.contains("use `and`/`or`"),
            want_hint,
            "the `and`/`or` hint appears for a Bool operand only: {}",
            d.message
        );
        // A `&`/`|` on Bools carries an APPLYABLE Replace on the operator head (swap the bitwise op
        // for its boolean connective); `^`/shifts / non-Bool operands carry no fix.
        match want_fix {
            Some(connective) => {
                let fix = d.fix.as_ref().unwrap_or_else(|| {
                    panic!("a `&`/`|` on Bools carries a connective fix: {src}")
                });
                assert_eq!(fix.kind, crate::abi::FixKind::Replace);
                assert_eq!(
                    fix.replacement, connective,
                    "swaps the bitwise op for its boolean connective: {src}"
                );
                assert!(
                    !fix.verified,
                    "the connective swap is a heuristic (intent guess)"
                );
            }
            None => assert!(
                d.fix.is_none(),
                "no connective fix for `^`/shifts / non-Bool operands: {src} fix={:?}",
                d.fix
            ),
        }
    }
    // ROUND-TRIP: applying the `&`→`and` fix yields a program with no bitwise-operand fault — the
    // connective swap is a real repair (a boolean `and` on Bools type-checks), witnessed by compiling.
    assert!(
        !crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (f (: a Bool) (: b Bool)) (and a b)) (def (main) 0) (export main))"
        )))
        .iter()
        .any(|d| d
            .message
            .contains("a bitwise/shift operator needs integer operands")),
        "applying the `&`→`and` fix clears the bitwise-operand fault"
    );
    // NO false positive: valid integer bitwise/shift, and the boolean connective `and` on Bools.
    for ok in [
        "(module m (def (f (: a Int64) (: b Int64)) (& a b)) (def (main) (f 5 3)) (export main))",
        "(module m (def (f (: x Int64)) (<< x 2)) (def (main) (f 1)) (export main))",
        "(module m (def (f (: a Bool) (: b Bool)) (and a b)) (def (main) 0) (export main))",
    ] {
        assert!(
            !crate::diagnostics(&mut crate::db::Db::load(parse(ok)))
                .iter()
                .any(|d| d
                    .message
                    .contains("a bitwise/shift operator needs integer operands")),
            "a valid integer bitwise op / boolean `and` is not flagged: {ok}"
        );
    }
}

// (a_bare_literal_past_int64_is_malformed_not_out_of_range migrated to corpus: the DECIMAL bare-overflow
// (9223372036854775808 → CDZ0201) + Int64.max-fits faces are the existing 01-literals cases; the HEX
// bare-overflow (0xFFFFFFFFFFFFFFFF → CDZ0201) + the UInt64-max-annotated no-misfire control are added to
// 01-literals; the explicit-width (: 256 (Int 8)) → CDZ0302 control is 06-numeric-model over-width. PASS wasm.)

#[test]
fn a_high_uint64_literal_operand_takes_uint64_from_context() {
    // A bare literal in [2^63, 2^64-1] as an OPERAND of a binary op whose sibling is a UInt64 value
    // takes UInt64 from that operand (numeric-model.md §a constraint on a literal takes precedence),
    // so it is NOT the malformed-out-of-range-for-Int64 fault a bare unannotated literal would be. The
    // gap was UInt64-only: only it has representable values above Int64.max, so the fit-check against
    // the i64 default rejected a full-width mask while UInt8/UInt32 highs (which fit i64) sailed through.
    let ok = |src: &str| {
        assert_eq!(
            reject_code(src),
            None,
            "a high UInt64 literal operand must take UInt64 from context: {src}"
        );
    };
    // Full-width mask (2^64-1), addition of 2^63 (i64::MAX+1), and a comparison — all against a UInt64.
    ok("(module m (def (main (: x UInt64)) (& x 18446744073709551615)) (export main))");
    ok("(module m (def (main (: x UInt64)) (+ x 9223372036854775808)) (export main))");
    ok("(module m (def (main (: x UInt64)) (if (< x 18446744073709551615) 1 0)) (export main))");
    // NO OVER-ACCEPTANCE: a bare literal past i64 with NO integer-operand context is still CDZ0201; a
    // value too big even for the contextual UInt64 (2^64) is still rejected (now naming UInt64).
    // A bare over-i64 literal that STILL FITS UNSIGNED 64 (`18446744073709551615` = 2^64-1) is
    // malformed as a bare (signed-default) literal, but has a concrete fixed type — so it names its
    // range AND offers the "annotate `UInt64`" repair (its range holds the value), NOT the
    // "widest fixed-size integer" dead-end.
    let d = reject_full("(module m (def (main) 18446744073709551615) (export main))")
        .expect("a bare literal past i64 is rejected");
    assert_eq!(
        d.code.as_deref(),
        Some("CDZ0201"),
        "a bare literal past i64 with no context is still malformed"
    );
    assert!(
        d.message.contains("the valid range is") && d.message.contains("UInt64"),
        "a fits-u64 bare literal names its range + the UInt64 route; got {}",
        d.message
    );
    assert_eq!(
        d.fix.as_ref().map(|f| f.replacement.as_str()),
        Some(format!("(: {} UInt64)", crate::abi::WRAP_HOLE)).as_deref(),
        "it carries the annotate-`UInt64` wrap fix: {}",
        d.message
    );
    // A value past UInt64.max (2^64) has NO fixed type — but `BigInt` holds an integer literal of any
    // magnitude, so it names "no fixed-size integer is wider" AND offers the total "annotate `BigInt`"
    // repair (a value the arbitrary-precision type represents in one shot).
    let past = reject_full("(module m (def (main) 99999999999999999999) (export main))")
        .expect("a value past u64 is rejected");
    assert!(
        past.message.contains("no fixed-size integer is wider") && past.message.contains("BigInt"),
        "a past-u64 bare literal names the BigInt route; got {}",
        past.message
    );
    assert_eq!(
        past.fix.as_ref().map(|f| f.replacement.as_str()),
        Some(format!("(: {} BigInt)", crate::abi::WRAP_HOLE)).as_deref(),
        "it carries the annotate-`BigInt` wrap fix: {}",
        past.message
    );
    // The BigInt fix actually clears the fault — a huge literal annotated `BigInt` compiles.
    assert!(
        reject_full("(module m (def (g) (: 99999999999999999999 BigInt)) (export g))")
            .is_none_or(|d| !d.message.contains("out of range")),
        "annotating the past-u64 literal `BigInt` resolves the range fault"
    );
    let d = reject_full(
        "(module m (def (main (: x UInt64)) (& x 18446744073709551616)) (export main))",
    )
    .expect("a value past UInt64.max must still be rejected");
    assert_eq!(d.code.as_deref(), Some("CDZ0201"), "got: {}", d.message);
    assert!(
        d.message.contains("UInt64"),
        "the reject names the overflowed contextual type: {}",
        d.message
    );
}

#[test]
fn a_constant_algebraic_identity_over_a_high_uint64_operand_folds_not_declines() {
    // REGRESSION (fold): a constant algebraic identity (`x+0`/`0+x`/`x-0`/`x*1`/`1*x`) over a UInt64
    // operand in [2^63, 2^64-1] must FOLD to the operand, NOT decline. `fold_arith` evaluates over
    // `i64`, so a UInt64 constant ≥ 2^63 (e.g. `UInt64.max = 2^64-1`) has no `i64` and `fold_arith`
    // returned CDZ0304 ("constant operand does not fit the integer width") — a SPURIOUS reject of valid
    // unsigned arithmetic. FIX (lower_arith): try the width-agnostic `arith_identity` BEFORE the i64
    // fold when an operand is out of i64 range — the identity returns an operand unchanged (correct at
    // any width). Both-constant folds previously dispatched straight to `fold_arith`, never reaching the
    // identity (which lived only in the not-both-constant fallthrough).
    //
    // These are OPERATIONS (both operands constant), distinct from the bare-literal acceptance the
    // sibling `a_high_uint64_literal_operand_takes_uint64_from_context` pins. All must compile clean.
    let compiles = |src: &str| {
        assert_eq!(
            reject_code(src),
            None,
            "a constant identity over a high UInt64 operand must fold, not decline (was a spurious \
                 CDZ0304 from the i64-only `fold_arith`): {src}"
        );
    };
    compiles(
        "(module m (def (main) (+ (: 18446744073709551615 UInt64) (: 0 UInt64))) (export main))",
    );
    compiles(
        "(module m (def (main) (+ (: 0 UInt64) (: 18446744073709551615 UInt64))) (export main))",
    );
    compiles(
        "(module m (def (main) (- (: 18446744073709551615 UInt64) (: 0 UInt64))) (export main))",
    );
    compiles(
        "(module m (def (main) (* (: 18446744073709551615 UInt64) (: 1 UInt64))) (export main))",
    );
    compiles(
        "(module m (def (main) (* (: 1 UInt64) (: 18446744073709551615 UInt64))) (export main))",
    );
    // 2^63 exactly (i64::MAX + 1) — the boundary the i64 fold first misses.
    compiles(
        "(module m (def (main) (+ (: 9223372036854775808 UInt64) (: 0 UInt64))) (export main))",
    );

    // NO OVER-ACCEPTANCE: a genuine unsigned OVERFLOW (not an identity — `u64max + 1`) still declines
    // (the general u64-fold is a separable follow-up; the boundary must not silently miscompile).
    assert_eq!(
        reject_code(
            "(module m (def (main) (+ (: 18446744073709551615 UInt64) (: 1 UInt64))) (export main))"
        )
        .as_deref(),
        Some("CDZ0304"),
        "a genuine constant unsigned overflow must still be rejected, not folded to a wrong value"
    );
    // And normal in-range i64 identities are unaffected (the fast path is byte-identical).
    assert_eq!(
        reject_code("(module m (def (main) (+ 5 0)) (export main))"),
        None
    );
    assert_eq!(
        reject_code("(module m (def (main) (* 7 1)) (export main))"),
        None
    );

    // The folded VALUE (the operand itself) is confirmed end-to-end by corpus 06 "a constant algebraic
    // identity over a high UInt64 operand folds to the operand, not a spurious reject" (8583) — this
    // keeps only the compile-time fold-vs-CDZ0304 decline witness (`reject_code`, no runtime).
}

#[test]
fn a_non_identity_constant_op_over_a_high_uint64_operand_folds_exactly_or_traps_on_overflow() {
    // REGRESSION (fold, follow-up to the identity fix): a NON-identity constant op (`/`/`-`/`%`/`+`)
    // over a UInt64 operand ≥ 2^63 must fold over EXACT `IntValue` and range-check the result against
    // the solved width — NOT decline via the i64-only `fold_arith` (which has no `i64` for a ≥2^63
    // operand and rejected it CDZ0304 "constant operand does not fit the integer width"). The identity
    // fix handled `x+0`/`x*1`; this handles the general case. FIX (lower_arith): when an operand is out
    // of i64 range and no identity fired, fold `Add`/`Sub`/`Mul`/`Div`/`Rem` over `IntValue` exactly,
    // then reuse the same `fits_width` range-check the i64 path applies.
    let compiles = |src: &str| {
        assert_eq!(
            reject_code(src),
            None,
            "a non-identity constant op over a high UInt64 operand whose result fits must fold, not \
                 decline (was a spurious CDZ0304 from the i64-only fold): {src}"
        );
    };
    // Result fits UInt64 → folds exactly.
    compiles(
        "(module m (def (main) (/ (: 18446744073709551614 UInt64) (: 2 UInt64))) (export main))",
    );
    compiles(
        "(module m (def (main) (- (: 18446744073709551615 UInt64) (: 1 UInt64))) (export main))",
    );
    compiles(
        "(module m (def (main) (% (: 18446744073709551615 UInt64) (: 10 UInt64))) (export main))",
    );
    compiles(
        "(module m (def (main) (+ (: 9223372036854775808 UInt64) (: 5 UInt64))) (export main))",
    );

    // Genuine unsigned OVERFLOW (result exceeds the solved width) still traps CDZ0304 — the exact fold
    // does NOT silently wrap.
    assert_eq!(
        reject_code(
            "(module m (def (main) (* (: 18446744073709551615 UInt64) (: 2 UInt64))) (export main))"
        )
        .as_deref(),
        Some("CDZ0304"),
        "a wide constant op whose result overflows the solved width must still trap"
    );
    assert_eq!(
            reject_code(
                "(module m (def (main) (+ (: 9223372036854775808 UInt64) (: 9223372036854775808 UInt64))) (export main))"
            )
            .as_deref(),
            Some("CDZ0304"),
            "2^63 + 2^63 = 2^64 overflows UInt64 → CDZ0304"
        );
    // Divide-by-a-constant-zero over a wide operand still traps CDZ0304 (divmod → None).
    assert_eq!(
        reject_code(
            "(module m (def (main) (/ (: 18446744073709551615 UInt64) (: 0 UInt64))) (export main))"
        )
        .as_deref(),
        Some("CDZ0304"),
        "a wide constant divide by zero must still trap"
    );

    // The folded VALUES (exact IntValue arithmetic, i64-truncation-free) are confirmed end-to-end by
    // corpus 06 "a constant division over a high UInt64 operand folds exactly to the quotient" (8616)
    // and its sub/mod/add-in-range siblings (8623/8629/8635) — this keeps only the compile-time
    // fold-vs-CDZ0304 decline witness (`reject_code`, no runtime).
}

#[test]
fn a_small_operand_shift_whose_result_exceeds_i64_folds_at_the_unsigned_width_but_signed_shr_sign_extends()
 {
    // REGRESSION (fold): `(<< (: 1 UInt64) 63)` = 2^63 FITS UInt64 but overflows i64. Both operands fit
    // i64, so the wide-operand fold path was NOT reached, and `fold_arith`'s `checked_shl_i64`
    // overflow-checked the shifted result against Int64 → a spurious CDZ0304 "overflows Int64". FIX:
    // `fold_shift_bitwise_at_width` folds a shift/bitwise over the SOLVED width for UNSIGNED types (even
    // with small operands), range-checking the result against that width, not i64.
    assert_eq!(
        reject_code("(module m (def (main) (<< (: 1 UInt64) (: 63 UInt64))) (export main))"),
        None,
        "1 << 63 at UInt64 = 2^63 fits the unsigned width and must fold, not decline as an Int64 overflow"
    );
    // The folded VALUES are confirmed end-to-end by corpus 06: "a constant shift-left whose result
    // exceeds i64 but fits the UInt64 width folds" (1<<63 = 2^63) and "a constant SIGNED shift-right
    // sign-extends" (-256 >> 4 = -16, the unsigned-width fold bails on signed). This keeps only the
    // compile-time fold-vs-CDZ0304 decline witnesses (`reject_code`, no runtime).
    // A small signed `<<` whose result FITS still folds (the i64 path owns signed; `-8 << 1 = -16` fits
    // Int64). (This is the fold-not-reject case — NOT an overflow; see the overflow assertion below.)
    assert!(
        reject_code("(module m (def (main) (<< (- 0 8) 1)) (export main))").is_none(),
        "a small signed `<<` that fits still folds"
    );
    // A genuine signed `<<` OVERFLOW is rejected CDZ0304 (the previously-untested case the mislabeled
    // comment claimed): `1 << 63` at signed Int64 = 2^63, which overflows Int64 (max 2^63-1). The signed
    // i64 fold path (fold_arith/checked_shl_i64) catches it — the unsigned-width fold bails on signed.
    assert_eq!(
        reject_code("(module m (def (main) (<< (: 1 (Int 64)) (: 63 (Int 64)))) (export main))")
            .as_deref(),
        Some("CDZ0304"),
        "a signed `<<` that overflows Int64 (1 << 63 = 2^63 > Int64.max) must be rejected"
    );
}

// (an_out_of_range_literal_names_the_valid_range migrated to corpus 06-numeric-model, the "CDZ0302 names the
// valid range" block: "an out-of-range signed literal names its valid range in the diagnostic" (`(: 128 Int8)`
// → message "-128..=127"), "an out-of-range unsigned literal names a range starting at zero" (`(: 256 UInt8)`
// → message "0..=255"), and "the widest unsigned range bound renders exactly (u128 arithmetic, not i64)"
// (`(: 18446744073709551616 UInt64)` → message "0..=18446744073709551615"). All three graded PASS on wasm.)

// [migrated → spec/semantics/06-numeric-model.sexp] a_suffixed_bigint_literal_annotated_or_passed_to_int64_faults_once_as_a_type_mismatch:
// a suffixed `999…N` literal carries type BigInt, so annotating it Int64 / passing it to an Int64 param is a
// CDZ0203 type mismatch (BigInt ≠ Int64), NOT the range-fit CDZ0302 a bare over-width literal takes. Corpus 06
// cases: annotation, argument-position, and FITS-range (`5N`) faces all assert CDZ0203 (message "BigInt" +
// "must be the same type") with (not "does not fit") pinning the ABSENCE of the double-reported range framing;
// the BARE (unsuffixed) over-Int64 counterpart still range-checks CDZ0302 (message "does not fit"). All PASS on wasm.

#[test]
fn a_bare_literal_grounds_to_bigint_in_a_constructor_payload_position() {
    // A bare, un-suffixed integer literal written as a constructor argument whose DECLARED payload type
    // is `BigInt` GROUNDS to `Ty::BigInt` — `(W 42)` for `(type W (W BigInt))` compiles, where it USED
    // to decline CDZ0201 "payload declared BigInt, but Int64 applied". Operator-approved contextual
    // grounding (`numeric-model.md`: an integer literal grounds to BigInt losslessly, and an explicit
    // context takes precedence over the declared default — a grounding, not a promotion; the same
    // mechanism as grounding a bare literal to a narrow width). Unblocks the quoted-Ast `(Ast.Int 42)`
    // directive (a bare literal against the BigInt-flipped payload, ~97 sites, no `(: … BigInt)` noise).
    for src in [
        "(module m (def (main) (match (W 42) ((W x) x))) (type W (W BigInt)) (export main))",
        // A value that OVERFLOWS i64 — the whole point of BigInt: it grounds LOSSLESSLY (a bare literal
        // this large would be a CDZ0302 range fault in an Int64 context; against BigInt it just grounds).
        "(module m (def (main) (match (W 99999999999999999999999) ((W x) x))) (type W (W BigInt)) \
             (export main))",
    ] {
        assert!(
            reject_code(src).is_none(),
            "a bare literal grounds to BigInt in a ctor payload (was CDZ0201): {src} → {:?}",
            reject_code(src)
        );
    }
    // DISCIPLINE (operator's load-bearing guard): the grounding fires ONLY for a BARE, un-suffixed,
    // uncomputed literal. A COMPUTED Int64 or an explicitly Int64-typed value in a BigInt payload
    // position is a GENUINE mismatch and MUST still decline (BigInt never silently promotes from Int64).
    assert_eq!(
        reject_code(
            "(module m (def (g (: n Int64)) n) (def (main) (W (g 5))) (type W (W BigInt)) \
                 (export main))"
        )
        .as_deref(),
        Some("CDZ0201"),
        "a COMPUTED Int64 in a BigInt payload still declines (grounding is bare-literal-only)"
    );
    assert_eq!(
        reject_code("(module m (def (main) (W (: 42 Int64))) (type W (W BigInt)) (export main))")
            .as_deref(),
        Some("CDZ0201"),
        "an explicitly Int64-typed value in a BigInt payload still declines (typed away from bare)"
    );
}

#[test]
fn a_bare_literal_grounds_to_bigint_in_a_list_element_position() {
    // SLICE 2 of the contextual BigInt grounding: a bare, un-suffixed integer literal written as an
    // ELEMENT of a list literal annotated `(List BigInt)` grounds to BigInt — `(: (list 1 2 3) (List
    // BigInt))` compiles, where it USED to decline CDZ0203 ("elements should be BigInt, but these are
    // Int64"). The collection-element analogue of the ctor-payload grounding (slice 1) — v-metaprogramming's
    // hand-written `(Ast.Int N)` inside a `(list …)`. Grounds each bare element; the list's value type
    // becomes `(List BigInt)`, so the annotation matches.
    for src in [
        "(module m (def (main) (: (list 1 2 3) (List BigInt))) (export main))",
        // An i64-overflowing element grounds LOSSLESSLY (would be a range fault against Int64).
        "(module m (def (main) (: (list 1 99999999999999999999999) (List BigInt))) (export main))",
    ] {
        assert!(
            reject_code(src).is_none(),
            "bare list elements ground to BigInt (was CDZ0203): {src} → {:?}",
            reject_code(src)
        );
    }
    // DISCIPLINE: only a BARE element grounds. A COMPUTED element stays Int64, so a `(list 1 (g 2))`
    // annotated `(List BigInt)` is HETEROGENEOUS (BigInt vs Int64) and MUST still decline — the
    // grounding never silently promotes the computed Int64.
    assert!(
        reject_code(
            "(module m (def (g (: n Int64)) n) (def (main) (: (list 1 (g 2)) (List BigInt))) \
                 (export main))"
        )
        .is_some(),
        "a computed Int64 list element in a (List BigInt) still declines (grounding is bare-only)"
    );
    // A `(List Int64)` list is UNCHANGED (bare elements keep the Int64 default).
    assert!(
        reject_code("(module m (def (main) (: (list 1 2 3) (List Int64))) (export main))")
            .is_none(),
        "a (List Int64) of bare literals is unaffected"
    );
}

#[test]
fn a_float_literal_that_overflows_float32_is_out_of_range() {
    // The float analogue of an out-of-range integer literal: `(: 1.0e300 Float32)` is finite as the
    // default `Float64` but rounds to `±inf` in `Float32` (a value with no written form), so CDZ0302 —
    // it was silently accepted before (only the `Float64` bare-literal overflow was caught). Fires at
    // BOTH a value annotation and a let-binder annotation (the shared `literal_width_fault`).
    for src in [
        "(module m (def (main) (: 1.0e300 Float32)) (export main))",
        "(module m (def (main) (let (((: x Float32) 1.0e300)) x)) (export main))",
    ] {
        let d = reject_full(src).expect("a Float32-overflowing literal is rejected");
        assert_eq!(d.code.as_deref(), Some("CDZ0302"), "got: {}", d.message);
        assert!(
            d.message.contains("Float32") && d.message.contains("infinity"),
            "names Float32 + the overflow-to-inf cause: {}",
            d.message
        );
        // The retype fix: `Float64` holds the value (the literal's own default width), the float twin
        // of the integer width-widen / BigInt fix. Replaces the annotation.
        assert_eq!(
            d.fix.as_ref().map(|f| f.replacement.as_str()),
            Some("Float64"),
            "retypes the annotation to the wider float: {}",
            d.message
        );
    }
    // The `(Float 32)` COMPOUND spelling gets the same `Float64` retype (the fix rewrites the whole
    // type-expr, either spelling).
    let compound = reject_full("(module m (def (main) (: 1.0e40 (Float 32))) (export main))")
        .expect("a (Float 32) overflow is rejected");
    assert_eq!(
        compound.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("Float64"),
        "the compound (Float 32) spelling also retypes to Float64: {}",
        compound.message
    );
    // ROUND TRIP: applying the widen (retype the annotation to `Float64`) recompiles clean — `Float64`
    // is the literal's own default width, so it holds the value that overflowed `Float32`. Both the
    // bare `Float32` and the compound `(Float 32)` cases repair to the same clean `Float64` form.
    for applied in [
        "(module m (def (main) (: 1.0e300 Float64)) (export main))",
        "(module m (def (main) (: 1.0e40 Float64)) (export main))",
    ] {
        assert!(
            reject_full(applied).is_none(),
            "applying the Float64 widen must recompile clean: {applied}"
        );
    }
    // NO false positive: a value that FITS Float32, and the SAME magnitude annotated Float64 (its
    // finite range holds it), both compile clean.
    assert!(reject_full("(module m (def (main) (: 3.0e38 Float32)) (export main))").is_none());
    assert!(reject_full("(module m (def (main) (: 1.5 Float32)) (export main))").is_none());
    assert!(reject_full("(module m (def (main) (: 1.0e300 Float64)) (export main))").is_none());
}

// (a_constant_argument_is_range_checked_against_a_narrow_parameter_width migrated to corpus 06-numeric-model:
// an out-of-range constant arg to a narrow param is range-checked (CDZ0302) through the def-call / inline-lambda
// / annotated-let-binder paths; a const arith overflow is CDZ0304; in-range runs; a Bool-vs-narrow clash is
// CDZ0203. 9 cases; bare-param control covered by generic bare-param cases. All PASS wasm.)

// [migrated → spec/semantics/09-functions.sexp] an_argument_fault_is_reported_whether_or_not_the_parameter_is_used:
// an argument fault surfaces whether the parameter is USED or DEAD (the linear-fault-walk drops the raw-arg
// descent for a used param, relying on the reduced body; a dead param's arg is still descended). Corpus 09
// faces: dead-param unbound-arg → CDZ0101 ("a dead (unreferenced) argument is still checked"), dead-params
// well-formed → runs ("a function using only its first parameter accepts…"), USED-param unbound-arg → CDZ0101
// ("…surfaced via the reduced body"), dead-param malformed-app (5 3) → CDZ0201 ("a non-unbound fault kind"). All PASS.

// (under_applying_a_unary_variant_constructor_is_a_type_error migrated to corpus 09-functions, the
// low-arity mirror after the over-applying-a-constructor cases: `(Some)` → CDZ0201 (message "`Some` needs
// its payload argument")(message "`(Some <value>)`") [generic payload omits the "carries" clause]; a
// concrete `(T.Wrap)` → CDZ0201 (message "`Wrap` needs its payload argument")(message "it carries an
// Int64"); + 2 controls that RUN: a nullary `(None)` constructs (→ 0), a correctly-applied `(Some 5)`
// compiles + matches (→ 5). --case grades codes + messages + run values. The NOT-"carries" negative on
// the generic Some is the inexpressible remainder, covered by the positive message halves.)
#[test]
fn symbol_reader_sugar_and_nominal_boundary() {
    // A Symbol is NOMINAL over String: comparing the two across the boundary is CDZ0202 with an
    // actionable `Symbol.of` wrap fix. (The `#"text"` reader sugar reads to the same value as
    // `(Symbol.of "text")` — a runtime value exercised by the corpus, not here; this test keeps the
    // white-box fix-detail assertion the corpus cannot express.)
    // A Symbol compared to the plain String it wraps is a nominal-boundary type error (CDZ0202), on
    // either operand order — NOT the generic CDZ0203, and NOT silently `false`. The reject now carries
    // a WRAP fix that interns the STRING operand into a Symbol via the total `Symbol.of` (bringing both
    // sides to Symbol) — the Symbol twin of the newtype-unwrap fix, so the diagnostic is actionable.
    for src in [
        "(module m (def (main) (= \"x\" (Symbol.of \"x\"))) (export main))",
        "(module m (def (main) (= (Symbol.of \"x\") \"x\")) (export main))",
    ] {
        let d = reject_full(src).expect("Symbol-vs-String must reject");
        assert_eq!(d.code.as_deref(), Some("CDZ0202"), "src: {src}");
        let fix = d
            .fix
            .expect("the Symbol-vs-String reject carries a `Symbol.of` wrap fix");
        assert_eq!(fix.kind, crate::abi::FixKind::Wrap);
        assert_eq!(
            fix.replacement,
            format!("(Symbol.of {})", crate::abi::WRAP_HOLE),
            "wraps the String operand in `Symbol.of`: {src}"
        );
        assert!(
            !fix.verified,
            "the author may have meant to compare as strings instead → heuristic"
        );
    }
}

/// A Symbol as a TUPLE ELEMENT of a runtime compound `=` — a Symbol IS a String byte-leaf handle at run
/// time, so a Symbol element boxes/reads-back/compares exactly like a String element (which already
/// worked). Before, `box_op_ty`/`get_op_ty` lacked `Ty::Symbol` (only `Ty::String`), so a Symbol tuple
/// element declined "needs the value heap"; the wasm twin of v-rust-backend's `Ty::Symbol → String` rep
/// (v-property-testing found the asymmetry). `(tuple (Symbol.of "a") n)` compares equal to itself.
#[test]
fn symbol_of_a_non_string_reports_one_error_not_a_misleading_runtime_string_decline() {
    // `(Symbol.of 5)` is a type error (`Symbol.of : String → Symbol`, applied to Int64) — CDZ0203.
    // It used to ALSO emit the emit-path decline "Symbol.of on a runtime string is not yet interned",
    // which is a LIE (5 is not a string at all) AND a second `error:`. Now the decline is suppressed
    // (an uncoded decline at a node that carries a coded reject is shadowed by it), so the type error
    // is the ONE story. (A genuine runtime STRING now interns via a byte-compact retag — see
    // `a_runtime_string_interns_to_a_symbol_by_content`; the non-string case here is still a type error.)
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (def (main) (Symbol.of 5)) (export main))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    );
    let errors: Vec<&crate::abi::Diagnostic> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "Symbol.of on a non-string = one type error, got: {:?}",
        out.diagnostics
    );
    assert_eq!(errors[0].code.as_deref(), Some("CDZ0203"));
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message.contains("runtime string")),
        "the misleading 'runtime string' decline must not accompany the type error"
    );
}

#[test]
fn an_export_naming_no_definition_is_reported_by_check() {
    // `(export nope)` with no `(def nope …)` is ill-formed — the public surface must name real
    // definitions. It used to be caught only in the emit-path LAYOUT (so `compile` failed but
    // `cdz check`'s Diagnostics query MISSED it). Now it is a coded CDZ0101 in `collect_faults`, so
    // BOTH surfaces report it, anchored at the `(export …)` clause and (for a typo) with a suggestion.
    // Use the DIAGNOSTICS query — the path `cdz check` runs (`collect_faults`) — where the fault is
    // the coded CDZ0101.
    let mut db = crate::db::Db::load(parse("(module m (def (main) 1) (export mian))"));
    let diags = crate::compile::diagnostics(&mut db);
    let d = diags
        .iter()
        .find(|d| {
            d.severity == crate::abi::Severity::Error && d.message.contains("names no definition")
        })
        .expect("check reports an export naming no definition");
    assert_eq!(
        d.code.as_deref(),
        Some("CDZ0101"),
        "coded as an unbound export"
    );
    assert!(
        d.message.contains("`mian`") && d.message.contains("did you mean `main`?"),
        "names the bad export + suggests the nearest def: {}",
        d.message
    );

    // The FULL `compile` pipeline agrees now (a check≡compile fix): layout declines a missing export
    // with an UNCODED "names no definition", and `compile` used to short-circuit on that BEFORE
    // `collect_faults` ran — so `cdz compile` showed the fix-less message while `cdz check` showed the
    // coded CDZ0101 + suggestion. `compile` now runs `collect_faults` on a layout decline and reports
    // its richer coded set, so BOTH surfaces carry the code, the "did you mean?", and the replace fix.
    let compiled = reject_full("(module m (def (main) 1) (export mian))")
        .expect("compile reports the missing export");
    assert_eq!(
        compiled.code.as_deref(),
        Some("CDZ0101"),
        "compile agrees with check — coded, not the uncoded layout decline: {}",
        compiled.message
    );
    assert!(
        compiled.message.contains("did you mean `main`?"),
        "compile carries the suggestion too: {}",
        compiled.message
    );
    assert_eq!(
        compiled.fix.as_ref().map(|f| f.kind),
        Some(crate::abi::FixKind::Replace),
        "compile carries the replace fix: {:?}",
        compiled.fix
    );
    assert_eq!(
        compiled.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("main"),
        "the fix rewrites the misspelled export to the nearest def: {:?}",
        compiled.fix
    );
    // ROUND TRIP: applying the fix (rewrite the export `mian` → the nearest def `main`) recompiles
    // clean — the corrected export names a real definition, so the CDZ0101 is gone. The "did you mean"
    // export fix's promised repair actually lands.
    assert!(
        reject_full("(module m (def (main) 1) (export main))").is_none(),
        "applying the export-typo fix must recompile clean"
    );
    // A layout decline with NO `collect_faults` fault still falls back to the layout message — a
    // program with no export at all is not a `collect_faults` fault, so its decline is preserved.
    let no_export =
        reject_full("(module m (def (main) 1))").expect("a program with no export declines");
    assert!(
        no_export.message.contains("nothing is public"),
        "the no-export layout decline is preserved (no collect fault to prefer): {}",
        no_export.message
    );
}

/// A WELL-FORMED `(pragma default-integer|default-fraction|default-float <T>)` written at the PROGRAM'S
/// TOP LEVEL — the root module's own directive, or a bare `(do …)` item — now TAKES EFFECT: `Db::load`
/// harvests it over the root scope's `(def …)` literals, exactly as a nested `(module NAME …)` pragma is
/// harvested for its members (numeric-model.md §A Module May Declare Its Default … Literal Type — a file
/// IS a module, no do-nesting requirement). So it is NO LONGER mis-scoped: no "has effect only inside a
/// nested module" placement fault, and NO misleading "unbound name `pragma`". A MALFORMED top-level
/// pragma still rejects via the registry pass (unknown key CDZ0601 / arity CDZ0602 / domain CDZ0303).
#[test]
fn a_top_level_default_pragma_takes_effect_not_a_placement_fault() {
    use crate::testkit::parse;
    // A well-formed top-level default pragma → NO placement fault, NO unbound-`pragma`, and (with a
    // well-formed body) NO error at all — it is honored.
    for src in [
        "(module m (pragma default-integer Int32) (def (main) 1) (export main))",
        "(do (pragma default-integer BigInt) (def (main) 1) (export main))",
        "(do (pragma default-fraction Rational) (def (main) (/ 1 2)) (export main))",
    ] {
        let all = crate::diagnostics(&mut crate::db::Db::load(parse(src)));
        assert!(
            !all.iter()
                .any(|d| d.message.contains("has effect only inside a nested")),
            "a well-formed top-level pragma is no longer mis-scoped: {src} -> {:?}",
            all.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert!(
            !all.iter()
                .any(|d| d.message.contains("unbound name `pragma`")),
            "no misleading unbound-pragma: {src} -> {:?}",
            all.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert!(
            all.iter()
                .all(|d| d.severity != crate::abi::Severity::Error),
            "a well-formed top-level default pragma program has no error: {src} -> {:?}",
            all.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
    // A MALFORMED top-level pragma keeps the MORE-SPECIFIC registry message:
    // unknown key → names the key; wrong arity → CDZ0602; non-integer type → CDZ0303.
    let unknown = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (pragma nonesuch 5) (def (main) 1) (export main))",
    )));
    assert!(
        unknown
            .iter()
            .any(|d| d.message.contains("`nonesuch` is not a module directive")),
        "an unknown top-level pragma key keeps the registry message: {:?}",
        unknown.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert_eq!(
        reject_code("(module m (pragma default-integer) (def (main) 1) (export main))").as_deref(),
        Some("CDZ0602"),
        "a wrong-arity top-level pragma keeps CDZ0602"
    );
    assert_eq!(
        reject_code("(module m (pragma default-integer String) (def (main) 1) (export main))")
            .as_deref(),
        Some("CDZ0303"),
        "a non-integer top-level default keeps CDZ0303"
    );
}

// (contract_input_output_pragmas_are_removed_and_now_reject_as_unknown migrated to corpus 11-modules,
// after the unknown/malformed pragma cases: the removed `contract`/`input`/`output` module directives
// are each now an unknown-directive reject → CDZ0601 (message "`<key>` is not a module directive").
// --case grades the code + message (all 3 PASS). Pins the #4542 removal; a re-add flips these cases.)

#[test]
fn a_default_integer_pragma_makes_a_bare_literal_take_the_declared_type() {
    // `numeric-model.md` §A Module May Declare Its Default Integer Literal Type: a bare, otherwise-
    // unconstrained integer literal WRITTEN in a `(pragma default-integer <T>)` module takes `<T>`
    // instead of Int64. Realized by the load-time `default_int_literals` map (keyed by the ORIGINAL
    // literal node, so it survives the β-copy that reparents an inlined body).
    //
    // (1) THE EFFECT: `double`'s bare `2` is a BigInt, so `(* x 2)` with `x : BigInt` is a homogeneous
    //     BigInt op and `(double (BigInt.of 21))` = 42 : BigInt — clean, no CDZ0301 mix.
    assert_eq!(
        reject_code(
            "(module top (def (main) (do (module crypto (pragma default-integer BigInt) \
                   (def (double x) (* x 2))) ((. crypto double) ((. BigInt of) 21)))) (export main))"
        ),
        None,
        "a bare literal in a default-integer=BigInt module is a BigInt, so (* x 2) is homogeneous"
    );
    // (2) AN EXPLICIT ANNOTATION STILL WINS: `(: 5 Int64)` in the same module is Int64, not BigInt —
    //     the default only decides the otherwise-unconstrained case (the `Annot` node fixes its type).
    assert_eq!(
        reject_code(
            "(module top (def (main) (do (module m (pragma default-integer BigInt) \
                   (def (pinned) (: 5 Int64))) ((. m pinned) unit))) (export main))"
        ),
        None,
        "an explicit annotation overrides the module default without a mismatch"
    );
    // (3) A literal OUTSIDE any pragma module is unaffected — still the Int64 default (no map entry).
    assert_eq!(
        reject_code("(module m (def (main) (+ 2 3)) (export main))"),
        None,
        "a literal outside a default-integer module keeps the ordinary Int64 default"
    );
}

#[test]
fn overflow_mode_of_resolves_the_single_mode_by_operand_signedness() {
    // STAGE 2a (ruling B): `infer::overflow_mode_of` resolves ONE mode per unqualified `+`/`-`/`*` node
    // — the module pragma's mode selected by the operand's concrete SIGNEDNESS, else the global manifest
    // default, else `Trap`.
    use crate::db::OverflowMode;
    // (1) SIGNED operand under `(signed wrap)(unsigned trap)` → Wrap (the signed slot).
    let mut db = crate::db::Db::load(parse(
        "(module m (pragma overflow (signed wrap) (unsigned trap)) \
               (def (f (: x Int64)) (+ x 1)) (export f))",
    ));
    let signed_ops: Vec<_> = db.overflow_specs.keys().copied().collect();
    assert!(!signed_ops.is_empty(), "the `(+ x 1)` node is marked");
    for n in signed_ops {
        assert_eq!(
            crate::infer::overflow_mode_of(&mut db, n),
            OverflowMode::Wrap,
            "a signed op under (signed wrap) resolves to Wrap"
        );
    }
    // (2) UNSIGNED operand under the SAME pragma → Trap (the unsigned slot — signedness selects).
    let mut db_u = crate::db::Db::load(parse(
        "(module m (pragma overflow (signed wrap) (unsigned trap)) \
               (def (f (: x UInt64)) (+ x 1)) (export f))",
    ));
    let unsigned_ops: Vec<_> = db_u.overflow_specs.keys().copied().collect();
    assert!(!unsigned_ops.is_empty());
    for n in unsigned_ops {
        assert_eq!(
            crate::infer::overflow_mode_of(&mut db_u, n),
            OverflowMode::Trap,
            "an UNSIGNED op under (unsigned trap) resolves to Trap — signedness selects the slot"
        );
    }
    // (3) NO pragma → no module spec, global default unset → the built-in `Trap`. (Any arith node
    //     resolves Trap; construct one via a scratch and read its type-of node through the same path.)
    let mut db_none =
        crate::db::Db::load(parse("(module m (def (f (: x Int64)) (+ x 1)) (export f))"));
    assert!(
        db_none.overflow_specs.is_empty(),
        "no pragma → no marked nodes"
    );
    // A node absent from `overflow_specs` resolves to the global/Trap level; pick the `f` def body's
    // occurrence range and assert the default holds for a representative node (the export def's body).
    let f = db_none.def_by_name("f").expect("def f");
    let body = db_none.defs[f].body.expect("f body");
    assert_eq!(
        crate::infer::overflow_mode_of(&mut db_none, body),
        OverflowMode::Trap,
        "with no pragma and no manifest default, overflow resolves to the built-in Trap"
    );
}

#[test]
fn an_overflow_pragma_marks_each_unqualified_arith_node_with_its_policy() {
    // `numeric-model.md` §A Module May Declare Its Overflow Policy: a `(pragma overflow (signed <mode>)
    // (unsigned <mode>))` module governs the trap/wrap behavior of each unqualified `+`/`-`/`*` WRITTEN
    // in it. STAGE 1 (this test): the load-time `overflow_specs` map records each such operator node →
    // the declared spec (the infer-time signed/unsigned SELECTION is stage 2). Keyed by the original
    // node, DEFINITION-SITE scoped, and a named `Int64.wrapping-*` form is IMMUNE (not an entry).
    use crate::db::{OverflowMode, OverflowSpec};
    let want = OverflowSpec {
        signed: Some(OverflowMode::Wrap),
        unsigned: Some(OverflowMode::Trap),
    };
    // (1) THE EFFECT: `(+ (* x 2) 1)` in an overflow module contributes TWO governed ops (`+` and `*`),
    //     each mapped to the declared spec.
    let db = crate::db::Db::load(parse(
        "(module top (def (main) (do (module m (pragma overflow (signed wrap) (unsigned trap)) \
               (def (f x) (+ (* x 2) 1))) unit)) (export main))",
    ));
    assert!(
        !db.overflow_specs.is_empty(),
        "an overflow-pragma module marks its arithmetic nodes"
    );
    assert!(
        db.overflow_specs.values().all(|&s| s == want),
        "every marked node carries the declared (signed wrap)(unsigned trap) spec: {:?}",
        db.overflow_specs.values().collect::<Vec<_>>()
    );
    assert_eq!(
        db.overflow_specs.len(),
        2,
        "the `+` and the `*` are both marked (the bare `1`/`2` literals are not ops)"
    );
    // (2) IMMUNITY: a named `Int64.wrapping-add` form in the SAME module is not an unqualified `+`, so it
    //     contributes NO entry (it carries its own overflow contract).
    let db_named = crate::db::Db::load(parse(
        "(module top (def (main) (do (module m (pragma overflow (signed wrap) (unsigned trap)) \
               (def (f x) ((. Int64 wrapping-add) x 1))) unit)) (export main))",
    ));
    assert!(
        db_named.overflow_specs.is_empty(),
        "a named wrapping-* form is immune — not governed by the pragma: {:?}",
        db_named.overflow_specs.values().collect::<Vec<_>>()
    );
    // (3) DEFINITION-SITE SCOPE: an op OUTSIDE any overflow module has no entry.
    let db_none = crate::db::Db::load(parse("(module m (def (main) (+ 2 3)) (export main))"));
    assert!(
        db_none.overflow_specs.is_empty(),
        "an op outside an overflow-pragma module is unmarked"
    );
}

// [migrated → spec/semantics/11-modules.sexp] an_overflow_pragma_validates_its_shape_and_does_not_block_registration:
// the `overflow` pragma is a MODELED directive with a richer shape than the single-arg keys — each arg is a
// nested (signed|unsigned <mode>) with mode in {trap,wrap}. Corpus 11-modules faces: well-formed (both
// signednesses) accepted + module registers (runs 1); unknown mode (signed nonesuch) → CDZ0602; no sub-form
// (pragma overflow) → CDZ0602. The single-sub-form well-formed face is covered by the (pragma overflow
// (signed wrap)) behavior cases in 06-numeric-model.sexp. All PASS on wasm.

// [migrated → spec/semantics/06-numeric-model.sexp] a_default_fraction_pragma_grounds_a_bare_numeric_literal_to_rational:
// a bare numeric literal in a (pragma default-fraction Rational) module grounds to Rational. Corpus 06
// default-fraction section covers all faces: (/ 1 3) exact = 1/3 ("makes a bare literal exact"), a
// Rational-defaulted literal mixed with an explicit Int64 → CDZ0301 ("fixes a type but adds no
// conversion"), an explicit annotation overrides ("an explicit annotation overrides the default-fraction
// pragma"); + the no-pragma control (top-level (/ 1 3) = 0 : Int64, ordinary integer division). All PASS.

#[test]
fn a_default_fraction_pragma_takes_precedence_over_default_float() {
    // Both pragmas in one module: the EXACT-fraction default is the stronger statement (exact by
    // default), so a bare decimal grounds to `Rational` (`0.5` → `1/2`), NOT the `default-float` width.
    // `(/ 1 3)` is then exact rational division — clean, homogeneous (proving fraction won over float).
    assert_eq!(
        reject_code(
            "(module top (def (main) (do (module m (pragma default-fraction Rational) (pragma default-float Float32) (def (third) (/ 1 3))) ((. m third) unit))) (export main))"
        ),
        None,
        "default-fraction takes precedence over default-float — bare literals ground to Rational"
    );
}

// [migrated → spec/semantics/06-numeric-model.sexp] a_default_integer_pragma_runs_the_narrow_literal_fit_check:
// a (pragma default-integer <NarrowT>) grounds a bare literal to NarrowT, so the SAME literal-fit range
// check an explicit (: v NarrowT) runs applies (soundness: no out-of-range literal silently admitted).
// Corpus 06 default-integer pragma section: well-past-range Int8 300 → CDZ0302 (pre-existing); + added
// boundary/sign faces: Int8 128 (one past max) → CDZ0302, Int8 127 (max) → runs 127, UInt8 -1 (negative)
// → CDZ0302; the UInt8/300 magnitude twin + widening-BigInt-never-faults control stay covered by those
// cases and the earlier default-integer BigInt cases. All PASS on wasm.

#[test]
fn a_nullary_module_member_body_is_type_checked() {
    // `type-system.md` §A program that is not well-typed MUST be rejected. A NULLARY module member
    // `(def (bad) …)` is NOT registered in `db.defs` (`modules::register_fn_def` registers only ≥1-param
    // members, for recursive-call lowering), so its body was never type-checked — an ill-typed one
    // DECLINED (a BigInt/Int64 mix) or emitted an INVALID COMPONENT (a Float/Int mix — a real
    // miscompile). `collect_faults` now type-checks each value/nullary member body too.
    // A numeric MIX in a nullary member is CDZ0301 (no silent promotion), not a miscompile:
    for src in [
        "(module top (def (main) (do (module m (def (bad) (+ 1 2.0))) ((. m bad) unit))) (export main))",
        "(module top (def (main) (do (module m (def (bad) (+ (BigInt.of 2) ((. Int64 of) 1)))) ((. m bad) unit))) (export main))",
    ] {
        assert_eq!(
            reject_code(src).as_deref(),
            Some("CDZ0301"),
            "a numeric mix in a nullary module member rejects CDZ0301, not a miscompile: {src}"
        );
    }
    // A well-typed nullary member still compiles clean.
    assert_eq!(
        reject_code(
            "(module top (def (main) (do (module m (def (ok) (+ 1 2))) ((. m ok) unit))) (export main))"
        ),
        None,
        "a well-typed nullary member is not falsely rejected"
    );
    // KEYSTONE: REGRESSION GUARD: a nullary member body that references a SIBLING (an effect `log`, a sibling
    // def) resolves through the module's in-scope context — the standalone type-check must NOT report
    // that sibling as `Unbound` (CDZ0101). Such a member (performing a sibling effect via `host`)
    // compiles clean; the member-body check drops a standalone `Unbound` for exactly this reason.
    assert_eq!(
        reject_code(
            "(module top (def (main) (do (module m (effect log (op emit (-> String Unit))) \
                   (def (run) (host (log) ((. log emit) \"hi\")))) (= 1 1))) (export main))"
        ),
        None,
        "a nullary member performing a sibling effect is not falsely CDZ0101 by the standalone check"
    );
}

#[test]
fn a_module_in_a_top_level_do_type_checks_its_members() {
    // `type-system.md` §A program that is not well-typed MUST be rejected. A `(module …)` that is an
    // ELEMENT of a TOP-LEVEL `(do …)` sequence root was registered by NEITHER the top-level scan (which
    // has no `module` branch) NOR the nested-declaration walk (which SKIPS a top-level item as
    // already-scanned) — so its members escaped `collect_faults` entirely (a residual of the
    // nullary-member-body fix, which reached a bare `(module m …)` and a def-body-nested one but not
    // this position). `scan_top_level` now registers a top-level module item via `collect_module_decl`.
    // An ill-typed member is now rejected wherever the module sits.
    for src in [
        // Float/Int mix → CDZ0301 (was silently accepted / a latent invalid component).
        "(do (module m (def (bad) (+ 1 2.0))) (def (main) 5) (export main))",
        // Bool/Int mix → CDZ0203.
        "(do (module m (def (bad) (+ true 1))) (def (main) 5) (export main))",
    ] {
        assert!(
            reject_code(src).is_some(),
            "an ill-typed member of a top-level-do module must be rejected (was a type-check hole): {src}"
        );
    }
    assert_eq!(
        reject_code("(do (module m (def (bad) (+ 1 2.0))) (def (main) 5) (export main))")
            .as_deref(),
        Some("CDZ0301"),
        "the Float/Int mix is the numeric no-promotion CDZ0301, matching the bare-module path"
    );
    // A WELL-TYPED top-do module still compiles clean (the position is accepted; only the missing
    // type-check was the bug).
    assert_eq!(
        reject_code("(do (module m (def (good) (+ 1 2))) (def (main) 5) (export main))"),
        None,
        "a well-typed top-do module member is not falsely rejected"
    );
}

#[test]
fn an_if_with_ml_then_else_keywords_names_the_syntax_not_the_arity() {
    // A user who knows the ML surface writes `(if b then 1 else 0)`, reaching for the `then`/`else`
    // KEYWORDS — but this s-expr surface's `if` is three POSITIONAL forms with no keywords. The stray
    // `then`/`else` land as bare operands (5-operand `if`), which would otherwise hit the generic
    // "if takes exactly 3 operands" arity reject that names the count, not the real mistake. It now
    // names the ML-keyword confusion + the correct shape. Flagged by v-compiler-ml (a reader trap that
    // cost them several ticks). The unbound `then`/`else` symbols must NOT surface as the primary fault.
    let ds = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f (: b Bool)) (if b then 1 else 0)) (export f))",
    )));
    let d = ds
        .iter()
        .find(|d| d.code.as_deref() == Some("CDZ0201") && d.message.contains("`then`/`else`"))
        .expect("an ML-keyword if names the syntax confusion");
    assert!(
        d.message.contains("positional") && d.message.contains("(if <cond> <then> <else>)"),
        "names the positional s-expr shape: {}",
        d.message
    );
    // The confusing "unbound name `then`" / `else` must NOT be the surfaced fault (the syntax hint
    // supersedes it — the stray keywords are a consequence of the syntax mistake, not independent).
    assert!(
        !ds.iter().any(|d| d.message.contains("unbound name `then`")
            || d.message.contains("unbound name `else`")),
        "the unbound then/else symbols are not surfaced as the primary fault: {:?}",
        ds.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // The corrected positional form compiles.
    assert!(
        crate::compile::compile_component(&crate::codec::encode(&parse(
            "(module m (def (f (: b Bool)) (if b 1 0)) (export f))"
        )))
        .is_ok(),
        "the corrected positional if compiles"
    );
    // NO false positive: an ordinary too-many-operand `if` (not the then/else signature) keeps the
    // generic arity message + its surplus-delete fix.
    let surplus = crate::diagnostics(&mut crate::db::Db::load(parse(
        "(module m (def (f (: b Bool)) (if b 1 0 9)) (export f))",
    )))
    .into_iter()
    .find(|d| d.code.as_deref() == Some("CDZ0201"))
    .expect("a 4-operand if rejects");
    assert!(
        surplus.message.contains("takes exactly 3 operands"),
        "an ordinary surplus if keeps the generic arity message: {}",
        surplus.message
    );
}

#[test]
fn an_empty_let_binding_list_names_the_binds_nothing_case_not_a_malformed_binding() {
    // `(let () <body>)` — an EMPTY binding list — is distinct from a MALFORMED one `(let ((a 1 2)) …)`:
    // there is no binding to be "malformed", the `let` just binds nothing. The message now says so
    // ("binds nothing — an empty `()` binding list has no effect; write the body directly") instead of
    // the misleading "each must be `(<name> <init>)`" (which implies a broken binding that isn't there).
    let msg = |src: &str| -> String {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.code.as_deref() == Some("CDZ0201"))
            .unwrap_or_else(|| panic!("expected CDZ0201 for {src}"))
            .message
    };
    let empty = msg("(module m (def (f) (let () 5)) (export f))");
    assert!(
        empty.contains("binds nothing") && empty.contains("empty `()`"),
        "an empty binding list names the binds-nothing case: {empty}"
    );
    // A GENUINELY MALFORMED binding list keeps the "malformed" message (a broken `(<name> <init>)`).
    for malformed in [
        "(module m (def (f) (let ((a 1 2)) a)) (export f))", // 3-element binding
        "(module m (def (f) (let (a) a)) (export f))",       // a bare-name non-pair binding
    ] {
        let m = msg(malformed);
        assert!(
            m.contains("bindings are malformed") && !m.contains("binds nothing"),
            "a malformed binding keeps the malformed message: {malformed} -> {m}"
        );
    }
    // NO regression: the degenerate `(let)` (no bindings AND no body) keeps its own message; a valid
    // one-binding let compiles clean.
    assert!(
        msg("(module m (def (f) (let)) (export f))").contains("no bindings and no body"),
        "the degenerate (let) keeps its no-bindings-and-no-body message"
    );
    assert!(
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (f) (let ((a 1)) a)) (export f))"
        )))
        .iter()
        .all(|d| d.severity != crate::abi::Severity::Error),
        "a valid one-binding let is clean"
    );
}

#[test]
fn record_without_and_merge_reshape_records_with_field_set_checks() {
    // 15-rows "dropping fields from a record leaves the remaining fields" / "...an absent field is
    // rejected" / "merging two records with disjoint fields unions their fields" / "merging records
    // that share a field name is rejected". `Record.without r (b)` drops the named fields (complement
    // of `project`); `Record.merge a b` unions two records' field sets, requiring DISJOINTNESS.
    // `without` of a PRESENT field is well-formed.
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.without (record (= a 1) (= b 2) (= c 3)) (b))) (export main))"
        ),
        None,
        "dropping a present field is well-formed"
    );
    // `without` of an ABSENT field → CDZ0212 (a drop of a field never held is a static error).
    assert_eq!(
        reject_code("(module m (def (main) (Record.without (record (= a 1)) (z))) (export main))")
            .as_deref(),
        Some("CDZ0212"),
        "dropping an absent field is CDZ0212"
    );
    // `merge` of DISJOINT field sets is well-formed.
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.merge (record (= a 1)) (record (= b 2)))) (export main))"
        ),
        None,
        "merging disjoint records is well-formed"
    );
    // `merge` of records SHARING a field → CDZ0211 (no silent clobber — the combined record cannot
    // choose which operand's value the shared field takes).
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.merge (record (= a 1)) (record (= a 2)))) (export main))"
        )
        .as_deref(),
        Some("CDZ0211"),
        "merging records that share a field is CDZ0211"
    );
}

#[test]
fn a_record_row_op_over_a_non_record_names_the_kind() {
    // A `Record.project`/`without`/`merge`/`extend`/`with` over a DEFINITE non-record operand
    // (`(Record.project n (x))` for `n : Int64`) is a kind error, like member access on a non-record.
    // It was check-INVISIBLE (the field checks only fire for a `Ty::Record` operand) and compiled to a
    // MISLEADING "record row operation over a runtime record is not yet built". Now CDZ0201 names the
    // op + the non-record type: "`Record.project` requires a record, found Int64".
    let msg = |src: &str| -> String {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.message.contains("requires a record"))
            .unwrap_or_else(|| panic!("no non-record-operand fault for {src}"))
            .message
    };
    for (op, arg2) in [
        ("project", "(x)"),
        ("without", "(x)"),
        ("merge", "(record (= y 1))"),
        ("extend", "(x 5)"),
        ("with", "(x 5)"),
    ] {
        let src = format!("(module m (def (g (: n Int64)) (Record.{op} n {arg2})) (export g))");
        let m = msg(&src);
        assert!(
            m.contains(&format!("`Record.{op}` requires a record")) && m.contains("Int64"),
            "{op}: {m}"
        );
    }
    // NO false positive: a real record operand, and a bare (unconstrained `Any`) parameter, are clean.
    for ok in [
        "(module m (def (g (: r (Record (x Int64)))) (Record.project r (x))) (export g))",
        "(module m (def (get-x r) (Record.project r (x))) (def (main) 1) (export main))",
    ] {
        assert!(
            !crate::diagnostics(&mut crate::db::Db::load(parse(ok)))
                .iter()
                .any(|d| d.message.contains("requires a record")),
            "a record / unconstrained operand is not flagged: {ok}"
        );
    }
}

#[test]
fn a_tuple_row_op_over_a_non_tuple_names_the_kind() {
    // The tuple twin of the record-row-op kind check: `Tuple.concat`/`split-at`/`pop` over a definite
    // non-tuple operand (`(Tuple.remove n)` for `n : Int64`) was check-invisible and compiled to a
    // misleading "Tuple.<op> over a runtime tuple is not yet built" (the operand is not a tuple at
    // all). Now CDZ0201 names the op + the non-tuple type.
    let msg = |src: &str| -> String {
        crate::diagnostics(&mut crate::db::Db::load(parse(src)))
            .into_iter()
            .find(|d| d.message.contains("requires a tuple"))
            .unwrap_or_else(|| panic!("no non-tuple-operand fault for {src}"))
            .message
    };
    for (op, rest) in [("concat", "n"), ("remove", ""), ("split-at", "1")] {
        let src = format!("(module m (def (g (: n Int64)) (Tuple.{op} n {rest})) (export g))");
        let m = msg(&src);
        assert!(
            m.contains(&format!("`Tuple.{op}` requires a tuple")) && m.contains("Int64"),
            "{op}: {m}"
        );
    }
    // NO false positive: a real tuple operand, and a bare (unconstrained `Any`) parameter, are clean.
    for ok in [
        "(module m (def (g (: t (Tuple Int64 Int64))) (Tuple.remove t)) (export g))",
        "(module m (def (f t) (Tuple.remove t)) (def (main) 1) (export main))",
    ] {
        assert!(
            !crate::diagnostics(&mut crate::db::Db::load(parse(ok)))
                .iter()
                .any(|d| d.message.contains("requires a tuple")),
            "a tuple / unconstrained operand is not flagged: {ok}"
        );
    }
}

#[test]
fn record_extend_with_pop_are_derived_row_ops_with_presence_checks() {
    // 15-rows extend/with/pop — the DERIVED row ops (rewrites of merge/without). `Record.extend r
    // #z v` ADDS an absent field (present → CDZ0211); `Record.with r #z v` REPLACES a present field
    // (absent → CDZ0212), retyping to the new value's type; `Record.pop r z` yields `(value,
    // remaining-record)` (absent → CDZ0212). extend/with take a `#z` field LABEL and a value operand
    // (DESIGN-record-update-syntax.md, 3-operand); pop takes a bare name.
    // extend adds an ABSENT field (well-formed); a PRESENT field is CDZ0211.
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.extend (record (= a 1)) #\"b\" 2)) (export main))"
        ),
        None,
        "extend of an absent field is well-formed"
    );
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.extend (record (= a 1)) #\"a\" 2)) (export main))"
        )
        .as_deref(),
        Some("CDZ0211"),
        "extend of a present field is CDZ0211 (use with)"
    );
    // with replaces a PRESENT field (well-formed, may retype); an ABSENT field is CDZ0212.
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.with (record (= a 1) (= b 2)) #\"b\" true)) (export main))"
        ),
        None,
        "with of a present field (even retyping) is well-formed"
    );
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.with (record (= a 1)) #\"z\" 5)) (export main))"
        )
        .as_deref(),
        Some("CDZ0212"),
        "with of an absent field is CDZ0212 (use extend)"
    );
    // pop of a PRESENT field is well-formed; an ABSENT field is CDZ0212.
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.pop (record (= a 1) (= b 2)) a)) (export main))"
        ),
        None,
        "pop of a present field is well-formed"
    );
    assert_eq!(
        reject_code("(module m (def (main) (Record.pop (record (= a 1)) z)) (export main))")
            .as_deref(),
        Some("CDZ0212"),
        "pop of an absent field is CDZ0212"
    );
    // A mistyped `pop`/`with` field near a real one carries a did-you-mean — the closed-set
    // suggestion `without`/`project` already give, over the operand record's fields.
    let dp = reject_full(
        "(module m (def (main) (Record.pop (record (= alpha 1) (= beta 2)) alpa)) (export main))",
    )
    .expect("pop of an absent field is CDZ0212");
    assert!(
        dp.message.contains("did you mean `alpha`?"),
        "a mistyped popped field suggests the near one; got {}",
        dp.message
    );
    let dw = reject_full(
            "(module m (def (main) (Record.with (record (= alpha 1) (= beta 2)) #\"alpa\" 9)) (export main))",
        )
        .expect("with of an absent field is CDZ0212");
    assert!(
        dw.message.contains("did you mean `alpha`?") && dw.message.contains("use `Record.extend`"),
        "a mistyped `with` field suggests the near one AND keeps the extend hint; got {}",
        dw.message
    );
    // Both near-misses also carry an APPLICABLE replace fix on the field occurrence (`alpa`→`alpha`),
    // the same closed-set fix `without`/`project` labels get (M63/M64) — not just the message hint.
    assert_eq!(
        dp.fix.as_ref().map(|f| (f.kind, f.replacement.as_str())),
        Some((crate::abi::FixKind::Replace, "alpha")),
        "pop near-miss carries a replace fix: {:?}",
        dp.fix
    );
    assert_eq!(
        dw.fix.as_ref().map(|f| (f.kind, f.replacement.as_str())),
        Some((crate::abi::FixKind::Replace, "alpha")),
        "with near-miss carries a replace fix: {:?}",
        dw.fix
    );
    // NO OVERREACH: a far-miss field keeps the message but carries no fix.
    let far = reject_full(
        "(module m (def (main) (Record.pop (record (= alpha 1)) zzzzzz)) (export main))",
    )
    .expect("pop of an absent field is CDZ0212");
    assert!(
        far.fix.is_none(),
        "no fix without a plausible near field: {:?}",
        far.fix
    );
}

#[test]
fn a_wrong_record_row_op_carries_the_operator_swap_fix_the_message_names() {
    // `Record.extend`/`Record.with` have complementary presence preconditions (extend REQUIRES the
    // field absent, with REQUIRES it present), and each rejection already NAMES the sibling op to use
    // ("use `Record.with` to replace" / "use `Record.extend` to add"). Now that named fix is APPLYABLE:
    // a one-token operator swap on the operation head, marked VERIFIED (the swap's precondition is
    // exactly the fault's condition — it clears the fault and preserves types by construction, no
    // guess). The row-op analogue of the numeric `float_sibling_operator` swap.

    // extend a PRESENT field → CDZ0211 + a VERIFIED swap `extend`→`with`.
    let de = reject_full(
        "(module m (def (main) (Record.extend (record (= a 1)) #\"a\" 2)) (export main))",
    )
    .expect("extend of a present field is CDZ0211");
    assert_eq!(de.code.as_deref(), Some("CDZ0211"), "got: {}", de.message);
    let fe = de.fix.as_ref().expect("carries the operator-swap fix");
    assert_eq!(fe.kind, crate::abi::FixKind::Replace);
    assert_eq!(
        fe.replacement, "with",
        "swaps the op to `with`: {}",
        de.message
    );
    assert!(
        fe.verified,
        "the presence precondition makes the swap verified"
    );
    // ROUND TRIP: applying the swap (`extend`→`with` on a PRESENT field) recompiles clean — updating
    // an existing field is exactly `with`'s precondition, so the applied form type-checks.
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.with (record (= a 1)) #\"a\" 2)) (export main))"
        ),
        None,
        "applying the verified `extend`→`with` swap must recompile clean"
    );

    // with an ABSENT field that is NOT a near typo → CDZ0212 + a VERIFIED swap `with`→`extend`.
    let dw = reject_full(
        "(module m (def (main) (Record.with (record (= alpha 1)) #\"zzzzz\" 5)) (export main))",
    )
    .expect("with of an absent field is CDZ0212");
    assert_eq!(dw.code.as_deref(), Some("CDZ0212"), "got: {}", dw.message);
    let fw = dw.fix.as_ref().expect("carries the operator-swap fix");
    assert_eq!(
        fw.replacement, "extend",
        "swaps the op to `extend`: {}",
        dw.message
    );
    assert!(
        fw.verified,
        "the absence precondition makes the swap verified"
    );
    // ROUND TRIP: applying the swap (`with`→`extend` on an ABSENT field) recompiles clean — adding a
    // new field is exactly `extend`'s precondition, so the applied form type-checks.
    assert_eq!(
        reject_code(
            "(module m (def (main) (Record.extend (record (= alpha 1)) #\"zzzzz\" 5)) (export main))"
        ),
        None,
        "applying the verified `with`→`extend` swap must recompile clean"
    );

    // A NEAR-miss field still prefers the label typo-fix (the likelier intent), NOT the op swap.
    let dn = reject_full(
        "(module m (def (main) (Record.with (record (= alpha 1)) #\"alpXa\" 5)) (export main))",
    )
    .expect("with of a near-miss field is CDZ0212");
    let fn_ = dn
        .fix
        .as_ref()
        .expect("a near typo carries the label rewrite");
    assert!(
        fn_.replacement.contains("alpha") && fn_.replacement != "extend",
        "a near typo rewrites the label, not the op: {}",
        fn_.replacement
    );
}

#[test]
fn tuple_cat_split_at_pop_reshape_tuples_positionally() {
    // 15-rows tuple reshaping — the POSITIONAL analogue of the record row ops. `Tuple.concat a b`
    // concatenates (arity = sum, each element keeps its position's type); `Tuple.split-at t k` splits
    // at compile-time `k` into a `(prefix suffix)` pair (k=0 → prefix is unit, k out of 0..=arity →
    // CDZ0201); `Tuple.remove t` takes element 0 off. cat/split-at/pop all compile over constant tuples.
    for src in [
        "(module m (def (main) (Tuple.concat (tuple 1 2) (tuple 3 4))) (export main))",
        "(module m (def (main) (Tuple.split-at (tuple 1 2 3) 1)) (export main))",
        "(module m (def (main) (Tuple.split-at (tuple 1 2) 0)) (export main))",
        "(module m (def (main) (Tuple.remove (tuple 1 2 3))) (export main))",
    ] {
        assert_eq!(
            reject_code(src),
            None,
            "a well-formed tuple reshaping must compile: {src}"
        );
    }
    // A split position OUTSIDE the operand's static arity `0..=len` is CDZ0201 (the `(. x N)`
    // static-bounds rule) — `(tuple 1 2)` has arity 2, so a split at 5 names a position it lacks.
    assert_eq!(
        reject_code("(module m (def (main) (Tuple.split-at (tuple 1 2) 5)) (export main))")
            .as_deref(),
        Some("CDZ0201"),
        "a split beyond the tuple's arity is CDZ0201"
    );
    // `Tuple` is STILL the tuple-TYPE constructor in type position — the dual-shape module did not
    // break `(: t (Tuple Int64 Bool))`.
    assert_eq!(
        reject_code("(module m (def (main) (: (tuple 1 true) (Tuple Int64 Bool))) (export main))"),
        None,
        "Tuple must still work as the tuple-type constructor in an annotation"
    );
}

#[test]
fn a_mixed_list_anchors_at_the_outlier_element_not_the_whole_list() {
    // The homogeneity reject anchors at the OUTLIER element (the one that broke homogeneity against the
    // established first-element type), not the enclosing `(list …)` — so the editor squiggle lands on
    // exactly the off element in a long list, not the whole literal. `"three"` is the String among
    // Int64s; its reported node must be that STRING ATOM, not the list.
    let src = "(module m (def (main) ((. List len) (list 1 2 \"three\" 4 5))) (export main))";
    let d = reject_full(src).expect("must reject");
    assert_eq!(d.code.as_deref(), Some("CDZ0201"), "got: {}", d.message);
    let node = d.node.expect("the reject carries an anchor node");
    let db = crate::db::Db::load(parse(src));
    // The anchor is the string literal `"three"`, not the `(list …)` form.
    assert_eq!(
        db.ast.as_str(crate::ast::StructId(node)),
        Some("three"),
        "the mismatch anchors at the outlier element `\"three\"`, not the whole list"
    );
}

#[test]
fn a_list_op_homogeneity_reject_anchors_at_the_offending_argument() {
    // The `List.push`/`List.update`/`List.concat` homogeneity reject (CDZ0201) must anchor at the
    // OFFENDING ARGUMENT — the pushed/updated element, or the second list in concat — not the whole
    // `(List.push …)` application node, so the squiggle points at the culprit (PR #399 review). Without
    // the anchor, `collect`'s `set_origin_if_absent(id)` stamped the whole call.
    // push: the pushed element `"x"` (a String into a List Int64) is the locus, not the call.
    let push =
        reject_full("(module m (def (f (: xs (List Int64))) ((. List push) xs \"x\")) (export f))")
            .expect("a wrong-element List.push rejects");
    assert_eq!(
        push.code.as_deref(),
        Some("CDZ0201"),
        "got: {}",
        push.message
    );
    let src_push = "(module m (def (f (: xs (List Int64))) ((. List push) xs \"x\")) (export f))";
    let db_push = crate::db::Db::load(parse(src_push));
    assert_eq!(
        db_push.ast.as_str(crate::ast::StructId(
            push.node.expect("push reject carries an anchor")
        )),
        Some("x"),
        "the List.push homogeneity reject anchors at the pushed element `\"x\"`, not the call: {}",
        push.message
    );
    // update: the updated element (arg 3) is the locus.
    let update = reject_full(
        "(module m (def (f (: xs (List Int64))) ((. List update) xs 0 \"y\")) (export f))",
    )
    .expect("a wrong-element List.update rejects");
    let src_up = "(module m (def (f (: xs (List Int64))) ((. List update) xs 0 \"y\")) (export f))";
    let db_up = crate::db::Db::load(parse(src_up));
    assert_eq!(
        db_up.ast.as_str(crate::ast::StructId(
            update.node.expect("update reject carries an anchor")
        )),
        Some("y"),
        "the List.update homogeneity reject anchors at the updated element `\"y\"`: {}",
        update.message
    );
}

#[test]
fn a_map_or_set_heterogeneity_structural_delta_hint_names_the_differing_subpart() {
    // STRUCTURAL-DELTA HINT (the peer-join hint the list/if/match sites carry, at set/map too): when
    // the two clashing element types are SAME-KIND compounds, the diagnostic names the SPECIFIC
    // differing sub-part instead of leaving two full renders. (The scalar names-types + int/float
    // retype-fix half of this family is now the corpus 05 "map/set twins of the list-homogeneity
    // message+fix" cases; this rust pin keeps the WHITE-BOX residual: the structural-delta wording and
    // the NEGATIVE that a scalar clash gets NO delta tail — a message-ABSENCE the corpus cannot assert.)
    // A set of records differing in one field TYPE.
    let sr = reject_full(
        "(module m (def x (Set.of (list (record (= x 1)) (record (= x true))))) (export x))",
    )
    .expect("reject");
    assert!(
        sr.message
            .contains("field `x` should be Int64, but this one is Bool"),
        "set names the differing record field: {}",
        sr.message
    );
    // A map KEY delta (records differing in a field) and a map VALUE delta (sum payload axis).
    let mkd = reject_full(
        "(module m (def x (map (= (record (= x 1)) 0) (= (record (= x true)) 1))) (export x))",
    )
    .expect("reject");
    assert!(
        mkd.message
            .contains("field `x` should be Int64, but this one is Bool"),
        "map key names the differing record field: {}",
        mkd.message
    );
    let mvd = reject_full("(module m (def x (map (= 0 (Some 5)) (= 1 (Some 2.0)))) (export x))")
        .expect("reject");
    assert!(
        mvd.message
            .contains("its payload should be Int64, but this one is Float64"),
        "map value names the differing sum payload axis: {}",
        mvd.message
    );
    // NO regression: a plain SCALAR clash gets NO structural-delta tail (only the type names) — a
    // message-ABSENCE the corpus cannot express, so it stays here.
    let sf = reject_full("(module m (def x (Set.of (list 1 2.0))) (export x))").expect("reject");
    assert!(
        !sf.message.contains("should be") && !sf.message.contains("field"),
        "a scalar set clash gets no structural-delta tail: {}",
        sf.message
    );
}

#[test]
fn a_set_of_and_member_op_arg_reject_anchor_at_the_culprit() {
    // The `Set.of` homogeneity reject (CDZ0201) and the prelude MEMBER-OP wrong-arg-type reject
    // (CDZ0203, `Module.op expects an argument of type …`) must anchor at the CULPRIT — the outlier
    // set element, or the wrong-typed argument — not the whole `(Set.of …)` / `(Module.op …)` node, so
    // the squiggle points at the minimal locus (the PR #399 anchoring family; behavior unchanged).
    let anchor_of = |src: &str| -> Option<String> {
        let d = reject_full(src)?;
        let db = crate::db::Db::load(parse(src));
        d.node
            .and_then(|n| db.ast.as_str(crate::ast::StructId(n)).map(str::to_string))
    };
    // Set.of — the outlier element `"z"` (a String among Int64s) is the locus, not the `(Set.of …)`.
    assert_eq!(
        anchor_of("(module m (def x (Set.of (list 1 2 \"z\"))) (export x))").as_deref(),
        Some("z"),
        "the Set.of homogeneity reject anchors at the outlier element `\"z\"`, not the call"
    );
    // member-op — the wrong-typed argument `"a"` (a String where List.at wants an Int64 index) is the
    // locus, not the `(List.at …)` application node.
    assert_eq!(
        anchor_of("(module m (def (f (: xs (List Int64))) ((. List at) xs \"a\")) (export f))")
            .as_deref(),
        Some("a"),
        "the member-op wrong-arg-type reject anchors at the argument `\"a\"`, not the call"
    );
    // NO REGRESSION on the deferring decline: `Symbol.of 5` (a non-string to a runtime-string op) still
    // reports exactly ONE error — the CDZ0203 (now anchored at the arg), not also the "operand is not a
    // string" lowering decline (which the flag-based dedup drops, since the two no longer share a node).
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(
                "(module m (def (main) ((. Symbol of) 5)) (export main))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    );
    let errors: Vec<&crate::abi::Diagnostic> = out
        .diagnostics
        .iter()
        .filter(|d| d.severity == crate::abi::Severity::Error)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "Symbol.of on a non-string = ONE error even with the arg-anchored CDZ0203: {:?}",
        out.diagnostics
    );
}

#[test]
fn a_mistyped_conversion_reports_one_error_not_a_shadowed_lowering_decline() {
    // A checked/wrapping conversion on a WRONG-TYPED operand — `(Int8.wrap 3.5)`, `(UInt8.wrap "hi")`
    // — is rejected by check_application with the coded CDZ0203 (`Int8.wrap expects an argument of type
    // Int64, but a value of type Float64 was given`). The conversion's LOWERING then ALSO declined "a
    // conversion of a non-scalar operand has no meaning" — the SAME wrong-operand defect surfacing at
    // emit (anchored at the op node, so the node-keyed dedup missed it). `dedup_faults` now drops that
    // decline when the conversion arg-type CDZ0203 is present, so a mis-typed conversion is ONE primary
    // error, not a coded reject shadowed by an emit-path decline.
    let one_error = |src: &str| {
        let out = crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(src)),
            )],
            &[crate::backend::Target::Wasm],
        );
        let errors: Vec<crate::abi::Diagnostic> = out
            .diagnostics
            .iter()
            .filter(|d| d.severity == crate::abi::Severity::Error)
            .cloned()
            .collect();
        assert_eq!(
            errors.len(),
            1,
            "a mistyped conversion = ONE error (the CDZ0203), no shadowing non-scalar decline: {} -> {:?}",
            src,
            out.diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            errors[0].code.as_deref(),
            Some("CDZ0203"),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("expects an argument of type")
                && !errors[0].message.contains("non-scalar operand"),
            "the one error is the arg-type CDZ0203, not the non-scalar decline: {}",
            errors[0].message
        );
    };
    one_error("(module m (def x (Int8.wrap 3.5)) (export x))");
    one_error("(module m (def x (UInt8.wrap \"hi\")) (export x))");
}

#[test]
fn a_map_insert_type_clash_names_the_map_and_operand_types() {
    // The Map.insert twin of the map-literal heterogeneity message (M75): inserting a key/value whose
    // type differs from the map's names BOTH the map's type and the operand's type (was a generic "the
    // inserted key's type differs from the map's"), and offers the int-literal→float retype where it
    // bridges the clash.
    let key = reject_full(
        "(module m (def (main) (Map.len ((. Map insert) (map (= 1 2)) \"k\" 3))) (export main))",
    )
    .expect("a Map.insert key-type clash must reject");
    assert_eq!(key.code.as_deref(), Some("CDZ0201"), "got: {}", key.message);
    assert!(
        key.message.contains("Int64") && key.message.contains("String"),
        "names the map's key type AND the inserted key type: {}",
        key.message
    );
    let val = reject_full(
        "(module m (def (main) (Map.len ((. Map insert) (map (= 1 2)) 9 \"v\"))) (export main))",
    )
    .expect("a Map.insert value-type clash must reject");
    assert!(
        val.message.contains("Int64") && val.message.contains("String"),
        "names the map's value type AND the inserted value type: {}",
        val.message
    );
    // Inserting an int VALUE into a map whose values are Float → the `n.0` retype fix (int-lit→float).
    let f = reject_full(
        "(module m (def (main) (Map.len ((. Map insert) (map (= 1 1.0)) 9 3))) (export main))",
    )
    .expect("reject");
    assert_eq!(
        f.fix.as_ref().map(|x| x.replacement.as_str()),
        Some("3.0"),
        "an int value inserted into a Float-valued map offers the retype: {}",
        f.message
    );
    // M184 audit: when the inserted VALUE and the map's value type are SAME-KIND compounds that differ
    // structurally (a record field-set diff here), the Map.insert arm appends the minimal-conflict
    // delta the map-LITERAL peer-join arm already carries — it names the field rather than leaving the
    // reader to diff `(Record (x Int64))` against `(Record (y Int64))`.
    let cd = reject_full(
        "(module m (def (f (: mm (Map String (Record (x Int64))))) \
             ((. Map insert) mm \"k\" (record (= y 2)))) (export f))",
    )
    .expect("a Map.insert compound value-type clash must reject");
    assert!(
        cd.message.contains("this value's type differs")
            && cd.message.contains("field `x`")
            && cd.message.contains('y'),
        "names the field-level delta on a compound value clash: {}",
        cd.message
    );
    // NO spurious delta on a SCALAR key clash — the earlier String-vs-Int64 key case carries no
    // structural-delta tail (structural_delta_hint is None for two scalars).
    assert!(
        !key.message.contains(" — "),
        "a scalar key clash carries no structural-delta tail: {}",
        key.message
    );
}

#[test]
fn a_map_homogeneity_reject_anchors_at_the_offending_entry_or_argument() {
    // The map homogeneity CDZ0201 rejects must anchor at the CULPRIT — the outlier entry's key/value in
    // a `(map …)` literal, or the inserted key/value in `Map.insert` — not the whole `(map …)` /
    // `(Map.insert …)` node, so the squiggle points at the minimal locus (the Map twin of the list-op
    // anchoring, PR #399). Without the anchor, `collect`'s `set_origin_if_absent` stamped the enclosing
    // node.
    let anchor_of = |src: &str| -> Option<String> {
        let d = reject_full(src)?;
        let db = crate::db::Db::load(parse(src));
        d.node
            .and_then(|n| db.ast.as_str(crate::ast::StructId(n)).map(str::to_string))
    };
    // Map LITERAL — the outlier value `"bad"` (a String among Int64 values) is the locus.
    assert_eq!(
        anchor_of(
            "(module m (def (main) ((. Map len) (map (= \"a\" 1) (= \"b\" \"bad\")))) (export main))"
        )
        .as_deref(),
        Some("bad"),
        "a map-literal value-heterogeneity reject anchors at the outlier value `\"bad\"`, not the map"
    );
    // Map.insert VALUE — the inserted `"bad"` (a String into a `Map String Int64`) is the locus.
    assert_eq!(
            anchor_of(
                "(module m (def (f (: mm (Map String Int64))) ((. Map insert) mm \"k\" \"bad\")) (export f))"
            )
            .as_deref(),
            Some("bad"),
            "a Map.insert value-clash reject anchors at the inserted value `\"bad\"`, not the call"
        );
    // Map.insert KEY — the inserted key `"nope"` (a String key where the map's keys are Int64) is the
    // locus. (Map keys Int64, value Int64; insert a String key.)
    assert_eq!(
            anchor_of(
                "(module m (def (f (: mm (Map Int64 Int64))) ((. Map insert) mm \"nope\" 1)) (export f))"
            )
            .as_deref(),
            Some("nope"),
            "a Map.insert key-clash reject anchors at the inserted key `\"nope\"`, not the call"
        );
}

mod part2;
