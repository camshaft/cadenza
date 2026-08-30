use crate::abi::{Artifact, Severity};
use crate::backend::Target;
use crate::compile::compile;
use crate::sidecar::{
    self, KIND_DIAGNOSTICS, KIND_DOC, KIND_EXPORTS, KIND_FUNC_LAYOUT, KIND_HIGHLIGHT,
    KIND_INSTANTIATIONS, KIND_PARAM_MANIFEST, KIND_RESOLVE, KIND_SCOPE, KIND_SYMBOLS, KIND_TYPE_AT,
    KIND_TYPE_INFO, KIND_USES, Query, Request,
};
use crate::testkit::parse;

/// Build the two input artifacts (the AST + a sidecar request list) for `src` and `requests`.
fn inputs(src: &str, requests: &[Request]) -> Vec<Artifact> {
    vec![
        Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&parse(src))),
        Artifact::new(sidecar::KIND_SIDECAR, "drive", sidecar::encode(requests)),
    ]
}

// The func-layout artifact is now canonical binary AST (operator P0 seq-284) rather than TAB text, so
// decode it via the shared codec and render the historical TAB rows — the SAME `render_text` the `cdz
// func-layout` CLI prints, so these assertions still pin the byte-stable text form.
fn func_layout_text(out: &crate::abi::CompileOutput) -> Option<String> {
    out.artifacts
        .iter()
        .find(|a| a.kind == KIND_FUNC_LAYOUT)
        .map(|a| {
            cadenza_compile_abi::func_layout_wire::render_text(
                &cadenza_compile_abi::func_layout_wire::decode(&a.bytes)
                    .expect("func-layout artifact decodes as binary AST"),
            )
        })
}

/// The RAW bytes of the first artifact of `kind` — for the query answers whose wire is canonical
/// binary AST (`KIND_USES`, …), which the test decodes via the shared `cadenza-compile-abi` codec.
fn artifact_bytes<'a>(out: &'a crate::abi::CompileOutput, kind: &str) -> Option<&'a [u8]> {
    out.artifacts
        .iter()
        .find(|a| a.kind == kind)
        .map(|a| a.bytes.as_slice())
}

/// The `(node-id, kind)` highlight pairs decoded from the binary-AST `KIND_HIGHLIGHT` wire.
fn highlight_pairs(out: &crate::abi::CompileOutput) -> Vec<(u32, String)> {
    cadenza_compile_abi::decode_highlight(
        artifact_bytes(out, KIND_HIGHLIGHT).expect("a highlight artifact"),
    )
}

/// Every `KIND_DOC` answer decoded from the binary-AST wire, in artifact (request) order.
fn doc_answers(out: &crate::abi::CompileOutput) -> Vec<cadenza_compile_abi::DocAnswer> {
    out.artifacts
        .iter()
        .filter(|a| a.kind == KIND_DOC)
        .map(|a| cadenza_compile_abi::decode_doc(&a.bytes))
        .collect()
}

#[test]
fn a_type_of_query_reads_the_type_column() {
    // A `TypeOf` request for a nullary def answers with its rendered type — the same canonical text
    // an annotation carries — read straight from the type column.
    let src = "(module m (def (main) (: 42 Int64)) (export main))";
    let out = compile(
        &inputs(
            src,
            &[Request::Query(Query::TypeOf {
                name: "main".into(),
            })],
        ),
        &[],
    );
    assert!(
        !out.has_error(),
        "a query does not fail: {:?}",
        out.diagnostics
    );
    // KIND_TYPE_INFO is now a tagged binary-AST verdict (`decode_type_info`); `main : Int64` is a
    // `Found` carrying the structured `(Int 64)` payload (head `Int`). The rendered display "Int64" is the
    // CONSUMER's job (render_ty_scheme).
    match cadenza_compile_abi::decode_type_info(
        artifact_bytes(&out, KIND_TYPE_INFO).expect("a type-info artifact"),
    ) {
        cadenza_compile_abi::TypeInfo::Found(ty) => {
            assert_eq!(ty.head_name(ty.root), Some("Int"), "main : Int64 payload")
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn a_type_of_query_for_a_function_renders_its_arrow_type() {
    // A def with a parameter denotes a function; its type is the arrow the annotation fixes — a `Found`
    // carrying a `(-> …)` arrow payload (head `->`); the "(-> Int64 Int64)" display is the consumer's job.
    let src = "(module m (def (f (: x Int64)) x) (def (main) (f 1)) (export main))";
    let out = compile(
        &inputs(src, &[Request::Query(Query::TypeOf { name: "f".into() })]),
        &[],
    );
    match cadenza_compile_abi::decode_type_info(
        artifact_bytes(&out, KIND_TYPE_INFO).expect("a type-info artifact"),
    ) {
        cadenza_compile_abi::TypeInfo::Found(ty) => {
            assert_eq!(ty.head_name(ty.root), Some("->"), "f is a function arrow")
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn a_type_of_query_emits_the_generic_scheme_arrow_payload() {
    // A GENERIC def's `TypeOf` verdict is a `Found` carrying the generalized scheme's STRUCTURED arrow
    // payload (the tie structure lives in the payload's `(Var N)` numbers — the SAME var on both sides of
    // `from-list : (-> (List a) (Iter a))`). Rendering those vars as stable letters `a`,`b`,… (vs collapsed
    // `_`), so a reader sees the tie, is now the CONSUMER's job via `render_ty_scheme` (v-syntax's parity
    // battery covers the lettering); here we assert the producer emits the arrow payload for both a generic
    // and a monomorphic scheme.
    let src = "(module m (type Iter (Nil) (Cons a (Iter a))) \
                    (def (from-list xs) (match xs ((list) (Iter.Nil)) ((list h .. t) (Iter.Cons h (from-list t))))) \
                    (def (main) 0) (export main))";
    let out = compile(
        &inputs(
            src,
            &[Request::Query(Query::TypeOf {
                name: "from-list".into(),
            })],
        ),
        &[],
    );
    match cadenza_compile_abi::decode_type_info(
        artifact_bytes(&out, KIND_TYPE_INFO).expect("a type-info artifact"),
    ) {
        cadenza_compile_abi::TypeInfo::Found(ty) => {
            assert_eq!(
                ty.head_name(ty.root),
                Some("->"),
                "from-list is a function arrow"
            )
        }
        other => panic!("expected Found, got {other:?}"),
    }
    // A MONOMORPHIC scheme also emits its arrow payload.
    let mono = "(module m (def (f (: x Int64)) x) (def (main) (f 1)) (export main))";
    let out2 = compile(
        &inputs(mono, &[Request::Query(Query::TypeOf { name: "f".into() })]),
        &[],
    );
    match cadenza_compile_abi::decode_type_info(
        artifact_bytes(&out2, KIND_TYPE_INFO).expect("a type-info artifact"),
    ) {
        cadenza_compile_abi::TypeInfo::Found(ty) => {
            assert_eq!(ty.head_name(ty.root), Some("->"), "f is a function arrow")
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn a_type_of_query_for_an_unknown_name_is_total() {
    // Querying a name that names no definition yields a DEFINED result, never an error — the
    // oracle contract (a query is total over every input). The result names the missing definition
    // and, like the compiler's unbound-name sites, offers the nearest defined name as a "did you
    // mean?"/"closest matches" hint (a `TypeOf` for a near-typo of a real def is almost always a typo).
    let src = "(module m (def (main) 42) (export main))";
    let out = compile(
        &inputs(
            src,
            &[Request::Query(Query::TypeOf {
                name: "ghost".into(),
            })],
        ),
        &[],
    );
    assert!(!out.has_error());
    // A name that names nothing → the `NoDef` verdict carrying the total message (the consumer prints it +
    // exits FAILURE — no string-match needed).
    match cadenza_compile_abi::decode_type_info(
        artifact_bytes(&out, KIND_TYPE_INFO).expect("a type-info artifact"),
    ) {
        cadenza_compile_abi::TypeInfo::NoDef(msg) => assert!(
            msg.starts_with("no such definition `ghost`"),
            "names the missing definition: {msg}"
        ),
        other => panic!("expected NoDef, got {other:?}"),
    }

    // A NEAR-typo of a real def gets a confident "did you mean?" pointing at it.
    let out2 = compile(
        &inputs(
            "(module m (def (compute) 42) (export compute))",
            &[Request::Query(Query::TypeOf {
                name: "computee".into(),
            })],
        ),
        &[],
    );
    match cadenza_compile_abi::decode_type_info(
        artifact_bytes(&out2, KIND_TYPE_INFO).expect("a type-info artifact"),
    ) {
        cadenza_compile_abi::TypeInfo::NoDef(msg) => assert_eq!(
            msg,
            "no such definition `computee` — did you mean `compute`?"
        ),
        other => panic!("expected NoDef, got {other:?}"),
    }
}

#[test]
fn a_query_answers_even_when_the_program_fails_to_emit() {
    // The program is ill-typed (`(if 5 1 2)` — a non-Bool condition, CDZ0203), so no component is
    // produced. But a `TypeOf` query is a PURE fact read that never denies an artifact: the answer
    // rides ALONGSIDE the error diagnostic. This is the "branch on a fact even for a broken program"
    // affordance — the whole reason a query is not gated on a clean emit.
    let src = "(module m (def (g) (: 7 Int64)) (def (main) (if 5 1 2)) (export main))";
    let out = compile(
        &inputs(
            src,
            &[
                Request::Query(Query::TypeOf { name: "g".into() }),
                Request::Emit(Target::Wasm),
            ],
        ),
        &[],
    );
    // Emit failed: an error diagnostic, and NO component artifact.
    assert!(out.has_error());
    assert!(out.artifact("component").is_none());
    // …but the query still answered, carried past the failure — a `Found` with `g`'s `(Int 64)` payload.
    match cadenza_compile_abi::decode_type_info(
        artifact_bytes(&out, KIND_TYPE_INFO).expect("a type-info artifact"),
    ) {
        cadenza_compile_abi::TypeInfo::Found(ty) => {
            assert_eq!(ty.head_name(ty.root), Some("Int"), "g : Int64 payload")
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn emit_tests_accepts_a_parameterized_property_test() {
    // A `@test` WITH parameters is a PROPERTY test: `compute_tests` crosses its params as ordinary
    // boundary parameters (was a hard "a test must be NULLARY" reject) so `cdz test` can invoke it with
    // generated inputs. Here `(@ test (def (prop (: n Int64)) …))` must EMIT a test component whose
    // export takes an Int64 param. A scalar param is boundary-representable, so the emit succeeds.
    let src = "(do (effect Test (op fail (-> String Unit))) \
                    (@ test (def (prop (: n Int64)) \
                       (if (> n n) (host (Test) (do ((. Test fail) \"x\") (trap \"x\"))) unit)))) ";
    let out = compile(&inputs(src, &[Request::EmitTests]), &[]);
    assert!(
        !out.has_error(),
        "a parameterized @test must emit a test component (property test): {:?}",
        out.diagnostics
    );
    assert!(
        out.artifacts.iter().any(|a| a.kind == "component"),
        "the test build produces a component artifact"
    );
}

#[test]
fn emit_tests_per_file_emits_one_component_per_file_byte_identical_to_a_per_file_build() {
    // `EmitTestsPerFile` lowers a linked closure ONCE and emits one `@test` wasm component PER FILE,
    // each artifact NAMED by the file's `link` path — instead of a separate per-file compile that
    // re-lowers the whole closure. The behavior CONTRACT this pins: each file's component is
    // BYTE-IDENTICAL to what a standalone `EmitTests` compile of that file alone produces (same tests,
    // same layout-view over the same Core) — so a caller can swap N per-file compiles for ONE
    // EmitTestsPerFile + demux-by-name with no observable change.
    use crate::abi::Artifact;
    let file_a = "(do (def (fa) 1) (@ test (def (ta) (if (= (fa) 1) unit (trap \"a\")))))";
    let file_b = "(do (def (fb) 2) (@ test (def (tb) (if (= (fb) 2) unit (trap \"b\")))))";
    // Per-file build: each file ALONE through `EmitTests` (the standalone oracle).
    let per_file = |src: &str| -> Vec<u8> {
        let out = compile(&inputs(src, &[Request::EmitTests]), &[]);
        out.artifacts
            .iter()
            .find(|a| a.kind == "component")
            .map(|a| a.bytes.clone())
            .unwrap_or_else(|| {
                panic!(
                    "standalone EmitTests emits a component: {:?}",
                    out.diagnostics
                )
            })
    };
    let oracle_a = per_file(file_a);
    let oracle_b = per_file(file_b);
    // Shared build: BOTH files linked into one arena + `EmitTestsPerFile` → 2 file-tagged components.
    let out = crate::host::run_with_compiler_stack(|| {
        crate::compile::compile(
            &[
                Artifact::new(
                    Artifact::KIND_AST,
                    "file_a",
                    crate::codec::encode(&parse(file_a)),
                ),
                Artifact::new(
                    Artifact::KIND_AST,
                    "file_b",
                    crate::codec::encode(&parse(file_b)),
                ),
                // A multi-file package needs an `entry` marker (as `cdz test` supplies); it names the
                // linkage-driving file but does NOT restrict which files' @tests emit — EmitTestsPerFile
                // buckets ALL linked test defs by file.
                cadenza_compile_abi::abi::entry_artifact("file_a"),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::EmitTestsPerFile]),
                ),
            ],
            &[],
        )
    });
    assert!(
        !out.has_error(),
        "EmitTestsPerFile compiles: {:?}",
        out.diagnostics
    );
    let components: Vec<_> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == "component")
        .collect();
    assert_eq!(components.len(), 2, "one component per file with a @test");
    let comp_a = components
        .iter()
        .find(|a| a.name == "file_a")
        .expect("a component named file_a");
    let comp_b = components
        .iter()
        .find(|a| a.name == "file_b")
        .expect("a component named file_b");
    // The load-bearing claim: per-file view over the shared arena == the standalone per-file compile.
    assert_eq!(
        comp_a.bytes, oracle_a,
        "file_a's shared-arena view is byte-identical to its standalone EmitTests"
    );
    assert_eq!(
        comp_b.bytes, oracle_b,
        "file_b's shared-arena view is byte-identical to its standalone EmitTests"
    );
}

#[test]
fn option_c_partition_splits_own_test_defs_from_the_shared_imported_closure() {
    // OPTION C increment (a): `layout::partition_reachable_for_file` splits a test build's reachable
    // defs into `own` (the test file's own @test bodies + file-local helpers) vs `shared` (defs from an
    // IMPORTED file — the closure Option C emits ONCE as its own component). Build a 2-file package: `lib`
    // exports `shared_helper`; `app` imports it and a @test calls it. The @test-reachable set spans both
    // files; the partition must put `app`'s test in `own` and `lib`'s helper in `shared`.
    use crate::testkit::parse;
    // `shared-helper` is RECURSIVE so it stays a standalone emitted def (a small non-recursive fn would
    // INLINE into the caller, leaving no `shared` row) — mirroring the FuncLayout tests' `sumto`.
    let lib = "(do (def (shared-helper (: n Int64)) (if (= n 0) 0 (+ 1 (shared-helper (- n 1))))) \
                    (export shared-helper))";
    let app = "(do (import \"lib\" (shared-helper)) \
                    (@ test (def (t-app) (if (= (shared-helper 5) 5) unit (trap \"x\")))))";
    // Compute + partition INSIDE the compiler-stack closure (Layout holds `Rc`, not `Send`, so it can't
    // cross the thread boundary) — return only the `Send` result: (own, shared, order len, cross-edges).
    let (own_names, shared_names, order_len, edge_names): (
        Vec<String>,
        Vec<String>,
        usize,
        Vec<String>,
    ) = crate::host::run_with_compiler_stack(|| {
        let files: Vec<(String, crate::ast::Arenas)> = vec![
            ("lib".to_string(), parse(lib)),
            ("app".to_string(), parse(app)),
        ];
        let linked = crate::link::link(&files, "app").expect("package links");
        let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
        let layout = crate::layout::compute_tests(&mut db).expect("the @test build lays out");
        // The @test file is `app` — its file index off the @test def's sig-occ (as EmitTestsPerFile).
        let test_def = *db.test_defs().first().expect("one @test");
        let own_file = db
            .file_of(db.defs[test_def].sig_occ)
            .expect("the @test def is in a file");
        let (own, shared) = crate::layout::partition_reachable_for_file(&db, &layout, own_file);
        // The cross-component interface set = shared defs an `own` def CALLS (own→shared edges).
        let edges = crate::layout::cross_component_edges(&mut db, &layout, own_file);
        (
            own.iter().map(|&d| db.defs[d].name.clone()).collect(),
            shared.iter().map(|&d| db.defs[d].name.clone()).collect(),
            layout.order.len(),
            edges.iter().map(|&d| db.defs[d].name.clone()).collect(),
        )
    });
    // The @test body is in `own`; the imported helper is in `shared` (it's `lib`'s, not `app`'s).
    assert!(
        own_names.iter().any(|n| n.starts_with("t-app")),
        "the @test body is in `own`: own={own_names:?}"
    );
    assert!(
        shared_names.iter().any(|n| n.starts_with("shared-helper")),
        "the imported helper is in `shared`: shared={shared_names:?}"
    );
    // Total over the reachable set (disjoint by construction — each def goes to exactly one side).
    assert_eq!(
        own_names.len() + shared_names.len(),
        order_len,
        "partition is total over the reachable set"
    );
    // The CROSS-COMPONENT INTERFACE set = shared defs an `own` def CALLS. `t-app` calls `shared-helper`,
    // so `shared-helper` is a cross-edge (the interface func the @test component imports). Every edge is
    // a SHARED def (never an own def — a cross-edge crosses the file boundary by definition).
    assert!(
        edge_names.iter().any(|n| n.starts_with("shared-helper")),
        "the called imported helper is a cross-component edge: edges={edge_names:?}"
    );
    for e in &edge_names {
        assert!(
            shared_names.iter().any(|s| s == e),
            "every cross-edge is a shared def, not own: edge `{e}` not in shared={shared_names:?}"
        );
    }
}

#[test]
fn option_c_cross_component_edges_union_covers_every_files_cross_edge() {
    // OPTION C increment (c)(iii): `cross_component_edges_union` folds the per-file cross-edge sets across
    // MANY files into ONE canonical set — the shared-closure PROVIDER's export set for a composed `cdz test
    // <dir>` build (`EmitTestsComposed`). Fixture: `lib` exports two recursive helpers alpha/beta; two
    // SEPARATE @test files each call a DIFFERENT one (`appA` → alpha, `appB` → beta). Per-file cross-edges
    // are each a SUBSET (appA={alpha}, appB={beta}); the UNION over both files = {alpha, beta}, in
    // `layout.order` order (the canonical order the provider exports + every consumer imports). This is the
    // union primitive the composed provider needs (one provider for the whole dir, not per-file).
    use crate::testkit::parse;
    let lib = "(do \
                    (def (alpha (: n Int64)) (if (= n 0) 0 (+ 2 (alpha (- n 1))))) \
                    (def (beta (: n Int64)) (if (= n 0) 1 (+ 3 (beta (- n 1))))) \
                    (export alpha) (export beta))";
    let app_a = "(do (import \"lib\" (alpha)) \
                    (@ test (def (t-a) (if (= (alpha 3) 6) unit (trap \"x\")))))";
    let app_b = "(do (import \"lib\" (beta)) \
                    (@ test (def (t-b) (if (= (beta 3) 10) unit (trap \"x\")))))";
    let (per_file_a, per_file_b, union_names): (Vec<String>, Vec<String>, Vec<String>) =
        crate::host::run_with_compiler_stack(|| {
            let files: Vec<(String, crate::ast::Arenas)> = vec![
                ("lib".to_string(), parse(lib)),
                ("app-a".to_string(), parse(app_a)),
                ("app-b".to_string(), parse(app_b)),
            ];
            let linked = crate::link::link(&files, "app-a").expect("package links");
            let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
            let layout = crate::layout::compute_tests(&mut db).expect("@test build lays out");
            // The two @test files' indices — bucket test_defs by file (as EmitTestsComposed will).
            let mut files_seen: Vec<usize> = db
                .test_defs()
                .iter()
                .filter_map(|&t| db.file_of(db.defs[t].sig_occ))
                .collect();
            files_seen.sort_unstable();
            files_seen.dedup();
            assert_eq!(files_seen.len(), 2, "two @test files (app-a, app-b)");
            let a = crate::layout::cross_component_edges(&mut db, &layout, files_seen[0]);
            let b = crate::layout::cross_component_edges(&mut db, &layout, files_seen[1]);
            let u = crate::layout::cross_component_edges_union(&mut db, &layout, &files_seen);
            (
                a.iter().map(|&d| db.defs[d].name.clone()).collect(),
                b.iter().map(|&d| db.defs[d].name.clone()).collect(),
                u.iter().map(|&d| db.defs[d].name.clone()).collect(),
            )
        });
    // Each file's per-file cross-edge set is a SINGLE helper (a proper subset of the union).
    assert_eq!(
        per_file_a.len(),
        1,
        "app-a calls exactly one helper: {per_file_a:?}"
    );
    assert_eq!(
        per_file_b.len(),
        1,
        "app-b calls exactly one helper: {per_file_b:?}"
    );
    assert_ne!(
        per_file_a, per_file_b,
        "the two files call DIFFERENT helpers (else the union wouldn't be exercised)"
    );
    // The UNION covers BOTH — alpha and beta.
    assert!(
        union_names.iter().any(|n| n.starts_with("alpha")),
        "union covers app-a's edge alpha: union={union_names:?}"
    );
    assert!(
        union_names.iter().any(|n| n.starts_with("beta")),
        "union covers app-b's edge beta: union={union_names:?}"
    );
    assert_eq!(
        union_names.len(),
        2,
        "union is exactly {{alpha, beta}}: {union_names:?}"
    );
    // Each per-file edge is a member of the union (subset containment).
    for e in per_file_a.iter().chain(per_file_b.iter()) {
        assert!(
            union_names.iter().any(|u| u == e),
            "per-file edge `{e}` must be in the union {union_names:?}"
        );
    }
}

#[test]
fn emit_tests_composed_emits_a_provider_iface_sidecar_and_per_file_consumers() {
    // OPTION C (c)(iii)c — the END-TO-END composed driver: a `Request::EmitTestsComposed` compile over a
    // multi-file package emits ONE `component-provider` (the hoisted shared closure), ONE `component-name`
    // sidecar (its interface string), and N `component` consumers (one per @test file, named by link path,
    // each importing the provider). 3-file fixture: `lib` exports two recursive helpers; `app-a`'s @test
    // calls alpha, `app-b`'s calls beta — so the union closure is {alpha, beta} and there are 2 consumers.
    let lib = crate::codec::encode(&parse(
        "(do \
             (def (alpha (: n Int64)) (if (= n 0) 0 (+ 2 (alpha (- n 1))))) \
             (def (beta (: n Int64)) (if (= n 0) 1 (+ 3 (beta (- n 1))))) \
             (export alpha) (export beta))",
    ));
    let app_a = crate::codec::encode(&parse(
        "(do (import \"lib\" (alpha)) (@ test (def (t-a) (if (= (alpha 3) 6) unit (trap \"x\")))))",
    ));
    let app_b = crate::codec::encode(&parse(
        "(do (import \"lib\" (beta)) (@ test (def (t-b) (if (= (beta 3) 10) unit (trap \"x\")))))",
    ));
    let out = crate::host::run_with_compiler_stack(|| {
        crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "lib", lib.clone()),
                Artifact::new(Artifact::KIND_AST, "app-a", app_a.clone()),
                Artifact::new(Artifact::KIND_AST, "app-b", app_b.clone()),
                // A multi-file package needs an entry (linkage root); a composed test build roots on the
                // `@test` defs, so any member serves — `cdz test <dir>` passes one likewise.
                cadenza_compile_abi::abi::entry_artifact("app-a"),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::EmitTestsComposed]),
                ),
            ],
            &[],
        )
    });
    assert!(
        !out.has_error(),
        "composed emit does not error: {:?}",
        out.diagnostics
    );
    // Exactly one provider + one interface-name sidecar.
    let providers: Vec<&Artifact> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == "component-provider")
        .collect();
    assert_eq!(
        providers.len(),
        1,
        "one shared-closure provider component: kinds={:?}",
        out.artifacts.iter().map(|a| &a.kind).collect::<Vec<_>>()
    );
    let iface_sidecar: Vec<&Artifact> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == crate::link::KIND_COMPONENT_NAME)
        .collect();
    assert_eq!(iface_sidecar.len(), 1, "one component-name iface sidecar");
    assert_eq!(
        cadenza_compile_abi::decode_name(&iface_sidecar[0].bytes).unwrap(),
        "cadenza:closure/api",
        "the iface sidecar carries the fixed closure-interface string"
    );
    // The provider's name IS the interface (so a runner can pair it with the sidecar).
    assert_eq!(providers[0].name, "cadenza:closure/api");
    // The closure-hash sidecar IS emitted on the MISS (provider) path — a runner persists the provider
    // keyed by it (recompute-free) + validates its own fold against it. A non-empty hex u64.
    let hashes: Vec<&Artifact> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == crate::sidecar::KIND_CLOSURE_HASH)
        .collect();
    assert_eq!(
        hashes.len(),
        1,
        "one closure-hash sidecar on the composed (miss) path"
    );
    // The closure-hash wire is canonical binary AST (a root `Ast.Int` u64) — decode via the shared codec
    // and render the `{:016x}` key form (operator P0 seq-284: binary AST everywhere, no bespoke hex text).
    let hstr = format!(
        "{:016x}",
        cadenza_compile_abi::decode_closure_hash(&hashes[0].bytes)
            .expect("closure-hash decodes to a u64")
    );
    assert!(
        hstr.len() == 16 && hstr.chars().all(|c| c.is_ascii_hexdigit()),
        "closure-hash is a 16-hex-digit u64: {hstr:?}"
    );
    // N=2 consumer `component` artifacts, named by the two @test files' link paths.
    let consumer_names: Vec<&str> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == crate::backend::Target::Wasm.artifact_kind())
        .map(|a| a.name.as_str())
        .collect();
    assert!(
        consumer_names.contains(&"app-a") && consumer_names.contains(&"app-b"),
        "one consumer component per @test file (by link name): {consumer_names:?}"
    );
    assert_eq!(
        consumer_names.len(),
        2,
        "exactly two consumers (one per @test file): {consumer_names:?}"
    );
    // Every emitted component (provider + consumers) validates.
    for a in &out.artifacts {
        if a.kind == "component-provider" || a.kind == crate::backend::Target::Wasm.artifact_kind()
        {
            let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
            v.validate_all(&a.bytes)
                .unwrap_or_else(|e| panic!("composed artifact `{}` validates: {e}", a.name));
        }
    }
}

