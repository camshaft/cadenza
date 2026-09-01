//! Delegated compilation — reach the compiler by SPAWNING the standalone `cdz-compile` process
//! instead of linking `rcdzc` in-process. Compiled when the `standalone` feature is OFF
//! (`design/DESIGN-cdz-delegate-compile.md`): the default (`standalone` ON) bundles the compiler
//! in-process, and the NIX build packages `cdz` with `--no-default-features` so it delegates — then a
//! compiler-only change need not rebuild `cdz` and `cdz` + the compiler cache/rebuild independently.
//!
//! Why this is behavior-identical: `cdz-compile` IS `rcdzc_cli::run` under its own name (a thin shim
//! over the same `CompileArgs` + `run` → `run_prepared`). So delegating is not a reimplementation — we
//! hand `cdz-compile` the SAME named artifacts and flags the in-process path would, and it executes the
//! exact host-boundary compile+report+write. In particular the diagnostics reporter runs THERE, over
//! the same `spans` artifacts we pass through, so a source-file compile's located `path:line:col`
//! errors come out byte-identical to the in-process path. Child stdio is inherited, so a `-o -` stdout
//! pipe and stderr diagnostics flow straight to the caller; the child's exit status is forwarded.
//!
//! This module is compiled ONLY under the feature; the in-process path (`compiler_cli::run` /
//! `run_prepared`) stays the default, so a dev/interactive `cdz` never pays the subprocess cost.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

// The compile-BOUNDARY types come from the shared `cadenza-compile-abi` crate (not via `rcdzc`'s
// re-exports) — the first step of making `cdz` speak the boundary vocabulary WITHOUT linking `rcdzc` in a
// `!standalone` build. `cadenza_compile_abi::{Artifact, OptLevel, Target}` are the identical definitions
// `rcdzc` produces + `pub use`s, so this is byte-identical here; the remaining fully-qualified `rcdzc::`
// boundary refs (Request/CompileOutput/sidecar) repoint in follow-up slices now that the dep edge exists.
use cadenza_compile_abi::{Artifact, OptLevel, Target};

/// Locate the `cdz-compile` binary to delegate to. Resolution order, most-specific first:
///  1. `$CDZ_COMPILE_BIN` — an explicit path (how the nix package injects the content-addressed
///     compiler bin, so the two derivations stay independently cached with no `$PATH`/CWD ambiguity).
///  2. beside THIS `cdz` — `current_exe().parent()/cdz-compile[.exe]`, the co-built location (the same
///     convention the `cdz smith`/`cdz cad` passthroughs use via [`crate::locate_sibling_bin`]).
///  3. bare `cdz-compile` — resolved on `$PATH` by `Command::new` (a system install).
fn locate() -> PathBuf {
    if let Some(p) = std::env::var_os("CDZ_COMPILE_BIN") {
        return PathBuf::from(p);
    }
    crate::locate_sibling_bin("cdz-compile")
        .unwrap_or_else(|| PathBuf::from(crate::bin_name("cdz-compile")))
}

/// The `--target` value string for a backend [`Target`] — matched to `rcdzc::cli`'s `TargetArg` clap
/// spelling (kebab-case), so the delegated flag parses identically to the in-process one.
fn target_cli(t: Target) -> &'static str {
    match t {
        Target::Wasm => "wasm",
        Target::WasmDebug => "wasm-debug",
        Target::Dwarf => "dwarf",
        Target::Rust => "rust",
        Target::RustAsync => "rust-async",
        Target::Cadenza => "cadenza",
    }
}

/// The `--opt-level` value string for an [`OptLevel`] — matched to `rcdzc::cli`'s `OptLevelArg` spelling.
fn opt_cli(o: OptLevel) -> &'static str {
    match o {
        OptLevel::O0 => "o0",
        OptLevel::O1 => "o1",
        OptLevel::O2 => "o2",
        OptLevel::O3 => "o3",
    }
}

/// The `--overflow-signed`/`--overflow-unsigned` value string for an [`cadenza_compile_abi::OverflowMode`] —
/// matched to `rcdzc::cli`'s `OverflowModeArg` spelling.
fn overflow_cli(m: cadenza_compile_abi::OverflowMode) -> &'static str {
    match m {
        cadenza_compile_abi::OverflowMode::Trap => "trap",
        cadenza_compile_abi::OverflowMode::Wrap => "wrap",
    }
}