/// `Request::EmitTestsShred` (compiler-driven shred, §S6b): emit ONE whole-library MAIN provider + one
/// thin CONSUMER per `@test`. This pins the KEY property the composed path lacks — the main is the WHOLE
/// library (all reachable EMITTED non-`@test` defs), NOT just the cross-FILE closure — so a SAME-FILE suite
/// (no cross-file imports) STILL gets a non-empty main + one per-test consumer each (uniform per-test
/// linking). Fixture: a single file with a RECURSIVE helper `tri` (recursive ⇒ emitted as a standalone
/// function, not inlined into each test — a trivial non-recursive helper would β-inline and leave `main`
/// empty; the caching-relevant shared defs are exactly the emitted ones) + two `@test`s that call it, so the
/// whole-library boundary is `{tri}` and there are exactly 2 per-test consumers (`t-a`, `t-b`).
#[test]
fn emit_tests_shred_same_file_suite_gets_a_whole_library_main_and_per_test_consumers() {
    let src = crate::codec::encode(&parse(
        "(do \
             (def (tri (: n Int64)) (if (= n 0) 0 (+ n (tri (- n 1))))) \
             (@ test (def (t-a) (if (= (tri 3) 6) unit (trap \"x\")))) \
             (@ test (def (t-b) (if (= (tri 4) 10) unit (trap \"x\")))))",
    ));
    let out = crate::host::run_with_compiler_stack(|| {
        crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "suite", src.clone()),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::EmitTestsShred]),
                ),
            ],
            &[],
        )
    });
    assert!(
        !out.has_error(),
        "shred emit does not error: {:?}",
        out.diagnostics
    );
    // Exactly ONE whole-library MAIN provider, named + carrying the closure interface.
    let providers: Vec<&Artifact> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == "component-provider")
        .collect();
    assert_eq!(
        providers.len(),
        1,
        "one whole-library main provider (even though the suite is SAME-FILE — no cross-file closure): \
             kinds={:?}",
        out.artifacts.iter().map(|a| &a.kind).collect::<Vec<_>>()
    );
    // MAIN artifact NAMED "main" (the file key → `main.wasm` under `-o D`), NOT the iface (the iface is the
    // component's INTERFACE identity, carried in the manifest's `main-iface`). No `component-name` sidecar in
    // the shred output — the manifest carries the iface.
    assert_eq!(providers[0].name, "main");
    assert!(
        out.artifacts
            .iter()
            .all(|a| a.kind != crate::link::KIND_COMPONENT_NAME),
        "shred emits NO component-name sidecar (the manifest carries main-iface)"
    );
    // ONE thin CONSUMER per @test, artifact NAMED `test-<def-name>` (→ `test-<name>.wasm` under `-o D`,
    // matching the manifest `target`).
    let consumer_names: Vec<&str> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == crate::backend::Target::Wasm.artifact_kind())
        .map(|a| a.name.as_str())
        .collect();
    assert!(
        consumer_names.contains(&"test-t-a") && consumer_names.contains(&"test-t-b"),
        "one consumer component per @test (named test-<def-name>): {consumer_names:?}"
    );
    assert_eq!(
        consumer_names.len(),
        2,
        "exactly two per-test consumers: {consumer_names:?}"
    );
    // Every emitted component (main + per-test consumers) validates as a wasm component.
    for a in &out.artifacts {
        if a.kind == "component-provider" || a.kind == crate::backend::Target::Wasm.artifact_kind()
        {
            let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
            v.validate_all(&a.bytes)
                .unwrap_or_else(|e| panic!("shred artifact `{}` validates: {e}", a.name));
        }
    }
    // The MANIFEST — a cadenza-ast VALUE (codec-encoded), one `(entry name is-property file export
    // target main-iface)` per emitted test. Decode it + assert the shape + field values the runner reads.
    let manifest_art = out
        .artifacts
        .iter()
        .find(|a| a.kind == crate::sidecar::KIND_SHRED_MANIFEST)
        .expect("a shred-manifest artifact");
    let arena = crate::codec::decode(&manifest_art.bytes).expect("manifest decodes as cadenza-ast");
    let root = arena.root;
    let entries = arena
        .as_form(root, "shred-manifest")
        .expect("root is (shred-manifest …)");
    assert_eq!(entries.len(), 2, "one manifest entry per emitted test");
    // Collect (is_property, export, target, main-iface, main-file) per entry; assert the fields for t-a/t-b.
    let mut seen: std::collections::HashMap<String, (bool, String, String, String, String)> =
        std::collections::HashMap::new();
    for &e in entries {
        let fields = arena.as_form(e, "entry").expect("each child is (entry …)");
        assert_eq!(
            fields.len(),
            7,
            "entry = name is-property file export target main-iface main-file"
        );
        let name = arena.as_str(fields[0]).expect("name Str").to_string();
        let is_property = arena.as_bool(fields[1]).expect("is-property Bool");
        let export = arena.as_str(fields[3]).expect("export Str").to_string();
        let target = arena.as_str(fields[4]).expect("target Str").to_string();
        let iface = arena.as_str(fields[5]).expect("main-iface Str").to_string();
        let main_file = arena.as_str(fields[6]).expect("main-file Str").to_string();
        seen.insert(name, (is_property, export, target, iface, main_file));
    }
    for t in ["t-a", "t-b"] {
        let (is_prop, export, target, iface, main_file) = seen
            .get(t)
            .unwrap_or_else(|| panic!("manifest has an entry for {t}"));
        assert!(!is_prop, "{t} is a nullary unit test (is-property=false)");
        assert_eq!(
            export, t,
            "export symbol = the raw @test def name for a plain @test"
        );
        assert_eq!(
            target,
            &format!("test-{t}.wasm"),
            "target = test-<name>.wasm"
        );
        assert_eq!(
            iface, "cadenza:closure/api",
            "main-iface = the --peer interface"
        );
        // This suite HAS a shared library (the recursive `tri` helper), so main-file = main.wasm.
        assert_eq!(
            main_file, "main.wasm",
            "main-file = the main this test --peers"
        );
    }
}

/// `Request::EmitTestsShredTwoStage` (§S6b two-stage): emit cadenza-ast FRAGMENTS, not wasm — ONE shared
/// no-export closure fragment (`closure`, the reachable non-`@test` library) + one per-`@test` fragment
/// (`test-<name>`), each a bare `(do (def..)..)` with NO export (the export is added later by the
/// `--export` splice). Same recursive-`tri` fixture as the wasm shred test. Pins: the closure fragment
/// carries `tri` (and no `(export …)`), each per-test fragment carries ONLY its own test def, and the
/// manifest records `target`=`test-<name>.cdzb` + `main-file`=`closure.cdzb` per entry (the two fragments
/// the fan-out splice-compiles via `rcdzc closure.cdzb test-<name>.cdzb --export <name>`).
#[test]
fn emit_tests_shred_two_stage_emits_closure_and_per_test_fragments() {
    let src = crate::codec::encode(&parse(
        "(do \
             (def (tri (: n Int64)) (if (= n 0) 0 (+ n (tri (- n 1))))) \
             (@ test (def (t-a) (if (= (tri 3) 6) unit (trap \"x\")))) \
             (@ test (def (t-b) (if (= (tri 4) 10) unit (trap \"x\")))))",
    ));
    let out = crate::host::run_with_compiler_stack(|| {
        crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "suite", src.clone()),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::EmitTestsShredTwoStage]),
                ),
            ],
            &[],
        )
    });
    assert!(
        !out.has_error(),
        "two-stage emit does not error: {:?}",
        out.diagnostics
    );
    // NO wasm/provider artifacts — two-stage emits FRAGMENTS (kind `ast`) only.
    assert!(
        out.artifacts.iter().all(|a| a.kind != "component-provider"
            && a.kind != crate::backend::Target::Wasm.artifact_kind()),
        "two-stage emits fragments, not wasm: kinds={:?}",
        out.artifacts.iter().map(|a| &a.kind).collect::<Vec<_>>()
    );
    // The shared closure fragment: a bare `(do (def tri …) …)` with NO `(export …)`.
    let closure = out
        .artifacts
        .iter()
        .find(|a| a.kind == Artifact::KIND_AST && a.name == "closure")
        .expect("a `closure` ast fragment");
    let ca = crate::codec::decode(&closure.bytes).expect("closure decodes as cadenza-ast");
    let citems = ca.as_form(ca.root, "do").expect("closure root is `(do …)`");
    let cnames: Vec<Option<&str>> = citems.iter().map(|&i| ca.head_name(i)).collect();
    assert!(
        cnames.iter().all(|h| *h != Some("export")),
        "closure fragment carries NO export: {cnames:?}"
    );
    // The closure carries the reachable library — at least one `(def …)` (the recursive `tri` helper,
    // emitted standalone rather than inlined; its exact name is optimization-dependent, e.g. an
    // accumulator-intro rename `tri$acc`, so assert a library def is PRESENT, not a specific name).
    assert!(
        citems.iter().any(|&i| ca.as_form(i, "def").is_some()),
        "closure fragment carries the reachable library def(s)"
    );
    // One per-test fragment each, named `test-<name>`, carrying ONLY that test's def.
    for tname in ["t-a", "t-b"] {
        let frag = out
            .artifacts
            .iter()
            .find(|a| a.kind == Artifact::KIND_AST && a.name == format!("test-{tname}"))
            .unwrap_or_else(|| panic!("a `test-{tname}` fragment"));
        let ta = crate::codec::decode(&frag.bytes).expect("per-test decodes");
        let titems = ta
            .as_form(ta.root, "do")
            .expect("per-test root is `(do …)`");
        assert!(
            titems.iter().all(|&i| ta.head_name(i) != Some("export")),
            "per-test fragment carries no export"
        );
    }
    // The manifest: `target`=`test-<name>.cdzb` + `main-file`=`closure.cdzb` per entry.
    let manifest = out
        .artifacts
        .iter()
        .find(|a| a.kind == crate::sidecar::KIND_SHRED_MANIFEST)
        .expect("a shred-manifest artifact");
    let ma = crate::codec::decode(&manifest.bytes).expect("manifest decodes");
    let entries = ma
        .as_form(ma.root, "shred-manifest")
        .expect("root is `(shred-manifest …)`");
    assert_eq!(entries.len(), 2, "one entry per @test");
    for &e in entries {
        let fields = ma.as_form(e, "entry").expect("`(entry …)`");
        // Positional (head stripped): [0]name [1]is-property [2]file [3]export [4]target [5]main-iface
        // [6]main-file.
        let name = ma.as_str(fields[0]).unwrap_or("");
        assert_eq!(
            ma.as_str(fields[3]),
            Some(name),
            "export = the test's raw name"
        );
        assert_eq!(
            ma.as_str(fields[4]),
            Some(format!("test-{name}.cdzb").as_str()),
            "target = the per-test fragment"
        );
        assert_eq!(
            ma.as_str(fields[6]),
            Some("closure.cdzb"),
            "main-file = the shared closure fragment"
        );
    }
}

/// `EmitTestsShred` on a STANDALONE suite (no emitted shared library — the `@test`s call only prims /
/// inlined defs, so `library_edges` is empty): emit NO main, each `@test` a SELF-CONTAINED component, and
/// the manifest `main-file` = "" (v-test-shred's exec then runs the target with NO `--peer`). This pins the
/// independent-file case (e.g. `iterators`, whose files declare no imports) — the shred must still produce
/// runnable per-test targets + a manifest, not decline.
#[test]
fn emit_tests_shred_standalone_suite_has_no_main_and_empty_main_file() {
    // Two @tests that call NO user def (only prim `=`/`+`), so nothing lands in the library edge set.
    let src = crate::codec::encode(&parse(
        "(do \
             (@ test (def (s-a) (if (= (+ 1 1) 2) unit (trap \"x\")))) \
             (@ test (def (s-b) (if (= (+ 2 2) 4) unit (trap \"x\")))))",
    ));
    let out = crate::host::run_with_compiler_stack(|| {
        crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "suite", src.clone()),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::EmitTestsShred]),
                ),
            ],
            &[],
        )
    });
    assert!(
        !out.has_error(),
        "standalone shred does not error: {:?}",
        out.diagnostics
    );
    // NO main provider (empty library).
    assert!(
        out.artifacts.iter().all(|a| a.kind != "component-provider"),
        "a standalone suite emits NO main provider"
    );
    // Two self-contained per-test components (test-s-a / test-s-b), all valid.
    let consumer_names: Vec<&str> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == crate::backend::Target::Wasm.artifact_kind())
        .map(|a| a.name.as_str())
        .collect();
    assert!(
        consumer_names.contains(&"test-s-a") && consumer_names.contains(&"test-s-b"),
        "self-contained per-test components: {consumer_names:?}"
    );
    for a in &out.artifacts {
        if a.kind == crate::backend::Target::Wasm.artifact_kind() {
            let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
            v.validate_all(&a.bytes)
                .unwrap_or_else(|e| panic!("standalone shred `{}` validates: {e}", a.name));
        }
    }
    // The manifest lists both, with main-file "" (→ the runner runs them with NO --peer).
    let manifest = out
        .artifacts
        .iter()
        .find(|a| a.kind == crate::sidecar::KIND_SHRED_MANIFEST)
        .expect("a shred-manifest artifact");
    let arena = crate::codec::decode(&manifest.bytes).expect("manifest decodes");
    let entries = arena
        .as_form(arena.root, "shred-manifest")
        .expect("(shred-manifest …)");
    assert_eq!(entries.len(), 2, "both standalone tests listed");
    for &e in entries {
        let f = arena.as_form(e, "entry").expect("(entry …)");
        assert_eq!(
            arena.as_str(f[6]).expect("main-file Str"),
            "",
            "standalone test has empty main-file (run with no --peer)"
        );
    }
}

/// `EmitTestsShredStandalone` FORCES the standalone shape even when a shared LIBRARY exists — the
/// operator-hybrid mode for small-closure suites: no main, each `@test` a SELF-CONTAINED component (its lib
/// INLINED), `main-file` = "". Contrast `EmitTestsShred` on the SAME fixture (recursive `tri` helper), which
/// emits a main + consumers. This is the key win: a compound-param `@test` that would DECLINE at the peer
/// boundary shreds cleanly here (no boundary), so a suite gets FULL coverage. Fixture reuses the recursive
/// helper (which UNDER `EmitTestsShred` produces a main) to prove the variant SUPPRESSES it.
#[test]
fn emit_tests_shred_standalone_forces_no_main_even_with_a_library() {
    let src = crate::codec::encode(&parse(
        "(do \
             (def (tri (: n Int64)) (if (= n 0) 0 (+ n (tri (- n 1))))) \
             (@ test (def (t-a) (if (= (tri 3) 6) unit (trap \"x\")))) \
             (@ test (def (t-b) (if (= (tri 4) 10) unit (trap \"x\")))))",
    ));
    let out = crate::host::run_with_compiler_stack(|| {
        crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "suite", src.clone()),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::EmitTestsShredStandalone]),
                ),
            ],
            &[],
        )
    });
    assert!(
        !out.has_error(),
        "standalone-forced shred does not error: {:?}",
        out.diagnostics
    );
    // NO main provider — even though `tri` is an emitted (recursive) library def.
    assert!(
        out.artifacts.iter().all(|a| a.kind != "component-provider"),
        "EmitTestsShredStandalone emits NO main even with a library"
    );
    // Two self-contained per-test components, all valid; manifest main-file "" for both.
    let consumer_names: Vec<&str> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == crate::backend::Target::Wasm.artifact_kind())
        .map(|a| a.name.as_str())
        .collect();
    assert!(
        consumer_names.contains(&"test-t-a") && consumer_names.contains(&"test-t-b"),
        "self-contained per-test components: {consumer_names:?}"
    );
    for a in &out.artifacts {
        if a.kind == crate::backend::Target::Wasm.artifact_kind() {
            let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
            v.validate_all(&a.bytes)
                .unwrap_or_else(|e| panic!("standalone-forced shred `{}` validates: {e}", a.name));
        }
    }
    let manifest = out
        .artifacts
        .iter()
        .find(|a| a.kind == crate::sidecar::KIND_SHRED_MANIFEST)
        .expect("a shred-manifest artifact");
    let arena = crate::codec::decode(&manifest.bytes).expect("manifest decodes");
    let entries = arena
        .as_form(arena.root, "shred-manifest")
        .expect("(shred-manifest …)");
    assert_eq!(entries.len(), 2);
    for &e in entries {
        let f = arena.as_form(e, "entry").expect("(entry …)");
        assert_eq!(
            arena.as_str(f[6]).expect("main-file Str"),
            "",
            "standalone-forced: main-file empty (no --peer)"
        );
    }
}

#[test]
fn consumer_only_emits_the_closure_hash_sidecar() {
    // GATE-PERF (codegen-skip-on-HIT, v-compiler-perf ↔ v-cdz-tooling): `EmitTestsConsumerOnly` now
    // emits the closure-hash sidecar too (it used to be MISS/provider-path-only). This lets
    // `precompile_group` do the cache-HIT decision from ONE `EmitTestsConsumerOnly` drive — read this
    // hash, confirm the HIT — WITHOUT the expensive provider mono+codegen the composed path pays (the
    // dominant ~230s warm-once cost, which on a HIT only produces bytes that get DISCARDED). This pins
    // (1) ConsumerOnly emits exactly one closure-hash sidecar, (2) it emits NO provider (the whole
    // point — skip the codegen), (3) its hash VALUE equals the composed path's (the two-sided key must
    // agree, else a HIT confirmed off the ConsumerOnly hash would mismatch the persisted provider's key).
    let lib = crate::codec::encode(&parse(
        "(do \
             (def (alpha (: n Int64)) (if (= n 0) 0 (+ 2 (alpha (- n 1))))) \
             (def (beta (: n Int64)) (if (= n 0) 1 (+ 3 (beta (- n 1))))) \
             (export alpha) (export beta))",
    ));
    let app_a = crate::codec::encode(&parse(
        "(do (import \"lib\" (alpha)) (@ test (def (t-a) (if (= (alpha 3) 6) unit (trap \"x\")))))",
    ));
    let app_b = crate::codec::encode(&parse(
        "(do (import \"lib\" (beta)) (@ test (def (t-b) (if (= (beta 3) 10) unit (trap \"x\")))))",
    ));
    let drive = |req: Request| {
        let (lib, app_a, app_b) = (lib.clone(), app_a.clone(), app_b.clone());
        crate::host::run_with_compiler_stack(move || {
            crate::compile::compile(
                &[
                    Artifact::new(Artifact::KIND_AST, "lib", lib),
                    Artifact::new(Artifact::KIND_AST, "app-a", app_a),
                    Artifact::new(Artifact::KIND_AST, "app-b", app_b),
                    cadenza_compile_abi::abi::entry_artifact("app-a"),
                    Artifact::new(sidecar::KIND_SIDECAR, "drive", sidecar::encode(&[req])),
                ],
                &[],
            )
        })
    };
    let hash_of = |out: &crate::abi::CompileOutput| -> String {
        let hashes: Vec<&Artifact> = out
            .artifacts
            .iter()
            .filter(|a| a.kind == crate::sidecar::KIND_CLOSURE_HASH)
            .collect();
        assert_eq!(hashes.len(), 1, "exactly one closure-hash sidecar");
        // Canonical binary-AST wire (root `Ast.Int` u64) → the `{:016x}` cache-key form.
        format!(
            "{:016x}",
            cadenza_compile_abi::decode_closure_hash(&hashes[0].bytes)
                .expect("closure-hash decodes to a u64")
        )
    };

    let consumer = drive(Request::EmitTestsConsumerOnly);
    assert!(
        !consumer.has_error(),
        "consumer-only emit does not error: {:?}",
        consumer.diagnostics
    );
    // (2) NO provider on the consumer-only path — the codegen we skip.
    assert_eq!(
        consumer
            .artifacts
            .iter()
            .filter(|a| a.kind == "component-provider")
            .count(),
        0,
        "consumer-only emits NO provider (the skipped codegen)"
    );
    // (1) exactly one closure-hash sidecar (was zero before this fix).
    let consumer_hash = hash_of(&consumer);
    assert!(
        consumer_hash.len() == 16 && consumer_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "closure-hash is a 16-hex-digit u64: {consumer_hash:?}"
    );
    // (3) the ConsumerOnly hash EQUALS the composed (provider) path's — the two-sided key agreement, so
    // a HIT confirmed off this hash matches the persisted provider's key.
    let composed = drive(Request::EmitTestsComposed);
    assert!(!composed.has_error());
    assert_eq!(
        consumer_hash,
        hash_of(&composed),
        "the consumer-only closure-hash must equal the composed path's — else the cache HIT decision \
             would mismatch the persisted provider's key"
    );
}