/// Append the GLOBAL overflow policy (`--overflow-signed`/`--overflow-unsigned`) to a `cdz-compile`
/// command, one flag per present side. An absent side (`None`) emits NOTHING — so `cdz-compile` sees no
/// flag and applies its own `None` (fall through to the built-in `Trap`), preserving the precedence
/// (module pragma > this global > trap) exactly as the in-process path does.
fn append_overflow_flags(cmd: &mut Command, overflow: cadenza_compile_abi::OverflowSpec) {
    if let Some(m) = overflow.signed {
        cmd.arg("--overflow-signed").arg(overflow_cli(m));
    }
    if let Some(m) = overflow.unsigned {
        cmd.arg("--overflow-unsigned").arg(overflow_cli(m));
    }
}

/// Spawn `cdz-compile` with `input_specs` (each a `path` / `name=path` / `kind:name=path` the compiler
/// CLI parses), the resolved `targets` (empty ⇒ let `cdz-compile` apply its own `[Wasm]` default),
/// optional `-o out`, and `--opt-level`. Optional `--entry`/`--component-name` flags for the
/// artifacts-in path (the source path bakes these into the artifact stream instead). Inherits stdio and
/// forwards the child's exit status; a spawn failure prints an actionable, `prog`-labelled error.
#[allow(clippy::too_many_arguments)]
fn spawn(
    input_specs: &[String],
    targets: &[Target],
    out: Option<&Path>,
    entry: Option<&str>,
    component_name: Option<&str>,
    opt_level: OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
    prog: &str,
) -> ExitCode {
    let program = locate();
    let mut cmd = Command::new(&program);
    cmd.args(input_specs);
    for t in targets {
        cmd.arg("--target").arg(target_cli(*t));
    }
    if let Some(o) = out {
        cmd.arg("-o").arg(o);
    }
    if let Some(e) = entry {
        cmd.arg("--entry").arg(e);
    }
    if let Some(c) = component_name {
        cmd.arg("--component-name").arg(c);
    }
    cmd.arg("--opt-level").arg(opt_cli(opt_level));
    append_overflow_flags(&mut cmd, overflow);

    match cmd.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => crate::exit_code_from_child(status.code()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "{prog}: cdz-compile not found (looked at $CDZ_COMPILE_BIN, beside `cdz`, then \
                 $PATH) — the `delegate-compile` feature spawns the compiler instead of linking it; \
                 build it with `cargo build -p rcdzc-cli --bin cdz-compile` and re-run"
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!(
                "{prog}: could not run cdz-compile ({}): {e}",
                program.display()
            );
            ExitCode::FAILURE
        }
    }
}

/// Delegate a PURE ARTIFACTS-IN `cdz compile` (no source-file input): forward the raw input specs +
/// `--entry`/`--component-name` flags + targets/`-o`/`--opt-level` verbatim to `cdz-compile`. The
/// specs already name on-disk artifacts (or `-` for stdin, whose stream the inherited stdin carries),
/// so no temp files are needed — this is the same `run` the standalone `rcdzc`/`cdz-compile` bin does.
pub fn delegate_args(args: &crate::compile_args::CompileArgs, prog: &str) -> ExitCode {
    spawn(
        args.input_specs(),
        &args.targets(),
        args.out_path().as_deref(),
        args.entry(),
        args.component_name(),
        args.opt_level(),
        args.overflow_spec(),
        prog,
    )
}