#[test]
fn closure_hash_query_matches_the_composed_miss_path_sidecar() {
    // OPTION C provider-cache DECISION KEY: `Query::ClosureHash` returns the canonical shared-closure hash
    // (layout-only, no provider emit) — the value a runner keys a cache HIT on BEFORE emitting. It MUST
    // equal the `closure-hash` sidecar `EmitTestsComposed` emits on the MISS path (same fold, one
    // definition) — the drift-guard: the decision key and the persist key agree. Same 3-file fixture.
    let mk = |lib: &[u8], a: &[u8], b: &[u8], req: Request| {
        crate::host::run_with_compiler_stack(move || {
            crate::compile::compile(
                &[
                    Artifact::new(Artifact::KIND_AST, "lib", lib.to_vec()),
                    Artifact::new(Artifact::KIND_AST, "app-a", a.to_vec()),
                    Artifact::new(Artifact::KIND_AST, "app-b", b.to_vec()),
                    cadenza_compile_abi::abi::entry_artifact("app-a"),
                    Artifact::new(sidecar::KIND_SIDECAR, "drive", sidecar::encode(&[req])),
                ],
                &[],
            )
        })
    };
    let lib = crate::codec::encode(&parse(
        "(do \
             (def (alpha (: n Int64)) (if (= n 0) 0 (+ 2 (alpha (- n 1))))) \
             (def (beta (: n Int64)) (if (= n 0) 1 (+ 3 (beta (- n 1))))) \
             (export alpha) (export beta))",
    ));
    let app_a = crate::codec::encode(&parse(
        "(do (import \"lib\" (alpha)) (@ test (def (t-a) (if (= (alpha 3) 6) unit (trap \"x\")))))",
    ));
    let app_b = crate::codec::encode(&parse(
        "(do (import \"lib\" (beta)) (@ test (def (t-b) (if (= (beta 3) 10) unit (trap \"x\")))))",
    ));
    // The QUERY hash (layout-only, no emit).
    let q = mk(&lib, &app_a, &app_b, Request::Query(Query::ClosureHash));
    let q_hash = q
        .artifacts
        .iter()
        .find(|a| a.kind == crate::sidecar::KIND_CLOSURE_HASH)
        .map(|a| {
            format!(
                "{:016x}",
                cadenza_compile_abi::decode_closure_hash(&a.bytes)
                    .expect("closure-hash decodes to a u64")
            )
        })
        .expect("Query::ClosureHash returns a closure-hash artifact");
    assert!(
        q_hash.len() == 16 && q_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "closure-hash is a 16-hex-digit u64: {q_hash:?}"
    );
    // The MISS-path sidecar hash (from a full EmitTestsComposed).
    let e = mk(&lib, &app_a, &app_b, Request::EmitTestsComposed);
    let e_hash = e
        .artifacts
        .iter()
        .find(|a| a.kind == crate::sidecar::KIND_CLOSURE_HASH)
        .map(|a| {
            format!(
                "{:016x}",
                cadenza_compile_abi::decode_closure_hash(&a.bytes)
                    .expect("closure-hash decodes to a u64")
            )
        })
        .expect("EmitTestsComposed emits a closure-hash sidecar");
    // The DRIFT-GUARD: the decision-key hash == the persist-key hash (one canonical definition).
    assert_eq!(
        q_hash, e_hash,
        "Query::ClosureHash (decision key) must equal the EmitTestsComposed miss-path closure-hash \
             sidecar (persist key) — same fold, or the cache would key inconsistently"
    );
}

#[test]
fn emit_tests_consumer_only_emits_consumers_and_iface_but_no_provider() {
    // OPTION C provider-cache follow-on: `Request::EmitTestsConsumerOnly` emits the N per-file CONSUMER
    // components + the `component-name` iface sidecar, but SKIPS the `component-provider` emit (the caller
    // supplies a CACHED provider at run time). Same 3-file fixture as the composed witness; the only
    // difference from EmitTestsComposed is NO component-provider artifact. This is the single-file-verify
    // fast path — the consumer layout excludes the cross-edges and doesn't lower the shared closure.
    let lib = crate::codec::encode(&parse(
        "(do \
             (def (alpha (: n Int64)) (if (= n 0) 0 (+ 2 (alpha (- n 1))))) \
             (def (beta (: n Int64)) (if (= n 0) 1 (+ 3 (beta (- n 1))))) \
             (export alpha) (export beta))",
    ));
    let app_a = crate::codec::encode(&parse(
        "(do (import \"lib\" (alpha)) (@ test (def (t-a) (if (= (alpha 3) 6) unit (trap \"x\")))))",
    ));
    let app_b = crate::codec::encode(&parse(
        "(do (import \"lib\" (beta)) (@ test (def (t-b) (if (= (beta 3) 10) unit (trap \"x\")))))",
    ));
    let out = crate::host::run_with_compiler_stack(|| {
        crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "lib", lib.clone()),
                Artifact::new(Artifact::KIND_AST, "app-a", app_a.clone()),
                Artifact::new(Artifact::KIND_AST, "app-b", app_b.clone()),
                cadenza_compile_abi::abi::entry_artifact("app-a"),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::EmitTestsConsumerOnly]),
                ),
            ],
            &[],
        )
    });
    assert!(
        !out.has_error(),
        "consumer-only emit does not error: {:?}",
        out.diagnostics
    );
    // NO provider is emitted (the caller supplies the cached one) — the whole point vs EmitTestsComposed.
    let providers = out
        .artifacts
        .iter()
        .filter(|a| a.kind == "component-provider")
        .count();
    assert_eq!(
        providers,
        0,
        "consumer-only must NOT emit a component-provider (the cache supplies it): kinds={:?}",
        out.artifacts.iter().map(|a| &a.kind).collect::<Vec<_>>()
    );
    // The iface sidecar IS emitted (the runner needs it to pair the cached provider).
    let iface: Vec<&Artifact> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == crate::link::KIND_COMPONENT_NAME)
        .collect();
    assert_eq!(
        iface.len(),
        1,
        "the component-name iface sidecar is still emitted"
    );
    assert_eq!(
        cadenza_compile_abi::decode_name(&iface[0].bytes).unwrap(),
        "cadenza:closure/api"
    );
    // The closure-hash sidecar IS emitted on the consumer-only path (changed 2026-08-02, the
    // codegen-skip-on-HIT enabler): it lets `precompile_group` do the cache-HIT decision from ONE
    // `EmitTestsConsumerOnly` drive (read this hash, confirm the HIT) WITHOUT the expensive provider
    // mono+codegen the composed path pays — see `consumer_only_emits_the_closure_hash_sidecar` for the
    // full rationale + the value-agrees-with-composed pin. (Previously this path emitted no hash; the
    // caller was assumed to already have it, but that forced the composed/provider path just to obtain
    // the hash — the whole ~230s warm-once HIT cost.)
    assert_eq!(
        out.artifacts
            .iter()
            .filter(|a| a.kind == crate::sidecar::KIND_CLOSURE_HASH)
            .count(),
        1,
        "consumer-only now emits exactly one closure-hash (the HIT-decision key, no provider codegen)"
    );
    // The N consumer components ARE emitted, named by file link path, and validate as components that
    // IMPORT the closure iface (the same consumers EmitTestsComposed emits — only the provider is skipped).
    let consumer_names: Vec<&str> = out
        .artifacts
        .iter()
        .filter(|a| a.kind == crate::backend::Target::Wasm.artifact_kind())
        .map(|a| a.name.as_str())
        .collect();
    assert!(
        consumer_names.contains(&"app-a") && consumer_names.contains(&"app-b"),
        "one consumer per @test file (by link name): {consumer_names:?}"
    );
    for a in &out.artifacts {
        if a.kind == crate::backend::Target::Wasm.artifact_kind() {
            let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
            v.validate_all(&a.bytes)
                .unwrap_or_else(|e| panic!("consumer `{}` validates: {e}", a.name));
        }
    }
}

#[test]
fn emit_tests_composed_declines_a_same_stem_multi_dir_collision() {
    // STEM-COLLISION guard (pr881/pr888): `db.file_path` is the file's LINK path, and a runner demuxes the
    // N consumer components by the file's STEM (basename) — dir-blind, load-bearing for import resolution.
    // Two files whose STEMS collide (`a/t.cdz` + `b/t.cdz` across dirs → both stem `t`) would map two
    // consumers to one name, so the composed driver must DECLINE (fall back to per-file), NOT silently drop
    // one. pr888 caught the earlier guard as DEAD (it deduped full paths, always unique) + the earlier test
    // as VACUOUS (two ast artifacts both named `t` were rejected by the LINKER before the guard). This
    // version feeds DISTINCTLY-NAMED artifacts whose STEMS collide (`a/t`, `b/t`) — the linker accepts the
    // distinct names, so execution REACHES the guard, which fires on the shared stem `t`.
    let lib = crate::codec::encode(&parse(
        "(do (def (h (: n Int64)) (if (= n 0) 0 (+ 1 (h (- n 1))))) (export h))",
    ));
    let t1 = crate::codec::encode(&parse(
        "(do (import \"lib\" (h)) (@ test (def (a) (if (= (h 2) 2) unit (trap \"x\")))))",
    ));
    let t2 = crate::codec::encode(&parse(
        "(do (import \"lib\" (h)) (@ test (def (b) (if (= (h 3) 3) unit (trap \"x\")))))",
    ));
    let out = crate::host::run_with_compiler_stack(|| {
        crate::compile::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "lib", lib.clone()),
                // Two DISTINCT link names (`a/t`, `b/t`) that share the STEM `t` — the linker accepts the
                // distinct names (no dup-name reject), so the stem-collision reaches the composed guard.
                Artifact::new(Artifact::KIND_AST, "a/t", t1.clone()),
                Artifact::new(Artifact::KIND_AST, "b/t", t2.clone()),
                cadenza_compile_abi::abi::entry_artifact("a/t"),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::EmitTestsComposed]),
                ),
            ],
            &[],
        )
    });
    // The package must LINK (distinct names) — a link failure would make the assertion vacuous (pr888).
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message.contains("failed to decode")
                || d.message.contains("duplicate")
                || d.message.contains("entry")),
        "the two distinctly-named files must link (else the test is vacuous): {:?}",
        out.diagnostics
    );
    // No provider is emitted on the collision path — the guard declines (stem `t` collides) before hoisting.
    let providers = out
        .artifacts
        .iter()
        .filter(|a| a.kind == "component-provider")
        .count();
    assert_eq!(
        providers, 0,
        "a same-STEM multi-file build must NOT emit a composed provider (stem-collision guard declines)"
    );
    // And the guard's decline diagnostic is present (it REACHED the guard, not a linker reject upstream).
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.message.contains("DISTINCT import stem")
                || d.message.contains("sharing a stem")),
        "the stem-collision guard's decline must fire (proves the guard is reached, not dead): {:?}",
        out.diagnostics
    );
}

#[test]
fn option_c_composed_provider_from_union_edges_exports_every_files_cross_edge() {
    // OPTION C increment (c)(iii)b: `compute_provider_for_edges` builds the ONE shared-closure provider
    // for a composed `cdz test <dir>` build from the UNION cross-edge set (not one file's) — so the
    // provider EXPORTS every file's cross-edges. Same 3-file fixture as the union test: `lib` exports
    // alpha/beta; `app-a`'s @test calls alpha, `app-b`'s calls beta. The composed provider (over the
    // union {alpha, beta}) must export BOTH — where a single-file `compute_shared_closure_provider` (one
    // own_file) would export only that file's one edge. Also emits a valid component (provider path).
    use crate::testkit::parse;
    let lib = "(do \
                    (def (alpha (: n Int64)) (if (= n 0) 0 (+ 2 (alpha (- n 1))))) \
                    (def (beta (: n Int64)) (if (= n 0) 1 (+ 3 (beta (- n 1))))) \
                    (export alpha) (export beta))";
    let app_a = "(do (import \"lib\" (alpha)) \
                    (@ test (def (t-a) (if (= (alpha 3) 6) unit (trap \"x\")))))";
    let app_b = "(do (import \"lib\" (beta)) \
                    (@ test (def (t-b) (if (= (beta 3) 10) unit (trap \"x\")))))";
    let (export_names, component_valid): (Vec<String>, bool) =
        crate::host::run_with_compiler_stack(|| {
            let files: Vec<(String, crate::ast::Arenas)> = vec![
                ("lib".to_string(), parse(lib)),
                ("app-a".to_string(), parse(app_a)),
                ("app-b".to_string(), parse(app_b)),
            ];
            let linked = crate::link::link(&files, "app-a").expect("package links");
            let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
            let layout = crate::layout::compute_tests(&mut db).expect("@test build lays out");
            let mut files_seen: Vec<usize> = db
                .test_defs()
                .iter()
                .filter_map(|&t| db.file_of(db.defs[t].sig_occ))
                .collect();
            files_seen.sort_unstable();
            files_seen.dedup();
            let union = crate::layout::cross_component_edges_union(&mut db, &layout, &files_seen);
            let provider = crate::layout::compute_provider_for_edges(&mut db, &union)
                .expect("composed provider lays out over the union edge set");
            let names: Vec<String> = provider.exports.iter().map(|e| e.name.clone()).collect();
            db.component_name = Some("cadenza:closure/api".to_string());
            let bytes =
                crate::backend::emit(crate::backend::Target::Wasm, &mut db, &provider, None, None)
                    .expect("composed provider emits");
            let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
            let valid = v.validate_all(&bytes).is_ok();
            (names, valid)
        });
    // The composed provider exports BOTH cross-edges (a single-file provider would export only one).
    assert!(
        export_names.iter().any(|n| n == "alpha"),
        "composed provider exports alpha: {export_names:?}"
    );
    assert!(
        export_names.iter().any(|n| n == "beta"),
        "composed provider exports beta: {export_names:?}"
    );
    assert!(
        component_valid,
        "the composed provider emits a valid component"
    );
}

#[test]
fn option_c_provider_layout_exports_the_cross_edge_and_closes_its_body() {
    // OPTION C increment (b)(ii): `layout::compute_shared_closure_provider` builds the SHARED-CLOSURE
    // provider layout — its EXPORTS are the cross-component edges (the shared defs the @tests call), and
    // `finish_layout` closes reachability so an edge's own intra-closure callees are emitted INSIDE the
    // provider. Same 2-file fixture: `app`'s @test calls `lib`'s recursive `shared-helper`, so the
    // provider layout must EXPORT `shared-helper` (the one cross-edge) and its `order` must include it.
    use crate::testkit::parse;
    let lib = "(do (def (shared-helper (: n Int64)) (if (= n 0) 0 (+ 1 (shared-helper (- n 1))))) \
                    (export shared-helper))";
    let app = "(do (import \"lib\" (shared-helper)) \
                    (@ test (def (t-app) (if (= (shared-helper 5) 5) unit (trap \"x\")))))";
    let (export_names, order_has_helper): (Vec<String>, bool) =
        crate::host::run_with_compiler_stack(|| {
            let files: Vec<(String, crate::ast::Arenas)> = vec![
                ("lib".to_string(), parse(lib)),
                ("app".to_string(), parse(app)),
            ];
            let linked = crate::link::link(&files, "app").expect("package links");
            let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
            let test_layout =
                crate::layout::compute_tests(&mut db).expect("the @test build lays out");
            let test_def = *db.test_defs().first().expect("one @test");
            let own_file = db
                .file_of(db.defs[test_def].sig_occ)
                .expect("the @test def is in a file");
            let provider =
                crate::layout::compute_shared_closure_provider(&mut db, &test_layout, own_file)
                    .expect("the shared-closure provider lays out");
            (
                provider.exports.iter().map(|e| e.name.clone()).collect(),
                provider
                    .order
                    .iter()
                    .any(|&d| db.defs[d].name.starts_with("shared-helper")),
            )
        });
    assert!(
        export_names.iter().any(|n| n.starts_with("shared-helper")),
        "the provider EXPORTS the cross-edge shared-helper: exports={export_names:?}"
    );
    assert!(
        order_has_helper,
        "the provider's reachable order includes shared-helper's body"
    );
}

#[test]
fn source_boundary_name_strips_the_transform_suffix() {
    // The shared provider↔consumer boundary-name contract: a transformed def's emitted name carries a
    // `$acc` (accumulator rewrite) or `#monoN` (monomorphization) suffix — invalid in a component extern
    // name and not the stable source name. `source_boundary_name` returns the base before the first
    // `$`/`#`; a plain name is unchanged. Both Option-C provider EXPORT and consumer IMPORT use it.
    use crate::layout::source_boundary_name;
    assert_eq!(source_boundary_name("shared-helper"), "shared-helper");
    assert_eq!(source_boundary_name("shared-helper$acc"), "shared-helper");
    assert_eq!(source_boundary_name("fac#mono7"), "fac");
    assert_eq!(
        source_boundary_name("f$acc#mono3"),
        "f",
        "strips at the FIRST marker (both suffixes)"
    );
    assert_eq!(source_boundary_name(""), "");
}

#[test]
fn boundary_export_names_disambiguate_two_specializations_of_one_base() {
    // REGRESSION (invalid-wasm on spine growth, v-compiler-ml lazy-Db-via-effects). The effect group
    // fold specializes a recursive performer per handler-context, minting `{base}#eff{N}`; a query
    // group's mutual performer (`type-of`) can specialize MORE THAN ONCE (distinct call-shapes), so two
    // specs `type-of#eff529`/`type-of#eff531` both cross the Option-C shared-closure provider boundary.
    // `source_boundary_name` strips the `#effN` suffix, so BOTH stripped to bare `type-of` → the provider
    // exported `type-of` TWICE → a duplicate component export name → invalid wasm ("failed to parse
    // WebAssembly module", caught only at load). `boundary_export_names` disambiguates the 2nd+ collision
    // with a letter-led kebab suffix so every export is unique AND a valid extern name, and derives it as
    // a pure function of the ordered edge slice so provider export == consumer import at each position.
    use crate::db::Def;
    let mut db = crate::db::Db::load(parse("(module m (def (main) 0) (export main))"));
    let push = |db: &mut crate::db::Db, name: &str| -> usize {
        let sig_name = db.push_name(name);
        let sig = db.push_list(vec![sig_name]);
        let body = db.push_name("0");
        db.defs.push(Def {
            name: name.to_string(),
            sig_occ: sig,
            params: Vec::new(),
            body: Some(body),
            internal: false,
        });
        db.defs.len() - 1
    };
    // Two specializations of `type-of` + one unique helper + an accumulator-rewritten `f$acc`.
    let e0 = push(&mut db, "type-of#eff529");
    let e1 = push(&mut db, "type-of#eff531");
    let e2 = push(&mut db, "cache-type#eff530");
    let e3 = push(&mut db, "helper$acc");
    let e4 = push(&mut db, "type-of#eff533"); // a THIRD spec of the same base
    let edges = vec![e0, e1, e2, e3, e4];
    let names = crate::layout::boundary_export_names(&db, &edges);
    assert_eq!(
        names,
        vec![
            "type-of".to_string(),
            "type-of-dup2".to_string(),
            "cache-type".to_string(),
            "helper".to_string(),
            "type-of-dup3".to_string(),
        ],
        "distinct specs of one base must get UNIQUE boundary names; a unique base is unchanged"
    );
    // The names must be unique (the invariant the wasm boundary requires) and derivation-stable: the same
    // ordered slice yields the same names (provider export order == consumer import order by construction).
    let n2 = crate::layout::boundary_export_names(&db, &edges);
    assert_eq!(names, n2, "derivation is a pure function of the edge slice");
    let unique: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "every boundary export name is unique"
    );
}

#[test]
fn def_content_hash_is_invariant_to_the_monomorph_mint_counter() {
    // REGRESSION (gate perf, v-cdz-tooling coordination): a monomorphized specialization mints its name
    // `{base}#mono{db.defs.len()}` (lower.rs), and `db.defs.len()` at mint time is RUN-VARYING. Two
    // things then leaked that run-varying counter into `sidecar::def_content_hash` — the shared-closure
    // provider-cache KEY (`closure_content_hash`) folds it — so the SAME specialization hashed
    // DIFFERENTLY each run → its `.provider.wasm` cache never hit → re-emitted (full lower) on every
    // `--warm-only`, the gate's ~256s. The two leaks, both fixed:
    //   (1) the def's own `#mono{N}` head NAME + body references to sibling `#mono{N}` names — now
    //       desuffixed to `source_boundary_name` when hashing a `Name` leaf;
    //   (2) the `decl.0` arena StructId baked as an Int leaf into an encoded `(Sum NAME <decl> …)` /
    //       `(Nominal NAME <decl> …)` type-expr in the body — now SKIPPED (child index 2) for those two
    //       shapes.
    // This pins BOTH: two defs identical up to the mint counter (in the name AND in a body Sum/Nominal
    // decl-id) MUST hash the same. Fixture-free — builds the two defs' subtrees directly in a Db arena,
    // no CLI / provider-cache path (that path only reproduces at compiler-ml's ~8min scale).
    use crate::ast::{IntValue, Leaf, Radix};
    use crate::db::Def;

    // Build one def named `{name}#mono{n}` whose body reads a field off a `(Nominal Foo <decl_id> (args) Unit)`
    // type-expr — so BOTH run-varying inputs (the mono name suffix AND the decl arena-id) are present.
    let make_def = |db: &mut crate::db::Db, mint_n: usize, decl_id: i64| -> usize {
        let sig_name = db.push_name(&format!("f#mono{mint_n}"));
        let sig = db.push_list(vec![sig_name]); // nullary sig `(f#monoN)`
        // body: `(typeval (Nominal Foo <decl_id> (args) Unit))` — a mono'd type-expr carrying the decl id.
        let tv_head = db.push_name("typeval");
        let nom = db.push_name("Nominal");
        let foo = db.push_name("Foo");
        let decl = db.push_atom(Leaf::Int {
            value: IntValue::from_i64(decl_id),
            radix: Radix::Dec,
        });
        let args_head = db.push_name("args");
        let args = db.push_list(vec![args_head]);
        let unit = db.push_name("Unit");
        let nominal = db.push_list(vec![nom, foo, decl, args, unit]);
        let body = db.push_list(vec![tv_head, nominal]);
        db.defs.push(Def {
            name: format!("f#mono{mint_n}"),
            sig_occ: sig,
            params: Vec::new(),
            body: Some(body),
            internal: false,
        });
        db.defs.len() - 1
    };

    let mut db = crate::db::Db::load(parse("(module m (def (main) 0) (export main))"));
    // Same specialization, but minted at DIFFERENT counters (3 vs 99) AND with a DIFFERENT arena decl-id
    // (100 vs 200) — mimicking two runs where mono order shifted both.
    let a = make_def(&mut db, 3, 100);
    let b = make_def(&mut db, 99, 200);
    assert_eq!(
        crate::sidecar::def_content_hash(&db, a),
        crate::sidecar::def_content_hash(&db, b),
        "def_content_hash must be INVARIANT to the run-varying #mono mint counter (name) AND the arena \
             decl-id (Sum/Nominal type-expr) — else a mono'd shared closure never cache-hits across runs"
    );

    // NEGATIVE guard: two GENUINELY-DIFFERENT specializations (different Nominal NAME) must still DIFFER,
    // so the desuffix/skip doesn't collapse distinct closures into one cache key.
    let sig_name = db.push_name("f#mono5");
    let sig = db.push_list(vec![sig_name]);
    let tv_head = db.push_name("typeval");
    let nom = db.push_name("Nominal");
    let bar = db.push_name("Bar"); // different nominal name
    let decl = db.push_atom(Leaf::Int {
        value: IntValue::from_i64(100),
        radix: Radix::Dec,
    });
    let args_head = db.push_name("args");
    let args = db.push_list(vec![args_head]);
    let unit = db.push_name("Unit");
    let nominal = db.push_list(vec![nom, bar, decl, args, unit]);
    let body = db.push_list(vec![tv_head, nominal]);
    db.defs.push(Def {
        name: "f#mono5".to_string(),
        sig_occ: sig,
        params: Vec::new(),
        body: Some(body),
        internal: false,
    });
    let c = db.defs.len() - 1;
    assert_ne!(
        crate::sidecar::def_content_hash(&db, a),
        crate::sidecar::def_content_hash(&db, c),
        "a DIFFERENT specialization (Nominal Foo vs Bar) must still hash differently — the fix strips the \
             run-varying counter/decl-id, NOT the semantic content (name/args/inner still distinguish)"
    );
}

#[test]
fn option_c_cross_edge_import_shift_offsets_positions_for_a_coexisting_peer_extern() {
    // PR#882 CORRECTNESS fix: `compute_tests_consumer` computes `cross_edge_import` positions 0-based (the
    // consumer layout carries no other extern imports at layout time). But the backend emit prepends a
    // PEER-BOUND escaping effect's extern imports FIRST (`db.effect_bindings`), so the cross-edge block
    // lands at `delta..delta+M` in the final `extern_order`, not `0..M` — and a `Lir::CallExternImport(pos)`
    // resolves against that FINAL order. `with_cross_edge_import_shift(delta)` reconciles the map to the
    // final positions. Without it, a consumer that BOTH imports the shared closure AND binds a peer effect
    // emits every cross-edge call off by `delta` → wrong import / invalid module. Here: build a 2-cross-edge
    // consumer layout (positions {0,1}), shift by delta=2 (as if 2 peer-effect externs preceded), assert the
    // map is now {2,3} and extern_order is untouched (the ORDER list is rebuilt by the backend from the full
    // extern_imports vector; the shift only moves the def→final-pos map select reads).
    use crate::testkit::parse;
    let lib = "(do \
                    (def (alpha (: n Int64)) (if (= n 0) 0 (+ 2 (alpha (- n 1))))) \
                    (def (beta (: n Int64)) (if (= n 0) 1 (+ 3 (beta (- n 1))))) \
                    (export alpha) (export beta))";
    let app = "(do (import \"lib\" (alpha beta)) \
                    (@ test (def (t-two) (if (= (+ (alpha 3) (beta 3)) 16) unit (trap \"x\")))))";
    let (base_positions, shifted_positions, extern_len_before, extern_len_after): (
        Vec<usize>,
        Vec<usize>,
        usize,
        usize,
    ) = crate::host::run_with_compiler_stack(|| {
        let files: Vec<(String, crate::ast::Arenas)> = vec![
            ("lib".to_string(), parse(lib)),
            ("app".to_string(), parse(app)),
        ];
        let linked = crate::link::link(&files, "app").expect("package links");
        let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
        let test_layout = crate::layout::compute_tests(&mut db).expect("@test build lays out");
        let test_def = *db.test_defs().first().expect("one @test");
        let own_file = db
            .file_of(db.defs[test_def].sig_occ)
            .expect("test def in a file");
        let provider_edges = crate::layout::cross_component_edges(&mut db, &test_layout, own_file);
        let tds = db.test_defs();
        let consumer = crate::layout::compute_tests_consumer(
            &mut db,
            &tds,
            &provider_edges,
            "cadenza:closure/api",
        )
        .expect("consumer lays out");
        let mut base: Vec<usize> = consumer.cross_edge_import.values().copied().collect();
        base.sort_unstable();
        let shifted_layout = consumer.with_cross_edge_import_shift(2);
        let mut shifted: Vec<usize> = shifted_layout.cross_edge_import.values().copied().collect();
        shifted.sort_unstable();
        (
            base,
            shifted,
            consumer.extern_order.len(),
            shifted_layout.extern_order.len(),
        )
    });
    assert_eq!(
        base_positions,
        vec![0, 1],
        "the consumer computes cross-edge positions 0-based at layout time"
    );
    assert_eq!(
        shifted_positions,
        vec![2, 3],
        "shift(2) offsets every cross-edge position by the peer-extern count (delta)"
    );
    assert_eq!(
        extern_len_before, extern_len_after,
        "the shift moves only the def→final-pos map, never extern_order (the backend rebuilds order)"
    );
}

#[test]
fn option_c_cross_edge_import_shift_zero_is_a_noop() {
    // The common consumer (no coexisting peer-bound effect) shifts by delta=0 → byte-identical map, so the
    // fix never perturbs the ordinary consumer emit. A trivial but load-bearing guard: delta=0 must be a
    // no-op or every existing consumer emit would regress.
    use crate::testkit::parse;
    let lib = "(do (def (shared-helper (: n Int64)) (if (= n 0) 0 (+ 1 (shared-helper (- n 1))))) \
                    (export shared-helper))";
    let app = "(do (import \"lib\" (shared-helper)) \
                    (@ test (def (t-app) (if (= (shared-helper 5) 5) unit (trap \"x\")))))";
    let same: bool = crate::host::run_with_compiler_stack(|| {
        let files: Vec<(String, crate::ast::Arenas)> = vec![
            ("lib".to_string(), parse(lib)),
            ("app".to_string(), parse(app)),
        ];
        let linked = crate::link::link(&files, "app").expect("package links");
        let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
        let test_layout = crate::layout::compute_tests(&mut db).expect("@test build lays out");
        let test_def = *db.test_defs().first().expect("one @test");
        let own_file = db
            .file_of(db.defs[test_def].sig_occ)
            .expect("test def in a file");
        let provider_edges = crate::layout::cross_component_edges(&mut db, &test_layout, own_file);
        let tds = db.test_defs();
        let consumer = crate::layout::compute_tests_consumer(
            &mut db,
            &tds,
            &provider_edges,
            "cadenza:closure/api",
        )
        .expect("consumer lays out");
        let shifted = consumer.with_cross_edge_import_shift(0);
        consumer.cross_edge_import == shifted.cross_edge_import
    });
    assert!(
        same,
        "shift(0) must be a no-op on the cross_edge_import map"
    );
}

#[test]
fn option_c_shared_closure_provider_emits_a_valid_component() {
    // OPTION C increment (b)(iii): the shared-closure provider layout, run through the PROVIDER emit
    // (db.component_name set), emits a VALID wasm component exporting the cross-edge interface. The
    // recursive cross-edge `shared-helper` emits internally as `shared-helper$acc` (accumulator rewrite),
    // but the provider names the boundary by the SOURCE name (layout::source_boundary_name strips the
    // `$acc`/`#mono` suffix), so the extern name is valid kebab `shared-helper` — no `$`.
    use crate::testkit::parse;
    let lib = "(do (def (shared-helper (: n Int64)) (if (= n 0) 0 (+ 1 (shared-helper (- n 1))))) \
                    (export shared-helper))";
    let app = "(do (import \"lib\" (shared-helper)) \
                    (@ test (def (t-app) (if (= (shared-helper 5) 5) unit (trap \"x\")))))";
    let provider_bytes: Vec<u8> = crate::host::run_with_compiler_stack(|| {
        let files: Vec<(String, crate::ast::Arenas)> = vec![
            ("lib".to_string(), parse(lib)),
            ("app".to_string(), parse(app)),
        ];
        let linked = crate::link::link(&files, "app").expect("package links");
        let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
        let test_layout = crate::layout::compute_tests(&mut db).expect("the @test build lays out");
        let test_def = *db.test_defs().first().expect("one @test");
        let own_file = db
            .file_of(db.defs[test_def].sig_occ)
            .expect("the @test def is in a file");
        let provider_layout =
            crate::layout::compute_shared_closure_provider(&mut db, &test_layout, own_file)
                .expect("the shared-closure provider lays out");
        db.component_name = Some("cadenza:closure/api".to_string());
        crate::backend::emit(
            crate::backend::Target::Wasm,
            &mut db,
            &provider_layout,
            None,
            None,
        )
        .expect("the shared-closure provider emits")
    });
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(&provider_bytes)
        .expect("the shared-closure provider component validates");
}

#[test]
fn option_c_consumer_layout_excludes_the_cross_edge_from_its_emission_set() {
    // OPTION C increment (c) layout-side: compute_tests_consumer lays out a per-file @test component that
    // EXCLUDES the cross-edge shared defs from `order` (they live in the provider component) + reports
    // them as `boundary_hits` (→ the consumer's extern imports). Same 2-file fixture: app's @test calls
    // lib's recursive shared-helper; the consumer layout must NOT emit shared-helper (it's a cross-edge)
    // but MUST hit it (record it for the extern import), while still emitting app's own @test def.
    use crate::testkit::parse;
    let lib = "(do (def (shared-helper (: n Int64)) (if (= n 0) 0 (+ 1 (shared-helper (- n 1))))) \
                    (export shared-helper))";
    let app = "(do (import \"lib\" (shared-helper)) \
                    (@ test (def (t-app) (if (= (shared-helper 5) 5) unit (trap \"x\")))))";
    let (order_names, mapped_names, extern_ops): (Vec<String>, Vec<String>, Vec<String>) =
        crate::host::run_with_compiler_stack(|| {
            let files: Vec<(String, crate::ast::Arenas)> = vec![
                ("lib".to_string(), parse(lib)),
                ("app".to_string(), parse(app)),
            ];
            let linked = crate::link::link(&files, "app").expect("package links");
            let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
            let test_layout = crate::layout::compute_tests(&mut db).expect("@test build lays out");
            let test_def = *db.test_defs().first().expect("one @test");
            let own_file = db
                .file_of(db.defs[test_def].sig_occ)
                .expect("test def in a file");
            // The provider's canonical export edge order (= cross_component_edges' layout.order order).
            let provider_edges =
                crate::layout::cross_component_edges(&mut db, &test_layout, own_file);
            let tds = db.test_defs();
            let consumer = crate::layout::compute_tests_consumer(
                &mut db,
                &tds,
                &provider_edges,
                "cadenza:closure/api",
            )
            .expect("consumer lays out");
            // Which cross-edge defs are mapped as extern imports, + their extern_order op names.
            let mapped: Vec<String> = consumer
                .cross_edge_import
                .keys()
                .map(|&d| db.defs[d].name.clone())
                .collect();
            let extern_ops: Vec<String> = consumer
                .extern_order
                .iter()
                .map(|(_, op)| op.clone())
                .collect();
            (
                consumer
                    .order
                    .iter()
                    .map(|&d| db.defs[d].name.clone())
                    .collect(),
                mapped,
                extern_ops,
            )
        });
    // The cross-edge shared-helper is EXCLUDED from the consumer's emitted `order`…
    assert!(
        !order_names.iter().any(|n| n.starts_with("shared-helper")),
        "the consumer must NOT emit the cross-edge shared-helper: order={order_names:?}"
    );
    // …is in the cross_edge_import map (→ select emits a CallExternImport for it)…
    assert!(
        mapped_names.iter().any(|n| n.starts_with("shared-helper")),
        "the cross-edge shared-helper is mapped as an extern import: mapped={mapped_names:?}"
    );
    // …and its extern_order op name is the SOURCE name (kebab), not the $acc transform name.
    assert!(
        extern_ops.iter().any(|op| op == "shared-helper"),
        "the extern import op is the source boundary name shared-helper (not $acc): extern_ops={extern_ops:?}"
    );
    // …and the @test def itself IS still emitted.
    assert!(
        order_names.iter().any(|n| n.starts_with("t-app")),
        "the consumer still emits its own @test def: order={order_names:?}"
    );
}

#[test]
fn option_c_consumer_component_emits_a_valid_component_importing_the_cross_edge() {
    // OPTION C increment (c)(ii-c): the CONSUMER layout, run through `backend::emit`, produces a VALID
    // wasm component that IMPORTS the shared cross-edge as a peer interface func (`extern_imports` built
    // from `layout.cross_edge_import` in `mod::emit`) — the emit end-to-end that FIRES the select
    // `CallExternImport` branch. Same 2-file fixture: app's @test calls lib's recursive shared-helper, so
    // the consumer emits its own @test def calling shared-helper as an IMPORT, not a local func. The
    // component must validate (its import's functype must match what the provider EXPORTS — the ABI-
    // agreement twin of the index-agreement the consumer's `cross_edge_import` already fixes).
    use crate::testkit::parse;
    let lib = "(do (def (shared-helper (: n Int64)) (if (= n 0) 0 (+ 1 (shared-helper (- n 1))))) \
                    (export shared-helper))";
    let app = "(do (import \"lib\" (shared-helper)) \
                    (@ test (def (t-app) (if (= (shared-helper 5) 5) unit (trap \"x\")))))";
    let consumer_bytes: Vec<u8> = crate::host::run_with_compiler_stack(|| {
        let files: Vec<(String, crate::ast::Arenas)> = vec![
            ("lib".to_string(), parse(lib)),
            ("app".to_string(), parse(app)),
        ];
        let linked = crate::link::link(&files, "app").expect("package links");
        let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
        let test_layout = crate::layout::compute_tests(&mut db).expect("the @test build lays out");
        let test_def = *db.test_defs().first().expect("one @test");
        let own_file = db
            .file_of(db.defs[test_def].sig_occ)
            .expect("the @test def is in a file");
        let provider_edges = crate::layout::cross_component_edges(&mut db, &test_layout, own_file);
        let tds = db.test_defs();
        let consumer = crate::layout::compute_tests_consumer(
            &mut db,
            &tds,
            &provider_edges,
            "cadenza:closure/api",
        )
        .expect("the consumer lays out");
        crate::backend::emit(crate::backend::Target::Wasm, &mut db, &consumer, None, None)
            .expect("the consumer emits")
    });
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(&consumer_bytes)
        .expect("the Option-C consumer component validates");
}

#[test]
fn option_c_two_cross_edges_agree_provider_export_and_consumer_import_index() {
    // OPTION C increment (c)(ii-d): the MULTI-cross-edge witness — the one that catches a SHIFTED-INDEX
    // invalid module (v-wasm-opt FINDING #22 family: consumer import index ≠ provider export order). A
    // single cross-edge can't expose an ordering bug (index 0 == 0 trivially); TWO independent shared
    // defs can. Fixture: `lib` exports two recursive helpers `alpha`/`beta`; `app`'s @test calls BOTH, so
    // there are two cross-edges. We assert (1) the PROVIDER exports them in `cross_component_edges` order,
    // (2) the CONSUMER maps each cross-edge def to EXACTLY its provider-export position (import idx ==
    // export idx per def — the index-agreement invariant), and (3) BOTH the provider and consumer
    // components VALIDATE. The two helpers have DIFFERENT ARITIES (alpha unary, beta binary) so a SWAPPED
    // import index (alpha's call resolving to beta's slot) would fail component validation on the functype
    // mismatch — the witness catches a shifted index at the EMIT level, not only the layout-map assertion.
    use crate::testkit::parse;
    let lib = "(do \
                    (def (alpha (: n Int64)) (if (= n 0) 0 (+ 2 (alpha (- n 1))))) \
                    (def (beta (: a Int64) (: b Int64)) (if (= a 0) b (+ 3 (beta (- a 1) b)))) \
                    (export alpha) (export beta))";
    let app = "(do (import \"lib\" (alpha beta)) \
                    (@ test (def (t-two) (if (= (+ (alpha 3) (beta 3 1)) 16) unit (trap \"x\")))))";
    struct Out {
        provider_export_order: Vec<String>,
        consumer_import_at: Vec<(String, usize)>,
        provider_bytes: Vec<u8>,
        consumer_bytes: Vec<u8>,
    }
    let out: Out = crate::host::run_with_compiler_stack(|| {
        let files: Vec<(String, crate::ast::Arenas)> = vec![
            ("lib".to_string(), parse(lib)),
            ("app".to_string(), parse(app)),
        ];
        let linked = crate::link::link(&files, "app").expect("package links");
        let mut db = crate::db::Db::load_linked(linked.arenas.clone(), Some(linked.linkage()));
        let test_layout = crate::layout::compute_tests(&mut db).expect("the @test build lays out");
        let test_def = *db.test_defs().first().expect("one @test");
        let own_file = db
            .file_of(db.defs[test_def].sig_occ)
            .expect("the @test def is in a file");
        // The canonical cross-edge order = the provider's EXPORT order (both from `cross_component_edges`).
        let provider_edges = crate::layout::cross_component_edges(&mut db, &test_layout, own_file);
        let provider_export_order: Vec<String> = provider_edges
            .iter()
            .map(|&d| crate::layout::source_boundary_name(&db.defs[d].name).to_string())
            .collect();
        // The consumer layout — each cross-edge def → its extern import position.
        let tds = db.test_defs();
        let consumer = crate::layout::compute_tests_consumer(
            &mut db,
            &tds,
            &provider_edges,
            "cadenza:closure/api",
        )
        .expect("the consumer lays out");
        // For each cross-edge def, its (source-name, consumer import position).
        let mut consumer_import_at: Vec<(String, usize)> = consumer
            .cross_edge_import
            .iter()
            .map(|(&d, &pos)| {
                (
                    crate::layout::source_boundary_name(&db.defs[d].name).to_string(),
                    pos,
                )
            })
            .collect();
        consumer_import_at.sort_by_key(|(_, pos)| *pos);
        let consumer_bytes =
            crate::backend::emit(crate::backend::Target::Wasm, &mut db, &consumer, None, None)
                .expect("the consumer emits");
        let provider_layout =
            crate::layout::compute_shared_closure_provider(&mut db, &test_layout, own_file)
                .expect("the shared-closure provider lays out");
        db.component_name = Some("cadenza:closure/api".to_string());
        let provider_bytes = crate::backend::emit(
            crate::backend::Target::Wasm,
            &mut db,
            &provider_layout,
            None,
            None,
        )
        .expect("the provider emits");
        Out {
            provider_export_order,
            consumer_import_at,
            provider_bytes,
            consumer_bytes,
        }
    });
    // Two cross-edges → two exports, two imports.
    assert_eq!(
        out.provider_export_order.len(),
        2,
        "the @test calls two shared defs → two cross-edges: {:?}",
        out.provider_export_order
    );
    // INDEX-AGREEMENT: the consumer's import position for each cross-edge == its provider export index.
    // (Both derived from `cross_component_edges`; a re-derivation that reordered would break this.)
    let import_names_in_order: Vec<String> = out
        .consumer_import_at
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    assert_eq!(
        import_names_in_order, out.provider_export_order,
        "consumer import index order must MATCH provider export order (the FINDING #22 \
             shifted-index invariant): imports={import_names_in_order:?} exports={:?}",
        out.provider_export_order
    );
    // Both components must VALIDATE — a mismatched import functype/index fails component validation.
    let mut vp = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    vp.validate_all(&out.provider_bytes)
        .expect("the two-export provider component validates");
    let mut vc = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    vc.validate_all(&out.consumer_bytes)
        .expect("the two-import consumer component validates");
}

#[test]
fn emit_tests_declines_a_non_scalar_property_param() {
    // A property-test parameter must be a boundary-representable scalar (the runner generates + passes
    // it). A param with no such type — an unannotated one inference cannot fix to a scalar — declines
    // with the annotate-it guidance `export_params` gives, rather than emitting an uncallable export.
    let src = "(do (effect Test (op fail (-> String Unit))) \
                    (@ test (def (prop x) x))) ";
    let out = compile(&inputs(src, &[Request::EmitTests]), &[]);
    assert!(
        out.has_error(),
        "a non-representable property param must decline, not emit an uncallable export"
    );
}