/// Delegate a compile of ALREADY-PREPARED input artifacts (the source-file path, where `cdz` has
/// parsed each source in-process into `ast` + `spans` artifacts and appended any `entry`/
/// `component-name` artifacts). Each artifact is materialized to a temp file and handed to
/// `cdz-compile` as a `kind:name=path` spec — including the `entry`/`component-name` artifacts, which
/// `cdz-compile` reads from the artifact stream exactly as if `--entry`/`--component-name` were passed
/// (so no flags are needed here). The temp directory is removed on every exit path.
pub fn delegate_from_artifacts(
    inputs: &[Artifact],
    targets: &[Target],
    out: Option<&Path>,
    opt_level: OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
    prog: &str,
) -> ExitCode {
    let dir = scratch_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("{prog}: cannot create temp dir {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    let _guard = crate::RemoveOnDrop::dir(dir.clone());

    let Some(specs) = materialize_specs(inputs, &dir, prog) else {
        return ExitCode::FAILURE;
    };
    spawn(&specs, targets, out, None, None, opt_level, overflow, prog)
}

/// Delegate an IN-MEMORY project compile-to-BYTES (the quiet `cdz run <project>` / `cdz test` build):
/// materialize the prepared `inputs` (ast/spans/entry/component-name) to temp files and spawn
/// `cdz-compile … --target wasm -o -`, CAPTURING stdout — the `-o -` path writes ONLY the single
/// `component` artifact's bytes to stdout (no `cdz: wrote …` notice), matching the in-process path's
/// quiet-on-success contract — while stderr is inherited so located diagnostics still surface on failure.
/// `Ok(Some(bytes))` = a component was produced; `Ok(None)` = compiled but produced none (empty stdout,
/// clean exit); `Err(())` = a compile/spawn failure (diagnostics already on stderr). The in-process
/// counterpart is `rcdzc::compile_with_opt` → the `component`-kind artifact's bytes.
pub fn delegate_project_to_bytes(
    inputs: &[Artifact],
    opt_level: OptLevel,
    overflow: cadenza_compile_abi::OverflowSpec,
    prog: &str,
) -> Result<Option<Vec<u8>>, ()> {
    let dir = scratch_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("{prog}: cannot create temp dir {}: {e}", dir.display());
        return Err(());
    }
    let _guard = crate::RemoveOnDrop::dir(dir.clone());

    let Some(specs) = materialize_specs(inputs, &dir, prog) else {
        return Err(());
    };

    let program = locate();
    let mut cmd = Command::new(&program);
    cmd.args(&specs)
        .arg("--target")
        .arg("wasm")
        .arg("-o")
        .arg("-")
        .arg("--opt-level")
        .arg(opt_cli(opt_level))
        // Capture the bytes on stdout; let diagnostics flow to the inherited stderr; no stdin.
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdin(Stdio::null());
    append_overflow_flags(&mut cmd, overflow);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "{prog}: cdz-compile not found (looked at $CDZ_COMPILE_BIN, beside `cdz`, then \
                 $PATH) — a delegating (`--no-default-features`) `cdz` spawns the compiler instead of \
                 linking it; build it with `cargo build -p rcdzc-cli --bin cdz-compile` and re-run"
            );
            return Err(());
        }
        Err(e) => {
            eprintln!(
                "{prog}: could not run cdz-compile ({}): {e}",
                program.display()
            );
            return Err(());
        }
    };
    // `wait_with_output` reads the piped stdout; stderr was inherited (not piped), so it flows to the
    // terminal and `output.stderr` is empty — exactly the quiet-on-success / diagnostics-on-failure split.
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{prog}: could not read cdz-compile output: {e}");
            return Err(());
        }
    };
    if !output.status.success() {
        // A compile error — cdz-compile already reported located diagnostics on the inherited stderr.
        return Err(());
    }
    // Clean exit: non-empty stdout is the component's bytes; empty means no `component` artifact
    // (a diagnostic-only run), the same as the in-process `Ok(None)`.
    if output.stdout.is_empty() {
        Ok(None)
    } else {
        Ok(Some(output.stdout))
    }
}

/// A per-process-unique scratch directory (pid + a monotonic counter, so concurrent `cdz` compiles in
/// the same process don't collide — no wall-clock/rng needed). The caller creates it + guards removal.
fn scratch_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "cdz-delegate-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Write each prepared artifact into `dir` and return its `cdz-compile` `kind:name=path` input spec, or
/// `None` (after printing a `prog`-labelled error) on a write failure. The file stem is just the index —
/// the `kind:name=` prefix carries the artifact's identity, so the stem need only be unique.
fn materialize_specs(inputs: &[Artifact], dir: &Path, prog: &str) -> Option<Vec<String>> {
    let mut specs: Vec<String> = Vec::with_capacity(inputs.len());
    for (i, a) in inputs.iter().enumerate() {
        let path = dir.join(i.to_string());
        if let Err(e) = std::fs::write(&path, &a.bytes) {
            eprintln!("{prog}: cannot write temp artifact {}: {e}", path.display());
            return None;
        }
        specs.push(format!("{}:{}={}", a.kind, a.name, path.display()));
    }
    Some(specs)
}