#[test]
fn emit_tests_declines_a_digit_led_kebab_segment_name() {
    // REGRESSION (v-iterators, 2026-07-15): a `@test` (or any component-boundary export) name with a
    // HYPHEN-DELIMITED SEGMENT STARTING WITH A DIGIT (`step-by-2`, `a-2-b`, `range-step-2x`) is a valid
    // Cadenza identifier but NOT a valid component-model kebab word — `wasmparser`'s `KebabStr` requires
    // each `-`-delimited label to start with a letter. `kebab_extern_name` keeps `-`/digits verbatim, so
    // it normalizes such a name to ITSELF (an invalid extern name), and emitting it produced a component
    // wasmtime rejects WHOLESALE at load — every test in the file reported "fail" / the artifact was
    // unloadable, with NO compiler diagnostic (the [[rcdzc-kebab-extern-name-gotcha]] family). It is now
    // a clear compile-time CDZ0201 naming the offending name, before emit.
    for name in ["step-by-2", "a-2-b", "range-step-2x", "step-2"] {
        let src = format!("(do (@ test (def ({name}) unit))) ");
        let out = compile(&inputs(&src, &[Request::EmitTests]), &[]);
        assert!(
            out.has_error(),
            "a @test named `{name}` (a digit-led kebab segment) must DECLINE, not silently emit an \
                 invalid component: {:?}",
            out.diagnostics
        );
        let boundary_diag = out.diagnostics.iter().find(|d| {
            d.code.as_deref() == Some("CDZ0201")
                && d.message.contains("valid component boundary name")
        });
        assert!(
            boundary_diag.is_some(),
            "the decline for `{name}` is the coded boundary-name CDZ0201: {:?}",
            out.diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
        // The diagnostic ANCHORS at the offending `@test`/export def (its `sig_occ`), so a consumer that
        // holds the span table points the reader AT the name to rename — not an unanchored bare message
        // (v-guide-infra's actionable-diagnostic ask, 2026-07-17: the guide now teaches `@test` authoring,
        // so a numeric-segment name must point at the name + say how to fix it).
        assert!(
            boundary_diag.unwrap().node.is_some(),
            "the boundary-name reject for `{name}` carries a source anchor (points at the @test name)"
        );
    }
}

#[test]
fn emit_tests_accepts_a_digit_inside_a_word_segment() {
    // NO REGRESSION: a digit INSIDE a word (`step2`, `range-step-by2`) or a trailing digit on a word
    // (`f2`, `call0`) IS a valid kebab word — the guard rejects only a digit that STARTS a `-`-delimited
    // segment. These names must still emit a test component.
    for name in ["step2", "range-step-by2", "f2"] {
        let src = format!("(do (@ test (def ({name}) unit))) ");
        let out = compile(&inputs(&src, &[Request::EmitTests]), &[]);
        assert!(
            !out.has_error() && out.artifacts.iter().any(|a| a.kind == "component"),
            "a @test named `{name}` (digit inside/after a word) must EMIT: {:?}",
            out.diagnostics
        );
    }
}

#[test]
fn emit_tests_declines_a_non_ascii_export_name_with_a_cause_specific_diagnostic() {
    // REGRESSION (concierge assign, 2026-07-16; v-syntax found+characterized): Cadenza's ML lexer
    // admits UNICODE idents (`def π`, `def café`, `a·b`), but a component extern name is ASCII kebab
    // only (`[a-z0-9-]`, `wasmparser`'s `KebabStr`). `kebab_extern_name` keeps a non-ASCII char VERBATIM,
    // so it fails `is_kebab_word` and (before the guard) emitted a component wasmtime rejects wholesale
    // at load — no compiler diagnostic. The `invalid_kebab_export_name` guard already rejects it; this
    // pins that the diagnostic is CAUSE-SPECIFIC: it names the offending NON-ASCII CHARACTER and points
    // at ASCII-kebab renaming, rather than the (wrong-for-this-case) "segment must start with a letter"
    // message — which would be actively confusing for `café` (which DOES start with a letter).
    for (name, bad) in [("π", "π"), ("café", "é"), ("a·b", "·"), ("ναμε", "ν")] {
        let src = format!("(do (@ test (def ({name}) unit))) ");
        let out = compile(&inputs(&src, &[Request::EmitTests]), &[]);
        assert!(
            out.has_error(),
            "a @test named `{name}` (non-ASCII) must DECLINE, not silently emit an unloadable component: {:?}",
            out.diagnostics
        );
        let d = out
            .diagnostics
            .iter()
            .find(|d| {
                d.code.as_deref() == Some("CDZ0201")
                    && d.message.contains("valid component boundary name")
            })
            .unwrap_or_else(|| {
                panic!(
                    "the decline for `{name}` is the coded boundary-name CDZ0201: {:?}",
                    out.diagnostics
                        .iter()
                        .map(|d| &d.message)
                        .collect::<Vec<_>>()
                )
            });
        // Cause-specific: names the bad char + points at ASCII kebab, NOT the digit-led "start with a letter" text.
        assert!(
            d.message.contains(&format!("it contains `{bad}`"))
                && d.message.contains("ASCII kebab-case only")
                && !d.message.contains("START WITH A LETTER"),
            "the decline for `{name}` must name the non-ASCII char `{bad}` + point at ASCII kebab \
                 (not the digit-led message): {}",
            d.message
        );
    }
}

#[test]
fn the_copied_interface_name_validator_agrees_with_cadenza_syntax_over_a_fuzz_corpus() {
    // COPY-INVARIANT GUARD. `cadenza-syntax` is a DEV-only dependency for the pure-lib core, so the
    // wasm backend keeps its OWN copy of the peer-BINDING validator
    // (`crate::backend::common::export_name::is_valid_interface_name`) rather than call the reference at emit time —
    // this is the guard that turns a silent invalid-component miscompile (an author's malformed
    // `(bind E "ns:pkg/iface")` string) into a compile-time CDZ0201. Nothing ENFORCED that the copy
    // stays faithful to `cadenza_syntax::extern_name::is_valid_interface_name` — a drift in either
    // (e.g. cadenza-syntax tightening the grammar as in `1a2b9333a`, or a local edit) would silently
    // make the compiler accept/reject a binding string differently from the wasmtime load-time
    // reality, re-opening the miscompile. This differential test pins the two to AGREE on every input:
    // hand cases bracketing each grammar edge + a delimiter-rich deterministic fuzz (`:` `/` `@` `-`
    // multibyte + control chars — the structural delimiters the grammar keys on). A future divergence
    // fails HERE (caught), not at a user's component load.
    //
    // BOTH copied functions are pinned: `is_valid_interface_name` (the peer-binding-string guard) AND
    // `kebab_extern_name` (the export/member-name normalizer). They agree on the whole class including
    // a word-separator-immediately-before-a-digit name (`step-2`, `a_0`): per the operator ruling
    // (2026-07-16) BOTH keep it VERBATIM as an invalid `-`-led segment so `invalid_kebab_export_name`
    // DECLINES it with an actionable rename — NOT a silent collapse to `step2`/`a0` (which would
    // rename the author's identifier across the component / path-deps boundary). Earlier the reference
    // silently collapsed; this test drove the discovery, the concierge ruled decline-with-rename, and
    // the reference was conformed to the backend copy — so the two now agree here too and stay pinned.
    use crate::backend::common::export_name::{is_valid_interface_name, kebab_extern_name};
    use cadenza_syntax::extern_name as reference;

    // Hand cases bracketing the grammar edges (valid names, and each rejection cause).
    let seeds: &[&str] = &[
        "cadenza:pkg/api",       // valid: ns:label/iface
        "cadenza:pkg/api@1.0.0", // valid: + version
        "a:b:c/d/e",             // valid: multi-namespace pkg + multi projection
        "cadenza:pkg/Api",       // valid: projection may be uppercase-kebab
        "Cadenza:pkg/api",       // INVALID: package segment not lowercase
        "cadenza/api",           // INVALID: <2 package segments
        "cadenza:pkg",           // INVALID: no projection
        "cadenza:pkg/api@",      // INVALID: empty version
        "cadenza:pkg/-api",      // INVALID: projection segment hyphen-led
        "cadenza:pkg/api-",      // INVALID: projection segment trailing hyphen
        "cadenza:0pkg/api",      // INVALID: package segment digit-led
        "cadenza::pkg/api",      // INVALID: empty package segment
        "",                      // INVALID: empty
        "café:pkg/api",          // INVALID: non-ASCII in package
        "cadenza:pkg/apé",       // INVALID: non-ASCII in projection
    ];
    // Extra hand cases for the normalizer's separator-before-digit class (the one the ruling settled).
    let normalizer_seeds: &[&str] = &[
        "inc",
        "my-func",
        "myFunc",
        "fA",
        "Foo",
        "parseHTTPResponse",
        "foo-bar2",
        "a__b",
        "a_",
        "step-2",
        "a_0",
        "my_2nd",
        "x_1y",
        "a-0",
        "A0",
        "foo2",
    ];
    for &s in seeds.iter().chain(normalizer_seeds) {
        assert_eq!(
            is_valid_interface_name(s),
            reference::is_valid_interface_name(s),
            "copy vs cadenza-syntax DISAGREE on is_valid_interface_name({s:?}) — the copy has \
                 drifted from the reference grammar",
        );
        assert_eq!(
            kebab_extern_name(s),
            reference::kebab_extern_name(s),
            "copy vs cadenza-syntax DISAGREE on kebab_extern_name({s:?}) — the copy has drifted \
                 from the reference normalizer",
        );
    }

    // Deterministic fuzz: build delimiter-rich strings from the alphabet the grammar keys on, so the
    // two validators are compared on the structurally-interesting inputs (not just random noise). A
    // tiny xorshift PRNG seeded from a fixed constant keeps it reproducible (no wall-clock / rng).
    let alphabet: &[char] = &[
        'a', 'b', 'z', 'A', 'Z', '0', '9', '-', ':', '/', '@', 'π', 'é', '·', '\u{7f}', ' ',
    ];
    let mut state: u32 = 0x9E37_79B9;
    let mut next = || {
        // xorshift32 — deterministic, no external entropy.
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    for _ in 0..20_000 {
        let len = (next() % 12) as usize;
        let s: String = (0..len)
            .map(|_| alphabet[(next() as usize) % alphabet.len()])
            .collect();
        // (a) neither implementation may PANIC on any input; (b) they must AGREE (both functions).
        assert_eq!(
            is_valid_interface_name(&s),
            reference::is_valid_interface_name(&s),
            "copy vs cadenza-syntax DISAGREE on fuzz input is_valid_interface_name({s:?})",
        );
        assert_eq!(
            kebab_extern_name(&s),
            reference::kebab_extern_name(&s),
            "copy vs cadenza-syntax DISAGREE on fuzz input kebab_extern_name({s:?})",
        );
    }
}

#[test]
fn a_host_op_result_crosses_at_every_aliased_int_width() {
    // The host-op boundary ABI (`host::abi_val_type`) crosses EVERY aliased INT width — the narrow
    // ints `Int8`/`Int16`/`Int32` + unsigned `UInt8`, not only the earlier `Int64`/`UInt32`. Each
    // crosses as its faithful component-model primitive (s8/u8/s16/s32/…), lowered to the core i32 slot
    // the canonical ABI uses. Before, a narrow-int result DECLINED ("no component boundary form"). Here
    // each op is performed + its result compared to a sample (so the export result stays an Int64
    // scalar the boundary already crossed) and the program must EMIT. A narrow result arrives correctly
    // — the canonical lowering sign/zero-extends into the i32 slot that IS the guest's narrow-int rep.
    // (`Float32` also crosses via `abi_val_type`, but consuming a runtime Float32 in-guest hits an
    // unrelated float-op gap — its result-crossing is exercised through the run path, not here.)
    for (w, sample) in [
        ("Int8", "(: 100 Int8)"),
        ("Int16", "(: 100 Int16)"),
        ("Int32", "(: 100 Int32)"),
        ("UInt8", "(: 100 UInt8)"),
    ] {
        let src = format!(
            "(do (effect Test (op g (-> Int64 {w}))) \
                 (def (main) (host (Test) (if (= ((. Test g) 0) {sample}) 1 0))) (export main))"
        );
        let out = compile(&inputs(&src, &[Request::Emit(Target::Wasm)]), &[]);
        assert!(
            !out.has_error() && out.artifacts.iter().any(|a| a.kind == "component"),
            "a host op with a `{w}` result must cross the boundary + emit: {:?}",
            out.diagnostics
        );
    }
}

#[test]
fn a_uses_of_query_finds_every_reference_and_excludes_the_definition() {
    // `helper` is referenced twice (in `main` and in `other`); the query returns those occurrences
    // as node indices in ascending order, and the definition itself is not a use.
    let src = "(module m \
                   (def (helper) 1) \
                   (def (main) (+ helper helper)) \
                   (def (other) helper) \
                   (export main))";
    let out = compile(
        &inputs(
            src,
            &[Request::Query(Query::UsesOf {
                name: "helper".into(),
            })],
        ),
        &[],
    );
    assert!(!out.has_error());
    let bytes = artifact_bytes(&out, KIND_USES).expect("a uses artifact");
    let ids: Vec<u32> = cadenza_compile_abi::decode_uses(bytes);
    // Three references to `helper`, none of them the def's body.
    assert_eq!(ids.len(), 3, "uses = {ids:?}");
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "ascending order: {ids:?}"
    );
    // Each reported node resolves to `helper` — spot-check via a fresh resolve.
    let mut db = crate::db::Db::load(parse(src));
    let helper_body = db.defs[db.def_by_name("helper").unwrap()].body.unwrap();
    for &id in &ids {
        match crate::resolve::resolved_of(&mut db, crate::ast::StructId(id)) {
            crate::resolved::Resolved::Ref { value } => {
                assert_eq!(value, helper_body, "node {id} must reference helper")
            }
            other => panic!("node {id} resolved to {other:?}, not a Ref to helper"),
        }
    }
}

#[test]
fn a_uses_of_query_excludes_every_declaration_site_across_a_wide_module() {
    // `uses_of` walks every node and skips DECLARATION-SITE name occurrences via a set of every def's
    // signature-name head (was an O(defs) `Vec::contains` per node → O(nodes × defs) = O(N²); now an
    // O(1) hash-set membership). This locks in the set's correctness at WIDTH: in a module of many
    // defs that each REFERENCE `helper`, the query returns EXACTLY one use per referencing def and NOT
    // a single one of the (many) declaration-name occurrences — the exclusion the set must preserve.
    // `helper` is NULLARY so each reference is a bare `helper` name resolving to a `Ref` at its body
    // (the same shape the small sibling test checks) — one reference per `d{i}` body.
    let n = 30;
    let defs = (0..n)
        .map(|i| format!("(def (d{i}) helper)"))
        .collect::<Vec<_>>()
        .join(" ");
    let src = format!("(module m (def (helper) 1) {defs} (def (main) 42) (export main))");
    let out = compile(
        &inputs(
            &src,
            &[Request::Query(Query::UsesOf {
                name: "helper".into(),
            })],
        ),
        &[],
    );
    assert!(!out.has_error());
    let bytes = artifact_bytes(&out, KIND_USES).expect("a uses artifact");
    let ids: Vec<u32> = cadenza_compile_abi::decode_uses(bytes);
    // Exactly N references (one bare `helper` per `d{i}`) — no declaration-site name (helper's own, or
    // any of the N `d{i}` / `main` sig names) leaked in, and none of the N uses was missed. Ascending.
    assert_eq!(ids.len(), n, "one use per referencing def: {ids:?}");
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "ascending: {ids:?}");
    // Every reported node genuinely resolves to `helper`'s body (not a mis-included declaration name).
    let mut db = crate::db::Db::load(parse(&src));
    let helper_body = db.defs[db.def_by_name("helper").unwrap()].body.unwrap();
    for &id in &ids {
        assert!(
            matches!(
                crate::resolve::resolved_of(&mut db, crate::ast::StructId(id)),
                crate::resolved::Resolved::Ref { value } if value == helper_body
            ),
            "node {id} must reference helper"
        );
    }
}

#[test]
fn a_uses_of_query_for_an_unused_or_unknown_name_is_empty() {
    // A name with no references (or no such definition) yields an empty list, not an error.
    let src = "(module m (def (lonely) 1) (def (main) 42) (export main))";
    let out = compile(
        &inputs(
            src,
            &[Request::Query(Query::UsesOf {
                name: "lonely".into(),
            })],
        ),
        &[],
    );
    assert!(!out.has_error());
    let bytes = artifact_bytes(&out, KIND_USES).expect("a uses artifact");
    assert!(cadenza_compile_abi::decode_uses(bytes).is_empty());
}

#[test]
fn an_emit_request_is_a_target_reached_through_the_list() {
    // An `Emit(Wasm)` request produces the component exactly as `targets: [Wasm]` does — Emit IS
    // the generalization of a Target. Here `targets` is EMPTY; the component comes only from the
    // sidecar request, proving the two paths are one.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile(&inputs(src, &[Request::Emit(Target::Wasm)]), &[]);
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    assert!(
        out.artifact("component").is_some(),
        "a component is produced"
    );
}

#[test]
fn emit_and_query_compose_in_one_run() {
    // A realistic driver: build the component AND ask a fact in one invocation. Both artifacts come
    // back, selected by kind — one kinded-artifact list, not two calls.
    let src = "(module m (def (main) (: 42 Int64)) (export main))";
    let out = compile(
        &inputs(
            src,
            &[
                Request::Emit(Target::Wasm),
                Request::Query(Query::TypeOf {
                    name: "main".into(),
                }),
            ],
        ),
        &[],
    );
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    assert!(out.artifact("component").is_some());
    // The TYPE_INFO verdict rides alongside the emit — a `Found` with the `(Int 64)` payload (head `Int`).
    match cadenza_compile_abi::decode_type_info(
        artifact_bytes(&out, KIND_TYPE_INFO).expect("a type-info artifact"),
    ) {
        cadenza_compile_abi::TypeInfo::Found(ty) => {
            assert_eq!(ty.head_name(ty.root), Some("Int"), "Int64 payload")
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn no_sidecar_input_is_todays_behavior() {
    // The common path: no `sidecar` artifact at all. Behavior is today's — `targets` drives emission:
    // the component, PLUS the bytes-second guest result-type map (`KIND_RESULT_TYPES`, #5951 run-wiring
    // emitted whenever the layout has boundary exports — here `(export main)`). No QUERY artifacts.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile(
        &[Artifact::new(
            Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(src)),
        )],
        &[Target::Wasm],
    );
    assert!(!out.has_error());
    assert_eq!(
        out.artifacts.len(),
        2,
        "the component + the result-type map (no query artifacts)"
    );
    assert!(out.artifact("component").is_some(), "the component");
    assert!(
        out.artifact(Artifact::KIND_RESULT_TYPES).is_some(),
        "the bytes-second result-type map"
    );
}

#[test]
fn a_malformed_sidecar_list_declines() {
    // A `sidecar` artifact whose bytes are not a valid request list is a DECLINE (a diagnostic),
    // never a panic or a silent drop — reject-don't-miscompile at the tool edge.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&parse(src))),
            Artifact::new(sidecar::KIND_SIDECAR, "drive", vec![0xff, 0xff, 0xff]),
        ],
        &[Target::Wasm],
    );
    assert!(out.has_error());
    let d = out
        .diagnostics
        .iter()
        .find(|d| d.severity == Severity::Error)
        .unwrap();
    assert!(
        d.message.contains("malformed `sidecar`"),
        "message: {}",
        d.message
    );
    // No component: the request list could not be understood, so nothing was driven.
    assert!(out.artifact("component").is_none());
}

#[test]
fn a_type_at_query_types_the_node_at_a_source_offset() {
    // The "type at cursor" query: the CONSUMER resolves a source offset to the innermost node id
    // (via the span table it holds), then asks `TypeAt { node }`. This proves the split — offset→node
    // at the boundary (span-owning), node→type in the compiler (span-free). Here the literal `42` is
    // annotated Int64; hovering it yields `Int64`.
    let src = "(module m (def (main) (: 42 Int64)) (export main))";
    // The consumer parses WITH spans and maps the offset of `42` to its node. Use `read_spanned`
    // (a SINGLE top-level form stays bare) — the same root convention the real `cdz` CLI uses; the
    // whole-program `read_all_spanned` would wrap a lone `(module …)` in `(do …)`, and the
    // compiler's top-level scan would then miss the module's defs. The AST crosses to the compiler
    // as BYTES (the copy-don't-depend bridge): `cadenza_syntax`'s codec produces the byte-identical
    // form `rcdzc::codec::decode` reads, so the `StructId`s line up — which is exactly what lets the
    // span-resolved node id name the same node inside the compiler.
    let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse with spans");
    // Hover the `42` literal — the innermost node there is the literal itself, whose width the
    // annotation `(: 42 Int64)` pins to Int64 (the type column carries the SOLVED type, not the
    // bare-literal deferred one). `node_at_offset` maps the offset to that literal node.
    let off = src.find("42").expect("the literal is in the source");
    let node = spans
        .node_at_offset(off)
        .expect("a node at the literal offset");
    let ast = cadenza_syntax::codec::encode(&arenas);
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", ast),
            Artifact::new(
                sidecar::KIND_SIDECAR,
                "drive",
                sidecar::encode(&[Request::Query(Query::TypeAt { node: node.0 })]),
            ),
        ],
        &[],
    );
    assert!(
        !out.has_error(),
        "a query does not fail: {:?}",
        out.diagnostics
    );
    // The `42` literal is annotated Int64 → a `Ty` verdict carrying the `(Int 64)` payload (head `Int`).
    assert_ty_head(
        &cadenza_compile_abi::decode_type_at(
            artifact_bytes(&out, KIND_TYPE_AT).expect("a type-at artifact"),
        ),
        "Int",
    );
}

#[test]
fn a_type_at_query_for_a_non_user_node_is_total() {
    // A node id past the program is not a user node — a DEFINED "unknown", never a crash (the query
    // is total, guarding a malformed request the span table would never actually produce).
    let src = "(module m (def (main) 42) (export main))";
    let out = compile(
        &inputs(src, &[Request::Query(Query::TypeAt { node: 100_000 })]),
        &[],
    );
    assert!(!out.has_error());
    // A node past the program → the `Unknown` verdict (a defined "unknown", never a crash).
    assert!(matches!(
        cadenza_compile_abi::decode_type_at(
            artifact_bytes(&out, KIND_TYPE_AT).expect("a type-at artifact")
        ),
        cadenza_compile_abi::TypeAt::Unknown
    ));
}

/// The decoded [`cadenza_compile_abi::TypeAt`] hover VERDICT for the node at `substr`'s first occurrence
/// in `src` (KIND_TYPE_AT is now a binary-AST verdict; the display-string rendering is the cdz consumer's
/// job via `render_ty_scheme`, so a rcdzc test asserts the STRUCTURED verdict — see the assert helpers).
fn hover_at(src: &str, substr: &str) -> cadenza_compile_abi::TypeAt {
    let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
    let off = src.find(substr).expect("substr in source");
    let node = spans.node_at_offset(off).expect("a node at the offset");
    let ast = cadenza_syntax::codec::encode(&arenas);
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", ast),
            Artifact::new(
                sidecar::KIND_SIDECAR,
                "drive",
                sidecar::encode(&[Request::Query(Query::TypeAt { node: node.0 })]),
            ),
        ],
        &[],
    );
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    cadenza_compile_abi::decode_type_at(
        artifact_bytes(&out, KIND_TYPE_AT).expect("a type-at artifact"),
    )
}

/// Assert a hover verdict is a `Keyword(kw)`.
fn assert_keyword(v: &cadenza_compile_abi::TypeAt, kw: &str) {
    match v {
        cadenza_compile_abi::TypeAt::Keyword(k) => assert_eq!(k, kw),
        other => panic!("expected Keyword({kw}), got {other:?}"),
    }
}

/// Assert a hover verdict is a `Ty` whose payload head is `head` (the rcdzc-visible structure — the display
/// name is rendered consumer-side).
fn assert_ty_head(v: &cadenza_compile_abi::TypeAt, head: &str) {
    match v {
        cadenza_compile_abi::TypeAt::Ty(a) => {
            assert_eq!(a.head_name(a.root), Some(head), "Ty payload head: {a:?}")
        }
        other => panic!("expected Ty(head={head}), got {other:?}"),
    }
}

/// Assert a hover verdict is a `Def{name}` whose signature payload head is `ty_head` (`None` = unsolved).
fn assert_def(v: &cadenza_compile_abi::TypeAt, name: &str, ty_head: Option<&str>) {
    match v {
        cadenza_compile_abi::TypeAt::Def { name: n, ty } => {
            assert_eq!(n, name, "def name");
            assert_eq!(
                ty.as_ref().and_then(|a| a.head_name(a.root)),
                ty_head,
                "def sig payload head"
            );
        }
        other => panic!("expected Def({name}), got {other:?}"),
    }
}

#[test]
fn hover_on_a_grammar_keyword_names_the_keyword_not_any() {
    // A grammar keyword (`def`/`export`/`module`/`:`) is syntax, not an expression — hover names it
    // rather than returning the misleading `Any` fallback.
    let src = "(module m (def (main) (: 42 Int64)) (export main))";
    assert_keyword(&hover_at(src, "def"), "def");
    assert_keyword(&hover_at(src, "export"), "export");
    assert_keyword(&hover_at(src, "module"), "module");
    assert_keyword(&hover_at(src, ": 42"), ":");
}

#[test]
fn hover_on_a_definition_shows_its_signature() {
    // Hovering a function's NAME or its `(def …)` form shows the SIGNATURE (the full arrow), not the
    // body's return type alone. A nullary def shows `name : T`.
    let src = "(module m (def (inc (: n Int64)) (+ n 1)) (def (g) (: 5 Int64)) (export main))";
    // The function name and the whole def form both identify the def → its arrow signature payload.
    assert_def(&hover_at(src, "inc"), "inc", Some("->"));
    assert_def(&hover_at(src, "(def (inc"), "inc", Some("->"));
    // A nullary def's signature payload is the scalar `(Int 64)` (head `Int`).
    assert_def(&hover_at(src, "(g)"), "g", Some("Int"));
}

#[test]
fn hover_on_a_def_in_a_wide_module_finds_the_right_def_via_the_ident_index() {
    // `def_identified_by` (which def does a hovered header node identify?) reads a header→index INDEX
    // (`Db::def_index_by_ident`), not a linear `defs.iter().enumerate()` scan — the O(N²) `cdz query
    // --where`'s per-match `TypeAt` hit (each query scanned all defs). This locks in that the index
    // resolves a hovered def name to the CORRECT def at width: in a 60-def module, hovering `d30`'s
    // name must show `d30`'s own signature (a wrong index would show a neighbour's or none).
    let n = 60;
    let defs = (0..n)
        .map(|i| format!("(def (d{i} (: p Int64)) (+ p {i}))"))
        .collect::<Vec<_>>()
        .join(" ");
    let src = format!("(module m {defs} (def (main) (d0 1)) (export main))");
    // Hover a def NAME deep in the list — must resolve to that def's own signature (its arrow payload).
    assert_def(&hover_at(&src, "(d30 "), "d30", Some("->"));
    assert_def(&hover_at(&src, "(d59 "), "d59", Some("->"));
}

#[test]
fn hover_on_a_reference_shows_the_value_type_not_the_signature() {
    // A USE of a name is a value — it hovers as the value's type. (Only the DEFINITION shows the
    // `name : sig` form; a reference to a nullary def shows the value it denotes.)
    let src = "(module m (def (v) (: 7 Int64)) (def (main) v) (export main))";
    // The `v` reference in `(def (main) v)` — the LAST occurrence.
    let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
    let off = src.rfind('v').expect("the v reference");
    let node = spans.node_at_offset(off).unwrap();
    let ast = cadenza_syntax::codec::encode(&arenas);
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", ast),
            Artifact::new(
                sidecar::KIND_SIDECAR,
                "drive",
                sidecar::encode(&[Request::Query(Query::TypeAt { node: node.0 })]),
            ),
        ],
        &[],
    );
    // A reference to a value hovers as the VALUE's type (a `Ty` payload `(Int 64)`, head `Int`), not a sig.
    assert_ty_head(
        &cadenza_compile_abi::decode_type_at(
            artifact_bytes(&out, KIND_TYPE_AT).expect("a type-at artifact"),
        ),
        "Int",
    );
}

#[test]
fn hover_on_an_unannotated_but_inferable_param_binder_shows_the_solved_type() {
    // The v-lsp inlayHint gap: an UN-ANNOTATED param binder used to hover as "unknown" even when its
    // type is locally inferable — but the whole point of an inlay hint is to show the type the author
    // did NOT write. A non-recursive def's param inlines at each call and is never solved standalone
    // (`type_of` at the binder reads `Any`), yet the body's uses constrain it: `(+ x 1)` pins `x` to
    // `Int64`. `query_param_ty` recovers that via the body-constraint solve, so the binder now hovers
    // as its inferred type. (`f x)` — the `x` in the SIGNATURE is the binder; the reference in the body
    // already typed via inlining.)
    let src = "(module m (def (f x) (+ x 1)) (def (main) (f 5)) (export main))";
    // Hover the binder occurrence itself (the `x` in the signature `(f x)`).
    let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
    let off = src.find("f x").expect("sig") + 2; // the binder `x`
    let node = spans.node_at_offset(off).expect("node at binder");
    let ast = cadenza_syntax::codec::encode(&arenas);
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", ast),
            Artifact::new(
                sidecar::KIND_SIDECAR,
                "drive",
                sidecar::encode(&[Request::Query(Query::TypeAt { node: node.0 })]),
            ),
        ],
        &[],
    );
    // The recovered inferred type is a `Ty` payload `(Int 64)` (head `Int`), via query_param_ty.
    assert_ty_head(
        &cadenza_compile_abi::decode_type_at(
            artifact_bytes(&out, KIND_TYPE_AT).expect("a type-at artifact"),
        ),
        "Int",
    );
}

#[test]
fn hover_on_a_fully_generic_param_binder_stays_unknown() {
    // The no-over-reach twin: a FULLY GENERIC param (`(def (id x) x)`) has NO single monomorphic type —
    // a query must NOT invent a width. `query_param_ty` returns `None` (the body imposes no operand
    // constraint), so the binder correctly stays "unknown" rather than being pinned to a call-site's
    // instantiation. Guards against the fix over-reaching into scheme variables.
    let src = "(module m (def (id x) x) (def (main) (id 5)) (export main))";
    let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
    let off = src.find("id x").expect("sig") + 3; // the binder `x`
    let node = spans.node_at_offset(off).expect("node at binder");
    let ast = cadenza_syntax::codec::encode(&arenas);
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", ast),
            Artifact::new(
                sidecar::KIND_SIDECAR,
                "drive",
                sidecar::encode(&[Request::Query(Query::TypeAt { node: node.0 })]),
            ),
        ],
        &[],
    );
    assert!(
        matches!(
            cadenza_compile_abi::decode_type_at(
                artifact_bytes(&out, KIND_TYPE_AT).expect("a type-at artifact")
            ),
            cadenza_compile_abi::TypeAt::Unknown
        ),
        "a fully-generic param must not be pinned to a monomorphic width"
    );
}

#[test]
fn hover_on_an_operator_does_not_leak_a_record() {
    // A prelude operator name (`+`) resolves to an internal record; hover must not leak the record — the
    // verdict is a `Ty` carrying the CALLABLE ARROW payload (head `->`, not `Record`), so the consumer
    // renders the operator's arrow.
    let src = "(module m (def (main) (+ 1 2)) (export main))";
    assert_ty_head(&hover_at(src, "+"), "->");
}

#[test]
fn a_diagnostics_query_reports_faults_without_an_export() {
    // The "diagnostics as you type" primitive: an ill-typed program with NO export still yields its
    // faults (the query is not gated on layout/export). `(if 5 1 2)` — a non-Bool condition — is a
    // CDZ0203, carried in the canonical binary-AST `KIND_DIAGNOSTICS` wire (seq-254).
    let src = "(module m (def (main) (if 5 1 2)))"; // note: NO (export …)
    let out = compile(&inputs(src, &[Request::Query(Query::Diagnostics)]), &[]);
    // A query never fails the compile; the diagnostics ride in the artifact, not the error channel.
    assert!(
        !out.has_error(),
        "a query does not fail: {:?}",
        out.diagnostics
    );
    let bytes = out
        .artifacts
        .iter()
        .find(|a| a.kind == KIND_DIAGNOSTICS)
        .map(|a| a.bytes.clone())
        .expect("a diagnostics artifact");
    // Binary-AST wire — decode with the shared codec, don't parse text/columns.
    let diags = crate::decode_diagnostics(&bytes);
    let d = diags
        .iter()
        .find(|d| d.code.as_deref() == Some("CDZ0203"))
        .unwrap_or_else(|| panic!("expected a CDZ0203 fault, got:\n{diags:?}"));
    assert_eq!(d.severity, crate::Severity::Error, "severity");
    assert!(
        d.node.is_some(),
        "the fault anchors to a node: {:?}",
        d.node
    );
    assert!(!d.message.is_empty(), "message is non-empty");
}

#[test]
fn a_diagnostics_query_on_a_clean_program_is_empty() {
    // A well-formed program yields the empty diagnostics result (total — no faults, no error).
    let src = "(module m (def (main) (: 42 Int64)) (export main))";
    let out = compile(&inputs(src, &[Request::Query(Query::Diagnostics)]), &[]);
    assert!(!out.has_error());
    // The KIND_DIAGNOSTICS wire is binary AST; a clean program decodes to zero faults (whether the
    // artifact is absent or an encoded empty list).
    let faults = out
        .artifacts
        .iter()
        .find(|a| a.kind == KIND_DIAGNOSTICS)
        .map(|a| crate::decode_diagnostics(&a.bytes))
        .unwrap_or_default();
    assert!(
        faults.is_empty(),
        "a clean program has no faults: {faults:?}"
    );
}

#[test]
fn a_resolve_of_query_finds_the_defining_occurrence() {
    // Go-to-definition: a reference to `helper` resolves to helper's def NAME occurrence — the token
    // an editor highlights when you jump — NOT its body/value. The consumer maps an offset to the
    // reference node; ResolveOf answers the defining NAME's node id.
    let src = "(module m (def (helper) 1) (def (main) helper) (export main))";
    let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse with spans");
    // The reference is the `helper` in `(def (main) helper)` — the LAST occurrence of "helper".
    let ref_off = src.rfind("helper").expect("a reference to helper");
    let ref_node = spans
        .node_at_offset(ref_off)
        .expect("a node at the reference");
    let ast = cadenza_syntax::codec::encode(&arenas);
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", ast),
            Artifact::new(
                sidecar::KIND_SIDECAR,
                "drive",
                sidecar::encode(&[Request::Query(Query::ResolveOf { node: ref_node.0 })]),
            ),
        ],
        &[],
    );
    assert!(!out.has_error());
    let target: u32 = cadenza_compile_abi::decode_resolve(
        artifact_bytes(&out, KIND_RESOLVE).expect("a resolve artifact"),
    )
    .expect("a node id");
    // The target is helper's def NAME occurrence (the sig's first child) — the go-to-definition
    // anchor. Spot-check via a fresh resolve: the sig's first child, NOT the body.
    let db = crate::db::Db::load(parse(src));
    let sig = db.defs[db.def_by_name("helper").unwrap()].sig_occ;
    let helper_name = match db.ast.get(sig) {
        crate::ast::Struct::List(kids) => kids[0],
        _ => panic!("a def sig is a list"),
    };
    assert_eq!(
        crate::ast::StructId(target),
        helper_name,
        "ResolveOf points at helper's def NAME occurrence, not its body"
    );
}

#[test]
fn a_resolve_of_query_for_a_non_reference_is_empty() {
    // A literal (or any non-navigable node) resolves to nothing — the empty result, total.
    let src = "(module m (def (main) 42) (export main))";
    let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
    let lit = spans
        .node_at_offset(src.find("42").unwrap())
        .expect("the literal node");
    let ast = cadenza_syntax::codec::encode(&arenas);
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", ast),
            Artifact::new(
                sidecar::KIND_SIDECAR,
                "drive",
                sidecar::encode(&[Request::Query(Query::ResolveOf { node: lit.0 })]),
            ),
        ],
        &[],
    );
    assert!(!out.has_error());
    let bytes = artifact_bytes(&out, KIND_RESOLVE).expect("a resolve artifact");
    assert_eq!(cadenza_compile_abi::decode_resolve(bytes), None);
}

/// Parse `src`, resolve `offset` to a node, run `ScopeAt`, and return each binding's `(name, ty-head)` —
/// the KIND_SCOPE wire is now binary AST (`cadenza_compile_abi::decode_scope`), each binding carrying the
/// FULL structured type payload; rcdzc has no consumer-side type-name renderer, so a test asserts the
/// payload's STRUCTURE (its head name, e.g. `Int` for an `(Int 64)` = Int64 binding) rather than a rendered
/// string (rendering is the cdz consumer's job via `render_ty_scheme`).
fn scope_bindings(src: &str, offset: usize) -> Vec<(String, String)> {
    let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
    let node = spans.node_at_offset(offset).expect("a node at the offset");
    let ast = cadenza_syntax::codec::encode(&arenas);
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", ast),
            Artifact::new(
                sidecar::KIND_SIDECAR,
                "drive",
                sidecar::encode(&[Request::Query(Query::ScopeAt { node: node.0 })]),
            ),
        ],
        &[],
    );
    assert!(
        !out.has_error(),
        "a query does not fail: {:?}",
        out.diagnostics
    );
    let bytes = artifact_bytes(&out, KIND_SCOPE).expect("a scope artifact");
    cadenza_compile_abi::decode_scope(bytes)
        .into_iter()
        .map(|b| {
            let head = b.ty.head_name(b.ty.root).unwrap_or("?").to_string();
            (b.name, head)
        })
        .collect()
}

#[test]
fn a_scope_at_query_lists_let_bindings_and_params_with_types() {
    // Inside `body`, both the parameter `p` (Int64) and the let-binding `q` (Int64) are visible.
    let src = "(module m (def (f (: p Int64)) (let ((q (: 5 Int64))) (+ p q))) (export main))";
    // Offset at the `(+ p q)` body.
    let off = src.find("(+ p q)").expect("the body");
    let scope = scope_bindings(src, off);
    // `p` and `q` are both in scope, both Int64 — the payload head is `Int` (an `(Int 64)` = Int64 type).
    assert!(
        scope.iter().any(|(n, t)| n == "p" && t == "Int"),
        "param p:Int64 in scope: {scope:?}"
    );
    assert!(
        scope.iter().any(|(n, t)| n == "q" && t == "Int"),
        "let-binding q:Int64 in scope: {scope:?}"
    );
}

#[test]
fn a_scope_at_query_at_the_top_level_is_empty() {
    // At a top-level def body with no enclosing binder, no local bindings are in scope.
    let src = "(module m (def (main) 42) (export main))";
    let off = src.find("42").expect("the literal");
    assert!(
        scope_bindings(src, off).is_empty(),
        "top level has no local scope"
    );
}

#[test]
fn a_scope_at_query_respects_sequential_let_scope() {
    // In `(let ((a 1) (b (+ a 1))) …)`, the initializer of `b` sees `a` but NOT `b` itself.
    let src = "(module m (def (main) (let ((a (: 1 Int64)) (b (+ a 1))) b)) (export main))";
    // Offset inside `b`'s initializer `(+ a 1)`.
    let off = src.find("(+ a 1)").expect("b's initializer");
    let scope = scope_bindings(src, off);
    assert!(
        scope.iter().any(|(n, _)| n == "a"),
        "a is visible in b's init: {scope:?}"
    );
    assert!(
        !scope.iter().any(|(n, _)| n == "b"),
        "b is NOT visible in its own init: {scope:?}"
    );
}

#[test]
fn an_exports_query_lists_each_export_with_its_type() {
    // The module interface: every `(export …)` clause paired with the named def's FULL structured type
    // payload (KIND_EXPORTS is now binary AST — `cadenza_compile_abi::decode_exports`; the type NAME
    // rendering "(-> Int64 Int64)"/"Int64" is the CONSUMER's job, so here we assert the producer emits
    // the right STRUCTURED payload: a function's arrow head, a value's scalar head, in export order).
    let src = "(module m (def (inc (: n Int64)) (+ n 1)) (def (v) (: 5 Int64)) \
                   (export inc) (export v))";
    let out = compile(&inputs(src, &[Request::Query(Query::Exports)]), &[]);
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    let bytes = artifact_bytes(&out, KIND_EXPORTS).expect("an exports artifact");
    let exports = cadenza_compile_abi::decode_exports(bytes);
    assert_eq!(exports.len(), 2, "both exports, in order: {exports:?}");
    assert_eq!(exports[0].name, "inc");
    let inc_ty = exports[0].ty.as_ref().expect("inc's type resolved");
    assert_eq!(
        inc_ty.head_name(inc_ty.root),
        Some("->"),
        "inc is a function arrow: {inc_ty:?}"
    );
    assert_eq!(exports[1].name, "v");
    let v_ty = exports[1].ty.as_ref().expect("v's type resolved");
    assert_eq!(
        v_ty.head_name(v_ty.root),
        Some("Int"),
        "v is an Int scalar: {v_ty:?}"
    );
    assert!(
        exports[0].node.is_some() && exports[1].node.is_some(),
        "each export carries a name-occurrence node for go-to"
    );
}

#[test]
fn an_exports_query_with_no_exports_is_empty() {
    // A module with no `(export …)` yields the empty interface (total, not an error). The binary-AST
    // value decodes to zero entries (the wire is `(exports)` — a non-empty byte string, but an empty list).
    let src = "(module m (def (main) 42))";
    let out = compile(&inputs(src, &[Request::Query(Query::Exports)]), &[]);
    assert!(!out.has_error());
    let bytes = artifact_bytes(&out, KIND_EXPORTS).expect("an exports artifact");
    assert!(
        cadenza_compile_abi::decode_exports(bytes).is_empty(),
        "no exports → empty interface"
    );
}

/// The `Symbols` outline as `(name, kind)` rows, in the artifact's emit order — decoded from the
/// binary-AST wire.
fn symbol_rows(src: &str) -> Vec<(String, String)> {
    let out = compile(&inputs(src, &[Request::Query(Query::Symbols)]), &[]);
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    cadenza_compile_abi::decode_symbols(
        artifact_bytes(&out, KIND_SYMBOLS).expect("a symbols artifact"),
    )
    .into_iter()
    .map(|(name, kind, _)| (name, kind))
    .collect()
}

#[test]
fn a_symbols_query_outlines_every_top_level_declaration_by_kind() {
    // The document outline: every declaration classified — a nullary def is a `value`, a def with
    // params a `function`, plus `type`/`effect`. Columns grouped (defs, then types, then effects).
    let src = "(do (type Color Red Green Blue) \
                   (effect Log (op emit (-> Int64 Unit))) \
                   (def answer 42) (def (double x) (+ x x)) (export double answer))";
    let rows = symbol_rows(src);
    assert_eq!(
        rows,
        vec![
            ("answer".to_string(), "value".to_string()),
            ("double".to_string(), "function".to_string()),
            ("Color".to_string(), "type".to_string()),
            ("Log".to_string(), "effect".to_string()),
        ],
        "rows: {rows:?}"
    );
}

#[test]
fn a_symbols_query_lists_private_declarations_not_just_exports() {
    // The superset property vs `Exports`: a def that is NOT exported still appears in the outline.
    let src = "(do (def (public-fn x) x) (def (private-fn y) y) (export public-fn))";
    let rows = symbol_rows(src);
    let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"public-fn"), "rows: {rows:?}");
    assert!(
        names.contains(&"private-fn"),
        "an outline lists the UNEXPORTED def too — rows: {rows:?}"
    );
}

#[test]
fn a_symbols_query_omits_prelude_types_and_module_internals() {
    // A prelude sum (`Option`/`Result`/…) is injected into `type_decls` with no source span — it is
    // NOT the user's declaration, so it must not appear. A module's member def is INTERNAL (a
    // synthesized callable), so it is omitted too — only the `module` itself is a top-level symbol.
    let src = "(do (module geo (def (area r) (* r r)) (export area)) \
                   (def (main) (geo.area 3)) (export main))";
    let rows = symbol_rows(src);
    let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"main"), "rows: {rows:?}");
    assert!(
        names.contains(&"geo") && rows.iter().any(|(n, k)| n == "geo" && k == "module"),
        "the module is a symbol — rows: {rows:?}"
    );
    assert!(
        !names.contains(&"Option") && !names.contains(&"Result"),
        "prelude types must not leak into the outline — rows: {rows:?}"
    );
    assert!(
        !names.contains(&"area"),
        "a module-member callable is internal, not a top-level symbol — rows: {rows:?}"
    );
}

#[test]
fn a_symbols_query_on_a_declarationless_program_is_empty() {
    // Total: a program with no top-level declarations yields the empty outline, never an error.
    let src = "42";
    let out = compile(&inputs(src, &[Request::Query(Query::Symbols)]), &[]);
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    let bytes = artifact_bytes(&out, KIND_SYMBOLS).expect("a symbols artifact");
    assert!(cadenza_compile_abi::decode_symbols(bytes).is_empty());
}

#[test]
fn a_symbols_query_carries_a_jumpable_name_node() {
    // The node id in each record is the declaration's NAME occurrence (a user node), so a consumer
    // can resolve it to a source range and jump — the go-to affordance the outline rides on.
    let src = "(do (type Color Red Green Blue) (def (f x) x) (export f))";
    let out = compile(&inputs(src, &[Request::Query(Query::Symbols)]), &[]);
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    let symbols = cadenza_compile_abi::decode_symbols(
        artifact_bytes(&out, KIND_SYMBOLS).expect("a symbols artifact"),
    );
    assert!(!symbols.is_empty(), "the outline has declarations");
    // Every reported node id is a real in-range arena node (the binary-AST wire carries a `u32` per
    // record, so a `-` sentinel is structurally impossible — a consumer can always resolve + jump).
    let arenas = parse(src);
    for (name, _kind, node) in &symbols {
        assert!(
            (*node as usize) < arenas.structure.len(),
            "name {name} node {node} out of range"
        );
    }
}

#[test]
fn a_param_manifest_query_renders_each_param_site_to_a_row() {
    // The `@param` WIDGET MANIFEST query (v-metaprogramming's scan + v-cdz-tooling's Query+CLI): one row
    // per `(: (@ (param <kv>) name) Type)` site, TAB-separated
    // `name  widget  type  range-lo  range-hi  options  default  name-node`. The DECLARED TYPE is
    // rendered here (the type column, `Ty::render_name`); the value fields are ARENA NODE IDS (or `-`),
    // which the CLI renders. (Range spelled `(list 0 100)` in s-expr — `[0 100]` is ML-surface sugar.)
    let src = "(module m \
                   (pragma param (param (: widget slider) (: range (list 0 100))) (: width Int64)) \
                   (pragma param (param (: widget toggle)) (: mirror Bool)) \
                   (def (main) 0) (export main))";
    let out = compile(&inputs(src, &[Request::Query(Query::ParamManifest)]), &[]);
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    // The manifest is now canonical binary AST (operator P0 seq-284): decode the sites via the shared
    // codec. The DECLARED TYPE rides as a FULL structured `Ty` sub-AST (a standalone arena per site),
    // NOT a render_name string — a scalar type's payload is a bare type-name leaf (`Int64`/`Bool`).
    let bytes = &out
        .artifacts
        .iter()
        .find(|a| a.kind == KIND_PARAM_MANIFEST)
        .expect("a param-manifest artifact")
        .bytes;
    let sites = cadenza_compile_abi::param_manifest_wire::decode(bytes);
    assert_eq!(sites.len(), 2, "two @param sites → two sites");

    // width: slider widget, Int64 type (a full Ty sub-AST), a range present, no options/default.
    let width = sites
        .iter()
        .find(|s| s.name == "width")
        .expect("width site");
    assert_eq!(width.widget.as_deref(), Some("slider"), "width widget");
    // Int64 rides as the STRUCTURED `(Int 64)` payload (encode_ty_payload), NOT a "Int64" render string —
    // the full-Ty-AST point. (A consumer renders it back to "Int64" via render_ty_name.)
    assert_eq!(
        width
            .ty
            .as_form(width.ty.root, "Int")
            .and_then(|t| t.first())
            .and_then(|&w| width.ty.as_int(w))
            .and_then(|v| v.to_i64()),
        Some(64),
        "width declared type is a structured (Int 64) sub-AST"
    );
    assert!(
        width.range.is_some(),
        "width has a range: {:?}",
        width.range
    );
    assert_eq!(width.options, None, "width has no options");
    assert_eq!(width.default, None, "width has no default");

    // mirror: toggle widget, Bool type, NO range (absent → None, a stable schema).
    let mirror = sites
        .iter()
        .find(|s| s.name == "mirror")
        .expect("mirror site");
    assert_eq!(mirror.widget.as_deref(), Some("toggle"), "mirror widget");
    assert_eq!(
        mirror.ty.as_name(mirror.ty.root),
        Some("Bool"),
        "mirror declared type"
    );
    assert_eq!(mirror.range, None, "mirror has no range");
}