// ── sidecar QUERY delegation ───────────────────────────────────────────────────────────────────────

/// Build the sidecar REQUEST as a binary-AST value — the delegating-driver counterpart of
/// `rcdzc::sidecar::encode`. Since the operator unified the request onto the binary AST (#3440), the
/// request is a `cdzast` tree (`root = Ast.List` of per-request forms) encoded with the FRONT-END's codec
/// copy (`cadenza_syntax::codec`), which is byte-identical to rcdzc's `crate::codec` (the copy-don't-depend
/// invariant) — so the bytes this produces are exactly what `cdz-compile`'s decode accepts. This MIRRORS
/// rcdzc's `sidecar::encode`/`encode_request`/`encode_query` shape exactly (a byte-identity unit test pins
/// `build_sidecar_request(rs) == rcdzc::sidecar::encode(rs)`). `cdz`'s query drivers only ever send `Query`
/// requests (emit-tests takes a different path), but all `Request` arms are mirrored for completeness.
pub fn build_sidecar_request(requests: &[cadenza_compile_abi::Request]) -> Vec<u8> {
    use cadenza_syntax::ast::IntValue;
    use cadenza_syntax::{Builder, Leaf, Radix};

    let mut b = Builder::new();
    let forms: Vec<_> = requests
        .iter()
        .map(|req| match req {
            cadenza_compile_abi::Request::Emit(t) => {
                let head = b.name("emit");
                let target = b.name(target_cli(*t));
                b.list(vec![head, target])
            }
            cadenza_compile_abi::Request::EmitTests => {
                let h = b.name("emit-tests");
                b.list(vec![h])
            }
            cadenza_compile_abi::Request::EmitTestsPerFile => {
                let h = b.name("emit-tests-per-file");
                b.list(vec![h])
            }
            cadenza_compile_abi::Request::EmitTestsComposed => {
                let h = b.name("emit-tests-composed");
                b.list(vec![h])
            }
            cadenza_compile_abi::Request::EmitTestsConsumerOnly => {
                let h = b.name("emit-tests-consumer-only");
                b.list(vec![h])
            }
            cadenza_compile_abi::Request::EmitTestsShred => {
                let h = b.name("emit-tests-shred");
                b.list(vec![h])
            }
            cadenza_compile_abi::Request::EmitTestsShredStandalone => {
                let h = b.name("emit-tests-shred-standalone");
                b.list(vec![h])
            }
            cadenza_compile_abi::Request::EmitTestsShredTwoStage => {
                let h = b.name("emit-tests-shred-two-stage");
                b.list(vec![h])
            }
            cadenza_compile_abi::Request::Query(q) => {
                use cadenza_compile_abi::sidecar::Query;
                let (selector, arg): (&str, Option<_>) = match q {
                    Query::TypeOf { name: n } => {
                        ("type-of", Some(b.atom_leaf(Leaf::Str(n.clone().into()))))
                    }
                    Query::UsesOf { name: n } => {
                        ("uses-of", Some(b.atom_leaf(Leaf::Str(n.clone().into()))))
                    }
                    Query::DocOf { name: n } => {
                        ("doc-of", Some(b.atom_leaf(Leaf::Str(n.clone().into()))))
                    }
                    Query::Instantiations { name: n } => (
                        "instantiations",
                        Some(b.atom_leaf(Leaf::Str(n.clone().into()))),
                    ),
                    Query::TypeAt { node } => ("type-at", Some(int_leaf(&mut b, *node))),
                    Query::ResolveOf { node } => ("resolve-of", Some(int_leaf(&mut b, *node))),
                    Query::ScopeAt { node } => ("scope-at", Some(int_leaf(&mut b, *node))),
                    Query::DocAt { node } => ("doc-at", Some(int_leaf(&mut b, *node))),
                    Query::Diagnostics => ("diagnostics", None),
                    Query::Highlight => ("highlight", None),
                    Query::Exports => ("exports", None),
                    Query::ExportedTypes => ("exported-types", None),
                    Query::Symbols => ("symbols", None),
                    Query::ParamManifest => ("param-manifest", None),
                    Query::FuncLayout => ("func-layout", None),
                    Query::ClosureHash => ("closure-hash", None),
                    // The `@test` enumeration for `cdz test --list` (v-inference's Query::TestList) — nullary,
                    // selector "test-list", matching `sidecar::encode_query`. Its result is a cadenza-ast VALUE
                    // (KIND_TEST_LIST) `cdz --list` forwards verbatim.
                    Query::TestList => ("test-list", None),
                };
                let head = b.name("query");
                let sel = b.name(selector);
                let mut children = vec![head, sel];
                children.extend(arg);
                b.list(children)
            }
        })
        .collect();
    let root = b.list(forms);
    return cadenza_syntax::codec::encode(&b.finish(root));

    // A `Leaf::Int` node id (a `u32` `StructId`) — `cadenza_syntax`'s Int leaf carries an `IntValue`
    // wire type (the same type rcdzc's copy names `IntValue`), radix `Dec`, matching rcdzc's `atom_int`.
    fn int_leaf(b: &mut Builder, node: u32) -> cadenza_syntax::StructId {
        b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(i64::from(node)),
            radix: Radix::Dec,
        })
    }
}