#[test]
fn a_param_manifest_query_on_a_paramless_program_is_empty() {
    // Total: a program with no `@param` sites yields the empty manifest, never an error.
    let src = "(do (def (main) 0) (export main))";
    let out = compile(&inputs(src, &[Request::Query(Query::ParamManifest)]), &[]);
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    let bytes = &out
        .artifacts
        .iter()
        .find(|a| a.kind == KIND_PARAM_MANIFEST)
        .expect("a param-manifest artifact")
        .bytes;
    assert!(
        cadenza_compile_abi::param_manifest_wire::decode(bytes).is_empty(),
        "no @param sites → empty manifest"
    );
}

#[test]
fn a_func_layout_query_reports_the_defs_begin_marker_and_a_recursive_defs_func_index_row() {
    // A `FuncLayout` request lays out the boundary and reports each reachable EMITTED def's absolute
    // func-index + a content-hash, preceded by a `defs-begin<TAB><import_base><TAB>-` marker. A
    // RECURSIVE def with a runtime arg stays a standalone function (a non-recursive small def would
    // INLINE and not appear) — so `sumto` is an emitted row. Scalar program → import_base 0.
    let src = "(module m \
                   (def (sumto (: n Int64)) (if (= n 0) 0 (+ n (sumto (- n 1))))) \
                   (def (main) (sumto 5)) (export main))";
    let out = compile(&inputs(src, &[Request::Query(Query::FuncLayout)]), &[]);
    assert!(
        !out.has_error(),
        "a query does not fail: {:?}",
        out.diagnostics
    );
    let text = func_layout_text(&out).expect("a func-layout artifact");
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("defs-begin\t0\t-"),
        "first row is the defs-begin marker with import_base:\n{text}"
    );
    // `sumto` is a standalone emitted function (recursive); the linear-recursion accumulator transform
    // emits it under a `sumto$acc` name (a tail-recursive copy), so match by the `sumto` prefix.
    let row = text
        .lines()
        .find(|l| l.split('\t').nth(2).is_some_and(|n| n.starts_with("sumto")))
        .unwrap_or_else(|| panic!("a `sumto*` row (recursive def stays standalone):\n{text}"));
    let cols: Vec<&str> = row.split('\t').collect();
    assert_eq!(cols.len(), 3, "row is idx<TAB>hash<TAB>name: {row:?}");
    assert!(
        cols[0].parse::<u32>().is_ok(),
        "func-index is a number: {row:?}"
    );
    assert!(
        cols[1].len() == 16 && cols[1].chars().all(|c| c.is_ascii_hexdigit()),
        "the content-hash is 16 hex digits: {row:?}"
    );
}

#[test]
fn a_func_layout_content_hash_is_stable_for_a_byte_identical_def_across_programs() {
    // The prove-first invariant the compile-reuse witness rides on: a def byte-identical in two DIFFERENT
    // programs reports the SAME content-hash — a function of the def's own AST subtree, NOT its global
    // StructId (which shifts when other defs precede it) nor the surrounding program. Use a RECURSIVE def
    // (`sumto`) so it stays a standalone emitted function (a non-recursive one inlines away).
    // Match the emitted row by NAME PREFIX (a recursive def emits under `<name>$acc` after the
    // accumulator transform), reading the hash column.
    let hash_of = |src: &str, name: &str| -> String {
        let out = compile(&inputs(src, &[Request::Query(Query::FuncLayout)]), &[]);
        assert!(!out.has_error(), "{:?}", out.diagnostics);
        let text = func_layout_text(&out).expect("func-layout");
        text.lines()
            .find(|l| l.split('\t').nth(2).is_some_and(|n| n.starts_with(name)))
            .and_then(|l| l.split('\t').nth(1))
            .unwrap_or_else(|| panic!("a `{name}*` row with a hash:\n{text}"))
            .to_string()
    };
    // Program A: `sumto` + `main`. Program B: an EXTRA recursive `dbl` declared BEFORE `sumto` (shifting
    // sumto's global StructId + func-index) + a `main` using both. `sumto`'s own source is identical.
    let a = "(module m \
                 (def (sumto (: n Int64)) (if (= n 0) 0 (+ n (sumto (- n 1))))) \
                 (def (main) (sumto 5)) (export main))";
    let b = "(module m \
                 (def (dbl (: k Int64)) (if (= k 0) 0 (+ 2 (dbl (- k 1))))) \
                 (def (sumto (: n Int64)) (if (= n 0) 0 (+ n (sumto (- n 1))))) \
                 (def (main) (+ (sumto 5) (dbl 3))) (export main))";
    assert_eq!(
        hash_of(a, "sumto"),
        hash_of(b, "sumto"),
        "a byte-identical `sumto` hashes the same regardless of surrounding defs / its StructId"
    );
}

#[test]
fn a_func_layout_query_roots_on_tests_when_a_program_has_no_export() {
    // The compile-reuse witness targets PURE `@test` files (sread-eval-fns / -ho) — no `(export …)`. A
    // FuncLayout on such a file must NOT decline (empty): it falls back to the `@test`-rooted layout
    // (`compute_tests`), the SAME func-index set `cdz test` emits, so the witness can diff the shared
    // (non-test) rows. Here a nullary `@test` calls a recursive `sumto`, which stays a standalone
    // emitted function (a shared def a real witness would key on).
    let src = "(do \
                    (def (sumto (: n Int64)) (if (= n 0) 0 (+ n (sumto (- n 1))))) \
                    (@ test (def (t) (if (= (sumto 5) 15) unit (trap \"x\"))))) ";
    let out = compile(&inputs(src, &[Request::Query(Query::FuncLayout)]), &[]);
    assert!(
        !out.has_error(),
        "a query does not fail: {:?}",
        out.diagnostics
    );
    let text = func_layout_text(&out).expect("a func-layout artifact");
    assert!(
        text.starts_with("defs-begin\t"),
        "a pure-@test program still lays out (rooted on @tests), not an empty decline:\n{text:?}"
    );
    // `sumto` (recursive, reachable from the @test) is a standalone emitted row — the kind of shared
    // def the witness diffs across the two files.
    assert!(
        text.lines()
            .any(|l| l.split('\t').nth(2).is_some_and(|n| n.starts_with("sumto"))),
        "the @test-rooted layout reaches + emits `sumto`:\n{text}"
    );
}

#[test]
fn a_doc_of_query_reads_a_definitions_docstring() {
    // A `DocOf` request answers with a definition's `(doc "…")` text — captured off the def body at
    // load (`strip_def_docs`) and read from the doc column, for both a value and a function def.
    let src = "(module m \
                   (def answer (doc \"the answer\") 42) \
                   (def (dbl (: x Int64)) (doc \"doubles x\") (* x 2)) \
                   (def (main) answer) (export main))";
    let out = compile(
        &inputs(
            src,
            &[
                Request::Query(Query::DocOf {
                    name: "answer".into(),
                }),
                Request::Query(Query::DocOf { name: "dbl".into() }),
            ],
        ),
        &[],
    );
    assert!(
        !out.has_error(),
        "a query does not fail: {:?}",
        out.diagnostics
    );
    // The FIRST DocOf artifact is `answer`'s doc, the SECOND is `dbl`'s (request order preserved).
    use cadenza_compile_abi::DocAnswer;
    assert_eq!(
        doc_answers(&out),
        vec![
            DocAnswer::Doc("the answer".to_string()),
            DocAnswer::Doc("doubles x".to_string())
        ]
    );
}

#[test]
fn a_doc_of_query_distinguishes_undocumented_from_unknown_and_is_total() {
    // Both are DEFINED answers (never an error — the oracle contract: a query is total over every
    // input), but with DISTINCT verdicts so a consumer can tell a real-but-undocumented name from a
    // typo: `main` IS a def (no doc → "no documentation for"), `ghost` names NOTHING ("no such
    // definition"). `cdz doc` maps the "no such definition" variant to a non-zero exit. A typo that is
    // a NEAR-MISS of a real def (`mian`) additionally gets a "did you mean?" suggestion.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile(
        &inputs(
            src,
            &[
                Request::Query(Query::DocOf {
                    name: "main".into(),
                }),
                Request::Query(Query::DocOf {
                    name: "ghost".into(),
                }),
                Request::Query(Query::DocOf {
                    name: "mian".into(),
                }),
            ],
        ),
        &[],
    );
    assert!(!out.has_error());
    // DISTINCT structured verdicts (not sentinel strings): a real-but-undocumented def, a typo naming
    // nothing, and a near-miss typo carrying a suggestion. The user-facing wording lives on the `cdz`
    // consumer now — the wire carries only the variant + the optional suggestion.
    use cadenza_compile_abi::DocAnswer;
    assert_eq!(
        doc_answers(&out),
        vec![
            DocAnswer::Undocumented,
            DocAnswer::NoSuchDef { suggestion: None },
            DocAnswer::NoSuchDef {
                suggestion: Some("main".to_string())
            },
        ]
    );
}

#[test]
fn a_doc_of_query_falls_back_to_a_builtin_meta_doc_channel() {
    // A built-in module is just a record, so its documentation is a `(meta doc)` channel on it, read
    // GENERICALLY — the query resolves `List` to its prelude record, then reads the channel (never a
    // name match). A grammar KEYWORD (`if`), not a binding, gets its doc from the keyword table.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile(
        &inputs(
            src,
            &[
                Request::Query(Query::DocOf {
                    name: "List".into(),
                }),
                Request::Query(Query::DocOf { name: "if".into() }),
            ],
        ),
        &[],
    );
    assert!(!out.has_error());
    use cadenza_compile_abi::DocAnswer;
    let answers = doc_answers(&out);
    let doc_text = |a: &DocAnswer| match a {
        DocAnswer::Doc(t) => t.clone(),
        other => panic!("expected a Doc answer, got {other:?}"),
    };
    let d0 = doc_text(&answers[0]);
    let d1 = doc_text(&answers[1]);
    assert!(
        d0.contains("persistent") && d0.contains("sequence"),
        "List's built-in doc: {d0:?}"
    );
    assert!(d1.starts_with("Conditional"), "if's keyword doc: {d1:?}");
}

#[test]
fn a_doc_at_query_reads_the_doc_at_a_reference_and_at_the_definition() {
    // `DocAt` is node-id-keyed: a USE of a documented def, and the def's OWN name occurrence, both
    // surface its doc (the hover). Resolve the two spellings to node ids through the arena.
    let src =
        "(module m (def helper (doc \"a helper value\") 7) (def (main) helper) (export main))";
    let arenas = parse(src);
    // The def's NAME occurrence (`helper` in the def signature) and its later USE (`helper` in main).
    let helper_ids: Vec<u32> = (0..arenas.structure.len() as u32)
        .filter(|&i| arenas.as_name(crate::ast::StructId(i)) == Some("helper"))
        .collect();
    assert_eq!(helper_ids.len(), 2, "one def-name occurrence + one use");
    for &node in &helper_ids {
        let out = compile(&inputs(src, &[Request::Query(Query::DocAt { node })]), &[]);
        assert!(!out.has_error(), "{:?}", out.diagnostics);
        assert_eq!(
            cadenza_compile_abi::decode_doc(
                artifact_bytes(&out, KIND_DOC).expect("a doc artifact")
            ),
            cadenza_compile_abi::DocAnswer::Doc("a helper value".to_string()),
            "node {node} should surface the doc"
        );
    }
}

#[test]
fn a_doc_at_query_for_an_undocumented_node_is_empty() {
    // A node that reaches no documented definition (a literal, an undocumented def's use) yields the
    // EMPTY result — total, not an error.
    let src = "(module m (def (main) 42) (export main))";
    let arenas = parse(src);
    // The `42` literal — not a reference to any documented def.
    let lit = (0..arenas.structure.len() as u32)
            .find(|&i| {
                matches!(
                    arenas.get(crate::ast::StructId(i)),
                    crate::ast::Struct::Atom(l) if matches!(arenas.leaf(*l), crate::ast::Leaf::Int { .. })
                )
            })
            .expect("a literal node");
    let out = compile(
        &inputs(src, &[Request::Query(Query::DocAt { node: lit })]),
        &[],
    );
    assert!(!out.has_error());
    assert_eq!(
        cadenza_compile_abi::decode_doc(artifact_bytes(&out, KIND_DOC).expect("a doc artifact")),
        cadenza_compile_abi::DocAnswer::Undocumented
    );
}

/// Run the `Highlight` query over `src` and collect the SET of `kind` strings assigned to the leaf
/// whose source spelling is `spelling` — a highlight is node-id-keyed, so re-resolve the spelling to
/// its node id(s) through the arena and pick their kinds. Returns every kind seen for that spelling
/// (usually one, but a name used in two roles — e.g. a type in two positions — could differ).
fn highlight_kinds_of(src: &str, spelling: &str) -> Vec<String> {
    let out = compile(&inputs(src, &[Request::Query(Query::Highlight)]), &[]);
    assert!(!out.has_error(), "{:?}", out.diagnostics);
    // node-id → kind (decoded from the binary-AST wire)
    let by_id: std::collections::BTreeMap<u32, String> =
        highlight_pairs(&out).into_iter().collect();
    // Find every leaf whose name spelling matches, then read its kind.
    let arenas = parse(src);
    let mut kinds = Vec::new();
    for i in 0..arenas.structure.len() {
        let id = crate::ast::StructId(i as u32);
        if arenas.as_name(id) == Some(spelling)
            && let Some(k) = by_id.get(&id.0)
        {
            kinds.push(k.clone());
        }
    }
    kinds
}

#[test]
fn highlight_classifies_a_builtin_constructor() {
    // `Some` and `None` are sum VARIANT constructors — read off the `(meta variant)` channel, not a
    // name match. `Option` (the type) is a TYPE. The distinction a lexical `Capitalized→type` pass
    // cannot make (it would paint all three the same).
    let src = "(module m (def (main) (match (Some 7) ((Some x) x) ((None u) 0))) (export main))";
    assert_eq!(
        highlight_kinds_of(src, "Some"),
        vec!["constructor", "constructor"]
    );
    assert_eq!(highlight_kinds_of(src, "None"), vec!["constructor"]);
}

#[test]
fn highlight_classifies_a_type_name() {
    // A type in annotation position AND a type-module operand both read as `type` — via `(meta t)` /
    // the type-constructor prim, not the leading capital.
    let src = "(module m (def (main) (: (List.len (list 1 2)) Int64)) (export main))";
    assert_eq!(highlight_kinds_of(src, "Int64"), vec!["type"]);
    // `List` heads a member access `(. List len)` → the OPERAND leaf is the type-module `List`.
    assert_eq!(highlight_kinds_of(src, "List"), vec!["type"]);
}

#[test]
fn highlight_classifies_functions_params_and_locals_distinctly() {
    // `inc` (a def with a parameter) → function; `x` (its parameter) → param, at BOTH the binder and
    // the reference; `main` (nullary def) referenced nowhere here; a `let` local → variable.
    let src = "(module m \
                   (def (inc (: x Int64)) (+ x 1)) \
                   (def (main) (let ((y 5)) (inc y))) \
                   (export main))";
    // `inc` — the def's NAME occurrence (a declaration of a function) AND the call site both read as
    // `function` (the name denotes the lambda either way). Two occurrences.
    assert_eq!(highlight_kinds_of(src, "inc"), vec!["function", "function"]);
    // `x`: the parameter binder + its use in the body — both `param`.
    assert_eq!(highlight_kinds_of(src, "x"), vec!["param", "param"]);
    // `y`: the let binder + its use — both `variable`.
    assert_eq!(highlight_kinds_of(src, "y"), vec!["variable", "variable"]);
}

#[test]
fn highlight_flags_an_unbound_name() {
    // An unbound reference (a typo) is `unbound` — the one classification a lexical tokenizer can
    // never make. `+` is a prelude operation → `function`.
    let src = "(module m (def (main) (+ 1 nope)) (export main))";
    assert_eq!(highlight_kinds_of(src, "nope"), vec!["unbound"]);
    assert_eq!(highlight_kinds_of(src, "+"), vec!["function"]);
}

#[test]
fn highlight_paints_quoted_data_as_symbol_not_unbound() {
    // Inside a (quasiquote …), a QUOTED name is inert DATA, not a live reference — so it must NOT be
    // painted `unbound` (error-red) even though it resolves to nothing. `reify_quotes` orphans the
    // original quoted children (they keep spans but detach from the root), so `classify_highlight`
    // reclassifies a DETACHED would-be-unbound leaf as `symbol`. Only the UNQUOTE hole is a live ref.
    // `mk`'s body is a valid quasiquote; `add`/`foo` are quoted data, `x` is the (bound) unquote hole.
    let src = "(module m (def (mk (: x Ast)) (quasiquote (add 1 (unquote x) foo))) (export mk))";
    assert_eq!(
        highlight_kinds_of(src, "add"),
        vec!["symbol"],
        "a quoted name is data, not an unbound typo"
    );
    assert_eq!(
        highlight_kinds_of(src, "foo"),
        vec!["symbol"],
        "a quoted name is data, not an unbound typo"
    );
    // The unquote hole `x` is a LIVE reference to the parameter — still classified as a param (it is
    // reachable from root, so the detached-orphan reclassification does not touch it). `x` appears
    // twice: the parameter binder occurrence + the unquote-hole use, both `param`.
    assert_eq!(highlight_kinds_of(src, "x"), vec!["param", "param"]);
}

#[test]
fn highlight_still_flags_a_reachable_unbound_typo() {
    // The quoted-data reclassification is NARROW: a genuine unbound typo in LIVE (root-reachable) code
    // stays `unbound` (red) — only DETACHED (quoted-orphan) leaves are softened to `symbol`.
    let src = "(module m (def (main) (nonexistent 1)) (export main))";
    assert_eq!(highlight_kinds_of(src, "nonexistent"), vec!["unbound"]);
}

#[test]
fn highlight_paints_a_well_formed_annotation_as_keyword() {
    // `db::strip_annotations` unwraps a `(@ NAME (def …))` wrapper IN PLACE and orphans the original
    // `@`/NAME occurrences (`parent_of == None`) — they keep source spans (so the highlighter paints
    // them) but resolve to nothing. `classify_highlight` recognizes a PARENTLESS `@` / known-annotation
    // name as an annotation token and paints it `keyword` (a decorator), not a generic `symbol` or a
    // spurious `unbound`. The `def` name and body still classify normally through resolution.
    let src = "(module m (@ test (def (t) (assert_eq 1 1 \"x\"))) (export t))";
    assert_eq!(highlight_kinds_of(src, "@"), vec!["keyword"]);
    assert_eq!(highlight_kinds_of(src, "test"), vec!["keyword"]);
    // A second known annotation spelling paints the same way.
    let src2 = "(module m (@ exhaustive (def (p (: n Int64)) (assert (> n 0) \"x\"))) (export p))";
    assert_eq!(highlight_kinds_of(src2, "exhaustive"), vec!["keyword"]);
}

#[test]
fn highlight_still_reds_an_annotation_that_wraps_no_definition() {
    // The annotation-token softening is NARROW: it fires only on the PARENTLESS occurrences an in-place
    // unwrap leaves behind. A `(@ NAME <non-def>)` is a genuine CDZ0201 (an annotation must wrap a def);
    // it is NOT rewritten, so its `@` KEEPS its parent → falls through to `unbound` (red), leaving the
    // real error visible rather than masking it as a decorator.
    let src = "(module m (def (main) (@ test 42)) (export main))";
    assert_eq!(highlight_kinds_of(src, "@"), vec!["unbound"]);
}

#[test]
fn highlight_paints_a_live_at_param_annotation_without_false_reds() {
    // A `@param(widget: slider) width : Int64` site parses to `(: (@ (param (: widget slider)) width)
    // Int64)` — a CALL-STYLE annotation on a PARAM binder, NOT wrapping a `(def …)`, so
    // `strip_annotations`' def-only unwrap never fires and the whole `(@ (param …) …)` form stays LIVE
    // and root-reachable (unlike the orphaned def-annotation case). Its `@` sigil + `param` head resolve
    // to nothing and its config kv leaves aren't a value scope, so a naive leaf walk paints FOUR tokens
    // `unbound` (error-red) on a program that compiles clean (the sidecar generates `Param`). The fix:
    // `@`/`param` paint `keyword` (a decorator); the config payload (`widget`/`slider`) softens to
    // `symbol` (inert metadata); the annotation TARGET (`width`, `Int64`) still classifies normally.
    let src = "(module m (: (@ (param (: widget slider)) width) Int64) \
                    (def (main) (host (Param) (Param.width))) (export main))";
    assert_eq!(highlight_kinds_of(src, "@"), vec!["keyword"]);
    assert_eq!(highlight_kinds_of(src, "param"), vec!["keyword"]);
    // Config payload — inert metadata, `symbol` not `unbound`.
    assert_eq!(highlight_kinds_of(src, "widget"), vec!["symbol"]);
    assert_eq!(highlight_kinds_of(src, "slider"), vec!["symbol"]);
    // The annotation TARGET still resolves — the declared type is a `type`, unchanged by the softening.
    assert_eq!(highlight_kinds_of(src, "Int64"), vec!["type"]);
}

#[test]
fn highlight_paints_a_tag_annotation_without_false_reds() {
    // A call-style `@tag("slow") def` reifies to `(@ (tag "slow") (def …))` — UNLIKE `@param`, the
    // `@tag` annotation DOES wrap a `(def …)`, so `db::strip_annotations` UNWRAPS it in place (the def
    // adopts the inner children) and ORPHANS both the `@` sigil AND the `(tag "slow")` application (they
    // keep source spans but their ancestor chain no longer reaches root). The `@` is caught as an
    // annotation token → `keyword`; the orphaned `tag` head + its `"slow"` string are inert (they resolve
    // to nothing and are unreachable from root), so the quoted-data `reaches_root` stopgap softens the
    // would-be-`unbound` `tag` head to `symbol` (data) rather than error-red. The tagged def + its body
    // still classify normally through resolution. This PINS the invariant that a valid `@tag` program
    // shows NO false-red highlight (a def-wrapping call-style annotation is the twin of the `@param`
    // false-red case, which needed a fix; `@tag` is already safe via the orphan/stopgap paths).
    //
    // NOTE on the `tag` head colour: it reads `symbol`, not `keyword`. Painting it `keyword` (a decorator,
    // matching the `@` sigil) is a cosmetic nicety, but the orphaned `(tag …)` app is STRUCTURALLY
    // IDENTICAL to a quoted `(tag …)` data list (both detach from root after their respective in-place
    // rewrites), so a keyword-paint keyed on "detached list headed `tag`" would misclassify quoted data —
    // an unsound heuristic for a purely-cosmetic gain (both colours are non-red). So `symbol` is the
    // deliberate, sound classification; this test locks it so a future change can't silently flip it.
    let src = "(module m (@ (tag \"slow\") (def (t) (+ 1 1))) (export t))";
    // The `@` sigil is an annotation decorator.
    assert_eq!(highlight_kinds_of(src, "@"), vec!["keyword"]);
    // The `tag` head of the orphaned call-style application — inert data, `symbol` not `unbound`.
    assert_eq!(highlight_kinds_of(src, "tag"), vec!["symbol"]);
    // The tagged def name still resolves — a nullary def reads as `variable` (a def WITH parameters
    // reads `function`; `t` takes none), at both the def occurrence and the export reference. Never red.
    assert_eq!(highlight_kinds_of(src, "t"), vec!["variable", "variable"]);
    // NOTHING in a clean `@tag` program is painted error-red.
    let out = compile(&inputs(src, &[Request::Query(Query::Highlight)]), &[]);
    let pairs = highlight_pairs(&out);
    assert!(
        !pairs.iter().any(|(_, k)| k == "unbound"),
        "no token in a clean @tag program is unbound:\n{pairs:?}"
    );
}