/// The result-artifact KIND a single sidecar `Query` request materializes its answer under — the kind a
/// delegated reader tags the captured bytes with (mirrors `rcdzc::sidecar::run_query`'s per-query kind).
/// `None` for a non-`Query` request (never sent through `run_sidecar`), so the caller falls back in-process.
fn query_result_kind(request: &cadenza_compile_abi::Request) -> Option<&'static str> {
    use cadenza_compile_abi::sidecar::{self, Query};
    let cadenza_compile_abi::Request::Query(q) = request else {
        return None;
    };
    Some(match q {
        Query::TypeOf { .. } => sidecar::KIND_TYPE_INFO,
        Query::UsesOf { .. } => sidecar::KIND_USES,
        Query::TypeAt { .. } => sidecar::KIND_TYPE_AT,
        Query::ResolveOf { .. } => sidecar::KIND_RESOLVE,
        Query::Diagnostics => sidecar::KIND_DIAGNOSTICS,
        Query::ScopeAt { .. } => sidecar::KIND_SCOPE,
        Query::Highlight => sidecar::KIND_HIGHLIGHT,
        Query::Exports => sidecar::KIND_EXPORTS,
        Query::ExportedTypes => sidecar::KIND_EXPORT_TYPES,
        Query::DocOf { .. } | Query::DocAt { .. } => sidecar::KIND_DOC,
        Query::Instantiations { .. } => sidecar::KIND_INSTANTIATIONS,
        Query::Symbols => sidecar::KIND_SYMBOLS,
        Query::ParamManifest => sidecar::KIND_PARAM_MANIFEST,
        Query::FuncLayout => sidecar::KIND_FUNC_LAYOUT,
        Query::ClosureHash => sidecar::KIND_CLOSURE_HASH,
        Query::TestList => sidecar::KIND_TEST_LIST,
    })
}

/// Delegate a SINGLE sidecar query over `arenas`: build the `ast` + binary-AST `sidecar` request and run
/// it via [`run_query_over_inputs`]. Returns `None` when it can't delegate (a non-query, or a batch of >1
/// — batch needs positional result-file reading, a later slice), so the caller runs in-process.
pub fn run_sidecar_delegated(
    arenas: &cadenza_syntax::Arenas,
    requests: &[cadenza_compile_abi::Request],
    prog: &str,
) -> Option<cadenza_compile_abi::CompileOutput> {
    let [request] = requests else {
        return None;
    };
    let kind = query_result_kind(request)?;
    let inputs = vec![
        Artifact::new(
            cadenza_compile_abi::Artifact::KIND_AST,
            "main",
            cadenza_syntax::codec::encode(arenas),
        ),
        Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            build_sidecar_request(requests),
        ),
    ];
    Some(run_query_over_inputs(&inputs, kind, prog))
}