#[test]
fn highlight_paints_stacked_annotations_without_false_reds() {
    // STACKED annotations on one def — `@test @tag("slow") (def …)` reifies to
    // `(@ test (@ (tag "slow") (def …)))`, nesting the wrappers. `db::strip_annotations` unwraps the
    // INNER `(@ (tag …) def)` first (the def adopts its children), then the OUTER `(@ test …)`, orphaning
    // BOTH `@` sigils, the `test` name, AND the `(tag "slow")` app — several detached tokens over one
    // def. This is the multi-annotation edge the single-annotation tests don't reach: it must still show
    // NO false-red (each `@`→keyword via the parentless-`@` path, `test`→keyword via KNOWN_ANNOTATIONS,
    // the orphaned `tag` head→symbol via the quoted-data stopgap), and the def + body classify normally.
    let src = "(module m (@ test (@ (tag \"slow\") (def (t) (+ 1 1)))) (export t))";
    // Both `@` sigils are decorators. (Two occurrences, one per stacked annotation.)
    assert_eq!(highlight_kinds_of(src, "@"), vec!["keyword", "keyword"]);
    // The known-annotation name `test` is a keyword.
    assert_eq!(highlight_kinds_of(src, "test"), vec!["keyword"]);
    // The call-style `tag` head is inert data (orphaned app) — `symbol`, not error-red.
    assert_eq!(highlight_kinds_of(src, "tag"), vec!["symbol"]);
    // The doubly-annotated def still resolves — nullary def reads `variable`, at the def + export ref.
    assert_eq!(highlight_kinds_of(src, "t"), vec!["variable", "variable"]);
    // NOTHING in a clean stacked-annotation program is painted error-red.
    let out = compile(&inputs(src, &[Request::Query(Query::Highlight)]), &[]);
    let pairs = highlight_pairs(&out);
    assert!(
        !pairs.iter().any(|(_, k)| k == "unbound"),
        "no token in a clean stacked-annotation program is unbound:\n{pairs:?}"
    );
}

#[test]
fn highlight_kind_all_is_the_complete_1to1_wire_vocabulary() {
    use crate::sidecar::HighlightKind;
    // `HighlightKind::ALL` is the canonical vocabulary a downstream consumer (the `cdz lsp` semantic-
    // token legend) iterates to prove it handles EVERY kind. Pin two invariants so it can't rot:
    // (1) ALL is COMPLETE — a `match` over a representative forces a compile error if a variant is
    //     added without extending ALL (the arm below is exhaustive, so a new variant breaks the build
    //     here, the single place that must be updated alongside the enum);
    // (2) the wire spellings are DISTINCT and 1:1 with ALL (no two kinds share a theme token, and
    //     every entry has a spelling).
    // Exhaustiveness guard: this match must list every variant. Adding a `HighlightKind` fails to
    // compile until it is added BOTH here and to `ALL` — the forcing function the comment promises.
    for &k in HighlightKind::ALL {
        match k {
            HighlightKind::Keyword
            | HighlightKind::Type
            | HighlightKind::Constructor
            | HighlightKind::Function
            | HighlightKind::Param
            | HighlightKind::Variable
            | HighlightKind::Effect
            | HighlightKind::Label
            | HighlightKind::Number
            | HighlightKind::Str
            | HighlightKind::Char
            | HighlightKind::Bytes
            | HighlightKind::Symbol
            | HighlightKind::Literal
            | HighlightKind::Unbound => {}
        }
    }
    // Distinct, non-empty wire spellings, one per ALL entry.
    let spellings: Vec<&str> = HighlightKind::ALL.iter().map(|k| k.as_str()).collect();
    assert!(
        spellings.iter().all(|s| !s.is_empty()),
        "every highlight kind has a wire spelling"
    );
    let unique: std::collections::BTreeSet<&str> = spellings.iter().copied().collect();
    assert_eq!(
        unique.len(),
        spellings.len(),
        "wire spellings must be DISTINCT (no two kinds share a theme token): {spellings:?}"
    );
}

#[test]
fn symbol_kind_all_is_the_complete_1to1_wire_vocabulary() {
    use crate::sidecar::SymbolKind;
    // `SymbolKind::ALL` is the canonical symbols-query vocabulary a downstream consumer (an LSP
    // document-outline / breadcrumb icon legend) iterates to prove it handles EVERY declaration
    // kind. Same two invariants as the `HighlightKind` guard above, so it can't rot:
    // (1) ALL is COMPLETE — the exhaustive `match` fails to compile if a variant is added without
    //     extending ALL (this and `ALL` are the single pair to update alongside the enum);
    // (2) the wire spellings are DISTINCT and 1:1 with ALL (no two kinds share an outline token,
    //     and every entry has a spelling).
    for &k in SymbolKind::ALL {
        match k {
            SymbolKind::Value
            | SymbolKind::Function
            | SymbolKind::Type
            | SymbolKind::Effect
            | SymbolKind::Module => {}
        }
    }
    let spellings: Vec<&str> = SymbolKind::ALL.iter().map(|k| k.as_str()).collect();
    assert!(
        spellings.iter().all(|s| !s.is_empty()),
        "every symbol kind has a wire spelling"
    );
    let unique: std::collections::BTreeSet<&str> = spellings.iter().copied().collect();
    assert_eq!(
        unique.len(),
        spellings.len(),
        "wire spellings must be DISTINCT (no two kinds share an outline token): {spellings:?}"
    );
}

#[test]
fn highlight_colours_literals_by_kind() {
    // Each literal leaf carries its own kind; a keyword head is `keyword`.
    let src = "(module m (def (main) (if true 42 0)) (export main))";
    assert_eq!(highlight_kinds_of(src, "if"), vec!["keyword"]);
    // `true`/`42`/`0` are non-NAME leaves (Bool / Int), so the by-name helper can't find them; check
    // the raw artifact carries a `literal` (the bool) and a `number` (the ints).
    let out = compile(&inputs(src, &[Request::Query(Query::Highlight)]), &[]);
    let pairs = highlight_pairs(&out);
    assert!(
        pairs.iter().any(|(_, k)| k == "literal"),
        "the bool literal is classified: {pairs:?}"
    );
    assert!(
        pairs.iter().any(|(_, k)| k == "number"),
        "a number token is classified: {pairs:?}"
    );
}

#[test]
fn highlight_colours_char_bytes_and_symbol_literals_by_their_constant_kind() {
    // The literal classifier colours each constant leaf by its RESOLVED kind, not its spelling. The
    // char (`#\a`), byte-string (`b"…"`), and symbol (`#"…"`) literal forms are distinct kinds that
    // the by-kind literal path emits — thinly covered before this pin (the earlier literals test only
    // exercised bool + number), so a regression collapsing one into `string`/`symbol`/`unbound` would
    // go unnoticed. Assert each emits its OWN wire spelling. (These are non-NAME leaves, so the
    // by-name helper can't find them — read the raw artifact, like the bool/number test.)
    let each = |src: &str, kind: &str| {
        let out = compile(&inputs(src, &[Request::Query(Query::Highlight)]), &[]);
        let pairs = highlight_pairs(&out);
        assert!(
            pairs.iter().any(|(_, k)| k == kind),
            "expected a `{kind}` token in:\n{pairs:?}"
        );
        // A well-formed literal program never paints error-red.
        assert!(
            !pairs.iter().any(|(_, k)| k == "unbound"),
            "a clean literal program has no unbound token:\n{pairs:?}"
        );
    };
    // A CHAR literal `#\a`.
    each("(module m (def (main) #\\a) (export main))", "char");
    // A BYTE-STRING literal `b\"hi\"`.
    each("(module m (def (main) b\"hi\") (export main))", "bytes");
    // A SYMBOL literal `#\"sym\"` — the literal form (distinct from a name taken as data).
    each("(module m (def (main) #\"sym\") (export main))", "symbol");
}

#[test]
fn highlight_does_not_flag_binder_declarations_as_unbound() {
    // A binding DECLARATION (a module name, a variant-pattern PAYLOAD binder) is not a reference — it
    // must NOT read as `unbound` on a clean program. Regression guard: a whole-program leaf walk that
    // resolved these as value lookups reported a spurious `unbound` (they bind before they resolve).
    let src = "(module m (def (main) (match (Some 7) ((Some v) v) ((None u) 0))) (export main))";
    // The module name binds the wrapper — a `variable`, never `unbound`.
    assert_eq!(highlight_kinds_of(src, "m"), vec!["variable"]);
    // `v`: the payload binder in `(Some v)` + its use in the arm body — both `variable`, none unbound.
    let v = highlight_kinds_of(src, "v");
    assert!(
        !v.is_empty() && v.iter().all(|k| k == "variable"),
        "payload binder + use are variables, not unbound: {v:?}"
    );
    // `u`: the `(None u)` payload binder (unused) — a `variable`, not unbound.
    assert_eq!(highlight_kinds_of(src, "u"), vec!["variable"]);
}

#[test]
fn highlight_classifies_the_effect_construct_not_as_unbound() {
    // An effect declaration + a handler: the declaration/effect-form HEADS (`effect`/`op`/`handle`/
    // `resume`) are keywords, the effect NAME + its OPERATION name are `effect`, and NOTHING is
    // `unbound`. Regression guard: the effect-syntax change left `effect`/`op` out of the highlighter's
    // keyword set (→ their heads + the op name painted `unbound`), and `desugar_handles` orphaned the
    // `handle` head (parent lost → also painted `unbound`) — spurious red squiggles under every effect
    // example in the guide, though the program compiles clean.
    let src = "(module m \
                   (effect Ask (op ask (-> Unit Int64))) \
                   (def (main) (handle Ask unit ((ask (u) s (resume 42 s))) (Ask.ask))) \
                   (export main))";
    let out = compile(&inputs(src, &[Request::Query(Query::Highlight)]), &[]);
    let pairs = highlight_pairs(&out);
    assert!(
        !pairs.iter().any(|(_, k)| k == "unbound"),
        "no token in a clean effect program is unbound:\n{pairs:?}"
    );
    // The declaration/control heads are keywords.
    assert_eq!(highlight_kinds_of(src, "effect"), vec!["keyword"]);
    assert_eq!(highlight_kinds_of(src, "op"), vec!["keyword"]);
    assert_eq!(highlight_kinds_of(src, "handle"), vec!["keyword"]);
    assert_eq!(highlight_kinds_of(src, "resume"), vec!["keyword"]);
    // The effect NAME (declaration + the `handle E` head) and the OPERATION name are `effect`. `ask`
    // occurs as the op-signature name AND the arm op; both read `effect` (the member key `Ask.ask` is
    // a `label`, so the trailing `ask` there is not counted here).
    assert!(
        highlight_kinds_of(src, "Ask").iter().all(|k| k == "effect"),
        "the effect name is `effect` everywhere it is a value: {:?}",
        highlight_kinds_of(src, "Ask")
    );
    assert!(
        highlight_kinds_of(src, "ask").contains(&"effect".to_string()),
        "the effect operation name reads as `effect`: {:?}",
        highlight_kinds_of(src, "ask")
    );
}

#[test]
fn highlight_treats_a_native_record_field_label_like_the_alias() {
    // M3 parity (native-recognition, token classification): a NATIVE `#record((= x 1))` field name is a
    // DATA label exactly like the `(record (x 1))` alias. `is_label_position` recognized the field label
    // only via `compound_ctor` (string head) + a 2-element positional entry, so a native record's field
    // name — sitting at the KEY of a 3-element FieldPair `(= x 1)` under a native ctor-leaf head — was
    // matched by NEITHER, and painted a spurious `unbound` instead of `label`. Now the grandparent is
    // recognized via `compound_form_of` and the FieldPair key is read as the label. Native ≡ alias:
    let native = "(module m (def (main) (. #record((= x 1) (= y 2)) x)) (export main))";
    let legacy = "(module m (def (main) (. (record (x 1) (y 2)) x)) (export main))";
    for (label, src) in [("native #record", native), ("alias record", legacy)] {
        let kinds = highlight_kinds_of(src, "x");
        assert!(
            !kinds.is_empty() && kinds.iter().all(|k| k == "label"),
            "{label}: a record field name (and member key) are labels, not unbound: {kinds:?}"
        );
    }
}

#[test]
fn highlight_treats_a_record_field_and_member_key_as_labels() {
    // A record field name and a member-access key are DATA (symbols), never resolved to a value — so
    // they are `label`, not a spurious unbound name.
    let src = "(module m (def (main) (. (record (x 1) (y 2)) x)) (export main))";
    // `x` appears as a record field name AND as the member key — both `label` (never `unbound`).
    let kinds = highlight_kinds_of(src, "x");
    assert!(
        !kinds.is_empty() && kinds.iter().all(|k| k == "label"),
        "record field / member key are labels: {kinds:?}"
    );
}

/// The raw `Instantiations` query text for `name` over `src` (all lines — `disp` + `inst`).
fn instantiations_text_of(src: &str, name: &str) -> String {
    let out = compile(
        &inputs(
            src,
            &[Request::Query(Query::Instantiations { name: name.into() })],
        ),
        &[],
    );
    assert!(
        !out.has_error(),
        "an instantiations query does not fail: {:?}",
        out.diagnostics
    );
    // The instantiations artifact is now canonical binary AST (operator P0 seq-284); decode it and
    // reconstruct the historical `disp`/`inst` TAB text so the existing parsing helpers/assertions
    // still pin the report shape. An unknown name (known=false) → empty, as the old empty artifact was.
    let Some(a) = out.artifacts.iter().find(|a| a.kind == KIND_INSTANTIATIONS) else {
        return String::new();
    };
    let report = cadenza_compile_abi::instantiations_wire::decode(&a.bytes)
        .expect("instantiations artifact decodes as binary AST");
    if !report.known {
        return String::new();
    }
    let node = report
        .name_node
        .map_or_else(|| "-".to_string(), |n| n.to_string());
    let mut text = format!("disp\t{node}\t{}\n", report.dispositions.join("+"));
    for inst in &report.instances {
        text.push_str(&format!(
            "inst\t{}\t{node}\t{}\n",
            inst.spec_name,
            inst.args.join(";")
        ));
    }
    text
}

/// The DISPOSITION of `name` — the `disp` line's third column (e.g. `specialized` / `inlined` /
/// `emitted` / `unreferenced` / `transformed→f$acc`). Empty string if there is no `disp` line (an
/// unknown name). The `transformed→` copy suffix embeds a `$acc` name (stable across runs).
fn disposition_of(src: &str, name: &str) -> String {
    instantiations_text_of(src, name)
        .lines()
        .find_map(|l| {
            let mut c = l.split('\t');
            (c.next() == Some("disp")).then(|| c.nth(1).unwrap_or("").to_string())
        })
        .unwrap_or_default()
}

/// The `arg;arg;…` field (the concrete per-argument instantiation) of each `inst` line, sorted —
/// dropping the tag, the synthesized spec name (`#mono<N>` embeds `db.defs.len()`, unstable) and the
/// node id. Empty when the def is not specialized.
fn instantiation_args(src: &str, name: &str) -> Vec<String> {
    let text = instantiations_text_of(src, name);
    let mut args: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            // `inst<TAB>spec<TAB>node<TAB>args` — the args are the 4th column.
            (cols.first() == Some(&"inst")).then(|| cols.get(3).copied().unwrap_or("").to_string())
        })
        .collect();
    args.sort();
    args
}

#[test]
fn a_recursive_generic_reports_one_instantiation_per_concrete_type() {
    // `loopn` threads a generic `x`; called at Int64 AND String it monomorphizes into two functions
    // (the rep-sensitive case from the recursive-generic design). The query enumerates BOTH, each with
    // its concrete per-parameter types — the reverse of "one source def, one function".
    let src = "(module m \
                   (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x))) \
                   (def (main (: a Int64)) (+ (loopn 3 a) (String.scalar-len (loopn 2 \"hi\")))) \
                   (export main))";
    assert_eq!(
        instantiation_args(src, "loopn"),
        vec!["n: Int64;x: Int64", "n: Int64;x: String"],
    );
}

#[test]
fn a_non_generic_definition_has_no_instantiations() {
    // A monomorphic def is never specialized — it emits no instantiation records (no `inst` line).
    let src = "(module m (def (f (: x Int64)) x) (def (main) (f 1)) (export main))";
    assert!(instantiation_args(src, "f").is_empty());
    // A non-recursive generic is INLINED (β-reduced) at each call site, emitting no shared function —
    // so it, too, reports no instantiation (a documented boundary of the query).
    let inl = "(module m (def (ident v) v) \
                   (def (main (: x Int64)) (+ (ident x) (ident 1))) (export main))";
    assert!(instantiation_args(inl, "ident").is_empty());
}

#[test]
fn an_instantiations_query_for_an_unknown_name_is_total() {
    // A name that names no definition yields the EMPTY result (no `disp` line at all), never an error.
    let src = "(module m (def (main) 42) (export main))";
    assert!(instantiations_text_of(src, "ghost").is_empty());
    assert!(disposition_of(src, "ghost").is_empty());
}

#[test]
fn disposition_reports_how_each_definition_was_compiled() {
    // The query reports a def's DISPOSITION — what the compiler DID with it — for every kind:
    //   - a NON-RECURSIVE function is INLINED (β-reduced away, no standalone function);
    //   - a TREE recursion (not accumulable, runtime arg) is EMITTED as one standalone function + called;
    //   - a RECURSIVE GENERIC is SPECIALIZED (monomorphized per type);
    //   - a LINEAR recursion is TRANSFORMED into an accumulator loop (`f$acc`);
    //   - an EXPORT is EMITTED (a boundary function);
    //   - a def nothing references is UNREFERENCED.
    let inl = "(module m (def (ident v) v) \
                   (def (main (: x Int64)) (+ (ident x) (ident 1))) (export main))";
    assert_eq!(disposition_of(inl, "ident"), "inlined");
    assert_eq!(disposition_of(inl, "main"), "emitted");

    // A TREE recursion with a runtime argument cannot fold or accumulate → a standalone emitted function.
    let tree = "(module m \
                    (def (fib (: n Int64)) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2))))) \
                    (def (main (: k Int64)) (fib k)) (export main))";
    assert_eq!(disposition_of(tree, "fib"), "emitted");

    // A RECURSIVE GENERIC is specialized (its instances are also listed).
    let generic = "(module m \
                   (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x))) \
                   (def (main (: a Int64)) (+ (loopn 3 a) (String.scalar-len (loopn 2 \"hi\")))) \
                   (export main))";
    assert_eq!(disposition_of(generic, "loopn"), "specialized");

    // A LINEAR non-tail recursion is rewritten into an accumulator loop — reported `transformed→NAME`
    // (the source's own body folds to a seed of the copy). The copy is named `<orig>$acc`.
    let acc = "(module m \
                   (def (sm (: n Int64)) (if (= n 0) 0 (+ n (sm (- n 1))))) \
                   (def (main (: k Int64)) (sm k)) (export main))";
    assert_eq!(disposition_of(acc, "sm"), "transformed→sm$acc");

    // A def nothing references at all is unreferenced.
    let dead = "(module m (def (dead (: x Int64)) (+ x 1)) (def (main) 0) (export main))";
    assert_eq!(disposition_of(dead, "dead"), "unreferenced");
}

#[test]
fn a_type_valued_parameter_reports_the_erased_type_argument() {
    // A recursive generic with a `(: t Type)` type-valued parameter monomorphizes per passed type; the
    // type arg is compile-time-only (ERASED from the runtime signature), so the query renders it as an
    // erased `const t = TYPE` and keeps the list parameter as `l: (Lst TYPE)`.
    let src = "(module m (type Lst Nil (Cons a (Lst a))) \
                   (def (len (: t Type) (: l (Lst t))) \
                     (match l ((Lst.Nil) 0) ((Lst.Cons h tl) (+ 1 (len t tl))))) \
                   (def (main) (+ (len Int64 (Lst.Cons 1 (Lst.Cons 2 Lst.Nil))) \
                                  (len String (Lst.Cons \"a\" Lst.Nil)))) \
                   (export main))";
    assert_eq!(
        instantiation_args(src, "len"),
        vec![
            "const t = Int64;l: (Lst Int64)",
            "const t = String;l: (Lst String)",
        ],
    );
}

#[test]
fn ad_hoc_polymorphism_reports_the_inlined_dictionary_per_instance() {
    // The ad-hoc-polymorphism case: a `const` dictionary parameter is inlined + erased at each call, so
    // `fold-n` monomorphizes once per DISTINCT dictionary. The query shows WHICH concrete dictionary
    // each instance baked in (the distinguishing data), rendered as the inlined source — not an opaque
    // fingerprint — while `n`/`acc` stay ordinary runtime parameters.
    let src = "(module m \
                   (def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64)) \
                     (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc)))) \
                   (def (main) (+ (fold-n (record (op (fn (x) (+ x 10)))) 3 0) \
                                  (fold-n (record (op (fn (x) (* x 2)))) 3 1))) \
                   (export main))";
    assert_eq!(
        instantiation_args(src, "fold-n"),
        vec![
            "const d = (record (op (fn (x) (* x 2))));n: Int64;acc: Int64",
            "const d = (record (op (fn (x) (+ x 10))));n: Int64;acc: Int64",
        ],
    );
}

#[test]
fn two_calls_at_the_same_instantiation_dedup_to_one() {
    // The specialization is memoized on the concrete instantiation, so two calls with the SAME
    // dictionary share ONE function — the query reports a single instance (no double-count).
    let src = "(module m \
                   (def (fold-n (const (: d (Record (op (-> Int64 Int64))))) (: n Int64) (: acc Int64)) \
                     (if (= n 0) acc (fold-n d (- n 1) ((. d op) acc)))) \
                   (def (main) (+ (fold-n (record (op (fn (x) (+ x 10)))) 3 0) \
                                  (fold-n (record (op (fn (x) (+ x 10)))) 2 5))) \
                   (export main))";
    assert_eq!(
        instantiation_args(src, "fold-n"),
        vec!["const d = (record (op (fn (x) (+ x 10))));n: Int64;acc: Int64"],
    );
}