/// Delegate a SINGLE-RESULT sidecar query over ALREADY-PREPARED input artifacts (`ast`[s] + a
/// `KIND_SIDECAR` request + any `entry`/`spans`) — the general delegated-query core, for the multi-file
/// (package) query paths as well as the single-file [`run_sidecar_delegated`]. Materializes the inputs to
/// temp files, spawns `cdz-compile … -o -` (query-only, no `--target`), and CAPTURES stdout — the `-o -`
/// path writes the ONE query result artifact's bytes to stdout — tagging them with `result_kind` (the kind
/// the caller reads via `out.artifact(kind)`). stderr is inherited, so a failed entry's located
/// diagnostics surface and an EMPTY output makes the caller fail, matching the in-process shape. The
/// caller must know the request produces exactly ONE result artifact of `result_kind` (true for every
/// single `Query`); an emit / multi-result request does NOT come through here.
pub fn run_query_over_inputs(
    inputs: &[Artifact],
    result_kind: &str,
    prog: &str,
) -> cadenza_compile_abi::CompileOutput {
    let dir = scratch_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("{prog}: cannot create temp dir {}: {e}", dir.display());
        return empty_output();
    }
    let _guard = crate::RemoveOnDrop::dir(dir.clone());

    let Some(specs) = materialize_specs(inputs, &dir, prog) else {
        return empty_output();
    };

    let program = locate();
    let mut cmd = Command::new(&program);
    cmd.args(&specs)
        .arg("-o")
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdin(Stdio::null());

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "{prog}: cdz-compile not found (looked at $CDZ_COMPILE_BIN, beside `cdz`, then \
                 $PATH) — a delegating (`--no-default-features`) `cdz` spawns the compiler instead of \
                 linking it; build it with `cargo build -p rcdzc-cli --bin cdz-compile` and re-run"
            );
            return empty_output();
        }
        Err(e) => {
            eprintln!(
                "{prog}: could not run cdz-compile ({}): {e}",
                program.display()
            );
            return empty_output();
        }
    };
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{prog}: could not read cdz-compile output: {e}");
            return empty_output();
        }
    };
    if !output.status.success() {
        // The query's entry failed to compile — cdz-compile printed located diagnostics on stderr. An
        // empty output makes the caller's `out.artifact(KIND)` miss + fail, same as in-process.
        return empty_output();
    }
    // The captured stdout IS the query result artifact's bytes; tag it with the known result kind so the
    // caller's `out.artifact(result_kind)` finds it.
    cadenza_compile_abi::CompileOutput {
        artifacts: vec![cadenza_compile_abi::Artifact::new(
            result_kind,
            "0",
            output.stdout,
        )],
        diagnostics: vec![],
        // A DELEGATED (subprocess) compile ran no in-process CSE partition — the metric is `rcdzc`'s own
        // test instrumentation, always 0 here (this field is always-present since CompileOutput moved to
        // the shared crate).
        cse_partition_core_eq_calls: 0,
        // Same as the CSE metric: `rcdzc`'s own test instrumentation, no in-process lowering ran here.
        value_range_uncached_calls: 0,
        // Same: no in-process effects home-analysis ran, so the param_apply_extra_handled counter is 0.
        param_apply_extra_handled_calls: 0,
        // Same: no in-process emit ran, so the is_cse_shareable counter is 0.
        is_cse_shareable_uncached_calls: 0,
    }
}

/// An empty `CompileOutput` (no artifacts, no diagnostics) — a delegated query run's failure shape, where
/// `cdz-compile` has already reported diagnostics on the inherited stderr.
fn empty_output() -> cadenza_compile_abi::CompileOutput {
    cadenza_compile_abi::CompileOutput {
        artifacts: vec![],
        diagnostics: vec![],
        // No compile ran (delegated failure shape); the CSE metric is always 0 here.
        cse_partition_core_eq_calls: 0,
        // Same as the CSE metric: no in-process lowering ran, so the value_range counter is 0.
        value_range_uncached_calls: 0,
        // Same: no in-process effects home-analysis ran, so the param_apply_extra_handled counter is 0.
        param_apply_extra_handled_calls: 0,
        // Same: no in-process emit ran, so the is_cse_shareable counter is 0.
        is_cse_shareable_uncached_calls: 0,
    }
}

/// The decline shape for a BATCH sidecar query (`--where` over N bindings) in the thin `!standalone`
/// dispatcher. The delegate protocol is single-result today — a batch needs positional result naming, a
/// later slice — and this build does not link `rcdzc` to run the batch in-process. So emit a clear stderr
/// line and return the empty shape (the caller's `out.artifacts` is then empty → it reports no matches,
/// having warned). The nix seedCompiler never runs an interactive batch `--where`; a batch query needs
/// `cdz --features standalone`.
pub(crate) fn sidecar_batch_unsupported(prog: &str) -> cadenza_compile_abi::CompileOutput {
    eprintln!(
        "{prog}: a batch sidecar query (`--where` over multiple bindings) is not supported by the thin \
         dispatcher — rebuild with `--features standalone` for in-process batch queries"
    );
    empty_output()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE correctness gate for the delegated sidecar-request encoder: the binary-AST tree `cdz` builds
    /// via `cadenza_syntax` MUST DECODE, via the compiler's own `rcdzc::sidecar::decode` (= what
    /// `cdz-compile` runs), back to exactly the requests we asked for — else a delegated query sends
    /// bytes the compiler misreads. This is a DECODE round-trip, NOT raw byte-equality: `cadenza_syntax`'s
    /// Builder and rcdzc's copy order the leaf POOL differently (names-first vs insertion-order), so the
    /// raw bytes differ, but both are valid encodings of the same tree — the codec is byte-compatible, so
    /// rcdzc's decode reconstructs the same tree from either. Cover every `Request`/`Query` variant + a
    /// batch. (This test only compiles under `!standalone`, where the `delegate` module + a linked `rcdzc`
    /// are both present; run it with `cargo test -p cdz --no-default-features`.)
    #[test]
    fn built_sidecar_request_decodes_back_to_the_requests() {
        use cadenza_compile_abi::Request;
        use cadenza_compile_abi::sidecar::Query;
        let reqs = vec![
            Request::Query(Query::TypeOf { name: "foo".into() }),
            Request::Query(Query::UsesOf { name: "bar".into() }),
            Request::Query(Query::DocOf { name: "baz".into() }),
            Request::Query(Query::Instantiations { name: "qux".into() }),
            Request::Query(Query::TypeAt { node: 42 }),
            Request::Query(Query::ResolveOf { node: 7 }),
            Request::Query(Query::ScopeAt { node: 300 }),
            Request::Query(Query::DocAt { node: 0 }),
            Request::Query(Query::Diagnostics),
            Request::Query(Query::Highlight),
            Request::Query(Query::Exports),
            Request::Query(Query::ExportedTypes),
            Request::Query(Query::Symbols),
            Request::Query(Query::ParamManifest),
            Request::Query(Query::FuncLayout),
            Request::Query(Query::ClosureHash),
            Request::Emit(Target::Wasm),
            Request::Emit(Target::WasmDebug),
            Request::Emit(Target::Dwarf),
            Request::Emit(Target::Rust),
            Request::Emit(Target::RustAsync),
            Request::EmitTests,
            Request::EmitTestsPerFile,
            Request::EmitTestsComposed,
            Request::EmitTestsConsumerOnly,
            Request::EmitTestsShred,
            Request::EmitTestsShredStandalone,
            Request::EmitTestsShredTwoStage,
            Request::Query(Query::TestList),
        ];
        // Each request individually (isolates a per-variant mismatch)…
        for r in &reqs {
            let bytes = build_sidecar_request(std::slice::from_ref(r));
            assert_eq!(
                cadenza_compile_abi::sidecar::decode(&bytes).as_deref(),
                Some(std::slice::from_ref(r)),
                "single-request round-trip mismatch for {r:?}",
            );
        }
        // …and the whole batch (a multi-form root list) in one decode.
        assert_eq!(
            cadenza_compile_abi::sidecar::decode(&build_sidecar_request(&reqs)),
            Some(reqs.clone()),
            "batch round-trip mismatch",
        );
    }

    /// The result-kind mapping covers every query and matches `rcdzc::sidecar`'s constants — a delegated
    /// reader tags captured bytes with this kind, so a drift would mis-key the caller's `out.artifact`.
    #[test]
    fn query_result_kinds_are_complete_and_correct() {
        use cadenza_compile_abi::Request::Query;
        use cadenza_compile_abi::sidecar::{self, Query as Q};
        let cases = [
            (Q::TypeOf { name: "n".into() }, sidecar::KIND_TYPE_INFO),
            (Q::UsesOf { name: "n".into() }, sidecar::KIND_USES),
            (Q::TypeAt { node: 1 }, sidecar::KIND_TYPE_AT),
            (Q::ResolveOf { node: 1 }, sidecar::KIND_RESOLVE),
            (Q::Diagnostics, sidecar::KIND_DIAGNOSTICS),
            (Q::ScopeAt { node: 1 }, sidecar::KIND_SCOPE),
            (Q::Highlight, sidecar::KIND_HIGHLIGHT),
            (Q::Exports, sidecar::KIND_EXPORTS),
            (Q::ExportedTypes, sidecar::KIND_EXPORT_TYPES),
            (Q::DocOf { name: "n".into() }, sidecar::KIND_DOC),
            (Q::DocAt { node: 1 }, sidecar::KIND_DOC),
            (
                Q::Instantiations { name: "n".into() },
                sidecar::KIND_INSTANTIATIONS,
            ),
            (Q::Symbols, sidecar::KIND_SYMBOLS),
            (Q::ParamManifest, sidecar::KIND_PARAM_MANIFEST),
            (Q::FuncLayout, sidecar::KIND_FUNC_LAYOUT),
            (Q::ClosureHash, sidecar::KIND_CLOSURE_HASH),
            (Q::TestList, sidecar::KIND_TEST_LIST),
        ];
        for (q, kind) in cases {
            assert_eq!(
                query_result_kind(&Query(q.clone())),
                Some(kind),
                "kind for {q:?}"
            );
        }
    }

    #[test]
    fn target_strings_match_the_cli_spelling() {
        // These MUST equal `rcdzc::cli::TargetArg`'s clap value names, else the delegated `--target`
        // flag fails to parse. Pin every variant so a new backend can't silently drift the mapping.
        assert_eq!(target_cli(Target::Wasm), "wasm");
        assert_eq!(target_cli(Target::WasmDebug), "wasm-debug");
        assert_eq!(target_cli(Target::Dwarf), "dwarf");
        assert_eq!(target_cli(Target::Rust), "rust");
        assert_eq!(target_cli(Target::RustAsync), "rust-async");
        assert_eq!(target_cli(Target::Cadenza), "cadenza");
    }

    #[test]
    fn opt_level_strings_match_the_cli_spelling() {
        assert_eq!(opt_cli(OptLevel::O0), "o0");
        assert_eq!(opt_cli(OptLevel::O1), "o1");
        assert_eq!(opt_cli(OptLevel::O2), "o2");
        assert_eq!(opt_cli(OptLevel::O3), "o3");
    }

    #[test]
    fn delegated_flags_parse_back_through_the_real_compiler_cli() {
        // The string tests above pin the SPELLING; this pins that `cdz-compile` (= `rcdzc::cli`)
        // actually ACCEPTS each spelling AND parses it back to the same variant we delegated from.
        // So a rename of a `TargetArg`/`OptLevelArg` value (or a new backend) that would silently make
        // the delegated `--target`/`--opt-level` unparseable fails HERE, not only at runtime spawn.
        use clap::Parser;
        for t in [
            Target::Wasm,
            Target::WasmDebug,
            Target::Dwarf,
            Target::Rust,
            Target::RustAsync,
            Target::Cadenza,
        ] {
            let parsed = crate::compile_args::CompileArgs::try_parse_from([
                "cdz-compile",
                "x.ast",
                "--target",
                target_cli(t),
            ])
            .unwrap_or_else(|e| panic!("--target {} must parse: {e}", target_cli(t)));
            assert_eq!(parsed.targets(), vec![t], "round-trip for {t:?}");
        }
        for o in [OptLevel::O0, OptLevel::O1, OptLevel::O2, OptLevel::O3] {
            let parsed = crate::compile_args::CompileArgs::try_parse_from([
                "cdz-compile",
                "x.ast",
                "--opt-level",
                opt_cli(o),
            ])
            .unwrap_or_else(|e| panic!("--opt-level {} must parse: {e}", opt_cli(o)));
            assert_eq!(parsed.opt_level(), o, "round-trip for {o:?}");
        }
    }

    #[test]
    fn cdz_compile_bin_env_override_wins() {
        // $CDZ_COMPILE_BIN takes precedence over the sibling/$PATH resolution — the nix-injection path.
        // SAFETY: single-threaded test; the var is set and read within this test only.
        unsafe { std::env::set_var("CDZ_COMPILE_BIN", "/opt/custom/cdz-compile") };
        assert_eq!(locate(), PathBuf::from("/opt/custom/cdz-compile"));
        unsafe { std::env::remove_var("CDZ_COMPILE_BIN") };
    }
}
