//! `cdz run-rust` — the RUST-backend value oracle for the fuzzer's rust-vs-wasm differential.
//!
//! Compiles a program to the rust backend (`cdz compile --target rust`), wraps the emitted module in a
//! driver that calls the export and renders the boundary value byte-identically to `cdz-run`, `rustc`-links
//! the pre-built `cdz-rt`/`cdz-num` rlibs, runs it, and maps the outcome to a one-line verdict (`value`/
//! `declined`/`trap`/`error`). Extracted from `main.rs` as its own module (main.rs was at the 512 KiB
//! file-size mandate limit); the only `main.rs`-private items it needs are `PROG` and `read_verdict_source`.
use crate::{PROG, RemoveOnDrop, read_verdict_source};
use std::process::ExitCode;

#[derive(clap::Args)]
pub(crate) struct RunRustArgs {
    /// The program SOURCE file (s-expr / ml surface). OMITTED → read the program from stdin. Mirrors
    /// `cdz run-ml`'s input contract so the fuzzer's differential harness is symmetric.
    file: Option<String>,
    /// The export to invoke (default: the sole exported `main`, or the sole export if unambiguous). A
    /// `--call NAME` selects a specific export when the program exports several.
    #[arg(long)]
    call: Option<String>,
    /// An argument to the export, repeatable, each a canonical value-form literal (`7`, `"abc"`,
    /// `(tuple 3 4)`) coerced to the export's parameter type — mirrors `cdz-run --arg` so the rust-vs-wasm
    /// differential feeds both backends identically. Empty ⇒ nullary. `allow_hyphen_values` so `--arg -4`
    /// is a value, not a flag.
    #[arg(long = "arg", value_name = "VALUE", allow_hyphen_values = true)]
    args: Vec<String>,
}

/// `cdz run-rust` — compile a program to the RUST backend, run it natively, print ONE verdict line.
///
/// The fuzzer's rust-vs-wasm differential ORACLE shells this to get the Rust-backend value and compare it
/// to the wasm value (`cdz run`) — so the render MUST match `cdz-run`'s byte-for-byte (it uses the shared
/// `cdz-rust-render` crate, the same one the corpus gate's `--target rust` path uses). Verdict grammar:
/// `value <sexpr>` (ran to that value — bare, exactly as `cdz-run` prints); `declined` (the front-end
/// REJECTED the program, OR the rust backend doesn't emit it yet — coverage-not-yet, NOT a mismatch the
/// fuzzer files); `trap <msg>` (the program TRAPPED at run time — a Cadenza trap lowered to a Rust panic,
/// compared by reason); `error <msg>` (the emitted `.rs` FAILED to `rustc` — a bad artifact, a MISCOMPILE
/// the fuzzer files). `declined` vs `error` are kept DISTINCT (the fuzzer's one requirement beyond run-ml).
/// Exit is 0 for any RUN outcome (a verdict is not a shell failure); a NON-ZERO exit is a HARNESS/USAGE
/// failure that produced no verdict — a file/stdin READ error, OR a usage mistake (a bad/ambiguous `--call`,
/// or a wrong `--arg` count). A harness ENVIRONMENT breakage that occurs mid-run (can't spawn the compiler/
/// rustc) surfaces as an `error <msg>` VERDICT + exit 0, so the oracle always gets a line (Copilot PR #547/#551).
///
/// A non-nullary export is invoked by passing its args via `--arg` (repeatable); each value-form literal is
/// marshalled to the Rust expression the emitted export expects (see `marshal_export_args`).
///
/// MECHANISM (mirrors the gate's `run_program_rust`, now that its render half is the shared crate): shell
/// `cdz compile - -o - --target rust` (self, via `current_exe`) to emit the `.rs`; wrap it in `mod prog {…}`
/// and add a driver `fn main` that calls the export and prints `cdz_rust_render::cdz_render_expr(...)`;
/// `rustc -O` it, linking the pre-built `cdz-rt`/`cdz-num` rlibs that sit BESIDE the `cdz` binary in
/// `target/<profile>/` (the same dir `current_exe` lives in); run the binary and map its outcome to a verdict.
pub(crate) fn run_run_rust(args: &RunRustArgs) -> ExitCode {
    // 1. Read the program source (file or stdin, incl. an explicit `-`). A read failure is the reserved
    //    harness-error path.
    let source = match read_verdict_source(args.file.as_deref(), "run-rust") {
        Ok(s) => s,
        Err(()) => return ExitCode::FAILURE,
    };

    // 2. Emit the Rust module: shell `cdz compile - -o - --target rust` to SELF (install-location-independent
    //    via `current_exe`). A compile FAILURE means the front-end rejected the program or the rust backend
    //    declines it → `declined` (coverage-not-yet), NOT an `error` (an error is a bad ARTIFACT — see step 5).
    //    An ENVIRONMENT failure (can't find/spawn self) is surfaced as an `error <msg>` VERDICT on stdout + exit
    //    0, NOT a non-zero shell exit: the fuzzer's oracle always expects a verdict line (the sole non-zero exit
    //    is a source READ failure above), so a harness breakage must not look like a crash (Copilot PR #547).
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            println!("error current_exe: {e}");
            return ExitCode::SUCCESS;
        }
    };
    let module = match emit_rust_module(&exe, &source) {
        EmitOutcome::Module(m) => m,
        EmitOutcome::Declined => {
            println!("declined");
            return ExitCode::SUCCESS;
        }
        EmitOutcome::Harness(msg) => {
            // Couldn't even run the compiler (spawn/temp-write failure) — a harness breakage, surfaced as an
            // `error` verdict + exit 0 so the oracle gets a line rather than a silent non-zero crash.
            println!("error {msg}");
            return ExitCode::SUCCESS;
        }
    };

    // 3. Determine the export to invoke + its Cadenza result type (read off the `// cdz-return[<ident>]:`
    //    note the backend emits). With no `--call`, use the SOLE exported `pub fn`; if the module has
    //    SEVERAL (multiple exports), do NOT guess — require `--call` (Copilot PR #547: splitting on the
    //    first `pub fn` picked an arbitrary export and could run the wrong one).
    let export = match &args.call {
        Some(name) => cdz_rust_render::rust_ident(name),
        None => {
            let names = emitted_pub_fn_names(&module);
            match names.as_slice() {
                [one] => one.clone(),
                [] => {
                    // No exported fn — the backend produced nothing runnable → declined.
                    println!("declined");
                    return ExitCode::SUCCESS;
                }
                many => {
                    eprintln!(
                        "{PROG} run-rust: the program exports {} functions ({}); pass `--call NAME` to \
                         pick one",
                        many.len(),
                        many.join(", ")
                    );
                    return ExitCode::FAILURE;
                }
            }
        }
    };
    // 3b. VALIDATE the export + `--arg` count against the emitted signature BEFORE building the driver, so a
    //     USAGE problem (bad `--call`, or a wrong `--arg` count) is a clean harness error — NOT the `error`
    //     verdict, which is reserved for a rust MISCOMPILE (an emitted `.rs` that fails rustc).
    let params = match emitted_params(&module, &export) {
        Some(p) => p,
        None => {
            eprintln!(
                "{PROG} run-rust: no exported `{export}` in the compiled program{}",
                match &args.call {
                    Some(_) => " (check the `--call` name against the program's `(export …)`)",
                    None => "",
                }
            );
            return ExitCode::FAILURE;
        }
    };
    // The backend's uniform-env param (`__cdz_env`, on an effectful/async export) is driver plumbing, not a
    // source argument — filter it (mirrors cdz-rust-run's `is_env_param`) so the `--arg` count is checked
    // against the source signature.
    let source_params: Vec<&str> = params
        .iter()
        .copied()
        .filter(|p| !p.trim_start().starts_with("__cdz_env"))
        .collect();
    if source_params.len() != args.args.len() {
        eprintln!(
            "{PROG} run-rust: export `{export}` takes {} argument(s); {} `--arg` value(s) given \
             (pass one `--arg <value>` per source parameter)",
            source_params.len(),
            args.args.len()
        );
        return ExitCode::FAILURE;
    }
    // The export CALL the driver splices in — `prog::<export>(<marshaled args>)` (empty args ⇒
    // `prog::<export>()`, byte-identical to the pre-`--arg` driver).
    let marshaled = marshal_export_args(&source_params, &args.args);
    let call = format!("prog::{export}({})", marshaled.join(", "));
    let ret_ty = cdz_rust_render::cdz_return_type(&module, &export);

    // 4. Build a driver: wrap the emitted module in `mod prog {…}` (so its `pub fn main` becomes
    //    `prog::main` and does NOT collide with the driver's own `fn main`), then print the boundary value
    //    rendered by the SHARED crate — byte-identical to what `cdz-run` prints.
    //
    // A DIVERGING export (`cdz-return[export]: !` — a provable-trap program lowers to `pub fn <export>() ->
    // ! { panic!(…) }`) is a special case: its result has type `!`, so there is NOTHING to bind or render —
    // the call itself panics. Emit a driver that just CALLS it (no `let __r`, no `println!`): `prog::main()`
    // diverges → panics → `compile_and_run_rust_driver` maps the panic to the `trap` verdict, matching
    // wasm's clean `trap unreachable`. Without this, run-rust reported `error` for EVERY diverging program:
    // first the post-call render was unreachable (rustc `-D warnings` → hard error), and even silencing that
    // (`#[allow(unreachable_code)]`) then failed because `!`/`()` doesn't `Display` — a `!`-typed result
    // can't be rendered at all. So diverging programs are handled by NOT rendering (breaker + corpus-bugfix,
    // whose fuzzer rust-vs-wasm differential this false-`error` would otherwise poison for every trap case).
    // FLAG-GATED value-doc path (`CDZ_VALUE_DOC`, default-OFF): when the module carries a
    // `// cdz-value-doc: <export>` marker (rcdzc emitted a self-contained `pub fn __cdz_doc_<export>() ->
    // String` for this nullary export — see `backend/rust/mod.rs`), the driver just PRINTS that fn's result
    // (the `CDZDOC:<hex>` marker string, decoded by `cdz_rust_run::value_doc::interpret_run_stdout` on the
    // read side). This replaces the type-note-driven `cdz_render_expr` string render with the rcdzc Ty-direct
    // walk. The marker is absent unless the flag was set for the `cdz compile` child (env-inherited), so with
    // the flag off this is byte-identical to the render path below.
    // Gate on the MARKER's PRESENCE — rcdzc emits `// cdz-value-doc: <export>` iff the compile child had
    // CDZ_VALUE_DOC set (which `emit_rust_module` does by default now), so the marker IS the authoritative
    // signal. (No env re-check here: the emit decision already happened in the child; the marker records it.)
    // The `__cdz_doc_<export>` helper is emitted only for a NULLARY export, so gate the doc path on no args
    // (an arg export never has the marker; the guard is defensive).
    let value_doc = args.args.is_empty()
        && module
            .lines()
            .any(|l| l.trim() == format!("// cdz-value-doc: {export}"));
    let driver = if value_doc {
        format!(
            "#[allow(warnings)]\nmod prog {{\n{module}\n}}\nfn main() {{\n    println!(\"{{}}\", prog::__cdz_doc_{export}());\n}}\n"
        )
    } else if ret_ty.as_deref() == Some("!") {
        format!("#[allow(warnings)]\nmod prog {{\n{module}\n}}\nfn main() {{\n    {call};\n}}\n")
    } else {
        let render = match &ret_ty {
            Some(ty) => {
                let sums = cdz_rust_render::cdz_sum_descriptors(&module);
                let newtypes = cdz_rust_render::cdz_newtype_descriptors(&module);
                let sum_params = cdz_rust_render::cdz_sum_params(&module);
                let qualified_heads = cdz_rust_render::cdz_sum_qualified_heads(&module);
                let unit_form = cdz_rust_render::cdz_unit_form(&module, &export);
                let scale = cdz_rust_render::cdz_scale(&module, &export);
                let qty_at = cdz_rust_render::cdz_qty_at(&module, &export);
                cdz_rust_render::cdz_render_expr(
                    ty,
                    &sums,
                    &newtypes,
                    &sum_params,
                    unit_form.as_deref(),
                    scale,
                    &qty_at,
                    &qualified_heads,
                )
            }
            // No `cdz-return` note (an older/void export) — fall back to Display of the result.
            None => "format!(\"{}\", __r)".to_string(),
        };
        format!(
            "#[allow(warnings)]\nmod prog {{\n{module}\n}}\nfn main() {{\n    let __r = {call};\n    println!(\"{{}}\", {render});\n}}\n"
        )
    };

    // 5. rustc the driver, linking the pre-built `cdz-rt`/`cdz-num` rlibs beside the `cdz` binary (same
    //    dir `current_exe` is in — `cargo build` puts `libcdz_{rt,num}.rlib` in `target/<profile>/`). A
    //    UNIQUE per-process temp dir (pid-stamped) so concurrent oracle invocations never race prog.rs/prog.
    // A rustc/driver HARNESS failure (couldn't write prog.rs, spawn rustc, or exec the binary) is surfaced
    // as an `error <msg>` VERDICT + exit 0 — the oracle always expects a verdict line; a non-zero shell exit
    // would look like a crash indistinguishable from a real harness break (Copilot PR #547). `compile_and_run_
    // rust_driver` already returns the value/error/trap VERDICT as `Ok`; its `Err` is only a harness break.
    match compile_and_run_rust_driver(&exe, &driver) {
        Ok(verdict) => println!("{verdict}"),
        Err(msg) => println!("error {msg}"),
    }
    ExitCode::SUCCESS
}

/// The names of every top-level `pub fn` the emitted module declares, in source order — used to pick the
/// default export (exactly one → use it; several → require `--call`). A `pub fn <name>(` / `pub fn
/// <name><generics>` at line start (the backend emits each export as a top-level `pub fn`); a nested/inner
/// `pub fn` would not be at column 0, so keying on the line start avoids counting one.
pub(crate) fn emitted_pub_fn_names(module: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in module.lines() {
        if let Some(rest) = line.strip_prefix("pub fn ") {
            let name = rest.split(['(', '<']).next().unwrap_or("").trim();
            // Skip the value-doc HELPER fns (`__cdz_doc_<export>`, emitted under CDZ_VALUE_DOC): they are
            // internal render helpers, not program exports — counting them would break the sole-export
            // pick (a nullary `main` + its `__cdz_doc_main` would spuriously read as "2 exports").
            if !name.is_empty() && !name.starts_with("__cdz_doc_") {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// The emitted `pub fn <export>(…)` parameter list, split into its top-level `<name>: <type>` slices —
/// `Some(vec![])` for a nullary export, `None` if no such `pub fn` (a bad `--call`). Finds `pub fn <export>`
/// (also `<generics>` for the async form), takes the `(…)` list to the matching close paren, and splits at
/// top-level commas (a comma inside a nested `(…)`/`<…>`/`[…]` is part of one param's type). Enough to count
/// source params, read their types for the arg marshal, and detect an absent export. (A closure-typed param
/// `Rc<dyn Fn(x) -> r>` has an inner `->` that unbalances this simple walk; such consumer exports are not
/// arg-driven here — cf. cdz-rust-run's arrow-aware `parse_emitted_sig`.)
pub(crate) fn emitted_params<'a>(module: &'a str, export: &str) -> Option<Vec<&'a str>> {
    // Find `pub fn <export>` where the name is a whole token (followed by `(` or `<`, not more ident chars).
    let needle = format!("pub fn {export}");
    let mut search_from = 0;
    let after = loop {
        let rel = module.get(search_from..)?.find(&needle)?;
        let idx = search_from + rel + needle.len();
        match module[idx..].chars().next() {
            Some('(') | Some('<') => break &module[idx..],
            // A longer name that merely starts with `export` (e.g. `main2`) — keep searching.
            _ => search_from = idx,
        }
    };
    // Skip any generic list `<…>` (the async form `pub fn f<E: CdzEnv>(…)`) to reach the param `(`.
    let params_open = after.find('(')?;
    let rest = &after[params_open + 1..];
    // Take up to the matching close paren, tracking nesting so a param type like `(i64, i64)` or a
    // generic `Vec<(A, B)>` doesn't end the list early.
    let mut depth = 0i32;
    let mut end = None;
    for (i, c) in rest.char_indices() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' if depth == 0 => {
                end = Some(i);
                break;
            }
            ')' | '>' | ']' => depth -= 1,
            _ => {}
        }
    }
    let params = rest[..end?].trim();
    if params.is_empty() {
        return Some(Vec::new());
    }
    // Split at TOP-LEVEL commas (a comma inside a nested `(…)`/`<…>`/`[…]` is part of one param's type).
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in params.char_indices() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(params[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(params[start..].trim());
    Some(out)
}

/// Marshal each `--arg` value-form literal to the Rust expression the export expects, paired positionally
/// with the export's SOURCE params (env param already filtered by the caller). A scalar / String / compound
/// goes through the shared `cdz_rust_render::rust_call_arg` (the same marshal the corpus rust gate uses), so
/// both backends see byte-identical args. TYPE-AWARE for a BigInt param: a bare decimal (`5`) has no
/// self-identifying suffix (unlike `"…"` or `5N`), so a `cdz_num::Big` param type routes it to `big_arg_expr`.
/// Mirrors cdz-rust-run's `marshal_call_args`. (Closure-param consumer exports are not arg-driven here.)
pub(crate) fn marshal_export_args(source_params: &[&str], args: &[String]) -> Vec<String> {
    args.iter()
        .zip(source_params)
        .map(|(a, p)| {
            let ty = p.split_once(':').map(|(_, t)| t.trim()).unwrap_or("");
            if ty == "cdz_num::Big"
                && let Ok(n) = a.trim().parse::<i128>()
            {
                cdz_rust_render::big_arg_expr(n)
            } else {
                cdz_rust_render::rust_call_arg(a)
            }
        })
        .collect()
}

/// The outcome of emitting a program to the rust backend: the `.rs` module text, a DECLINE (front-end
/// reject / backend not-yet), or a HARNESS failure (couldn't spawn the compiler).
enum EmitOutcome {
    Module(String),
    Declined,
    Harness(String),
}

/// Shell `<exe> compile <src>.sexp -o - --target rust` → the emitted `.rs` on stdout. The source is
/// written to a per-process temp `.sexp` FILE (not piped to stdin `-`, which `cdz compile` reads as a
/// pre-built binary AST — a `.sexp` file's extension selects in-process SOURCE parsing). A non-zero
/// compile exit is a DECLINE (front-end reject or rust-backend not-yet); a spawn/IO failure is a harness error.
fn emit_rust_module(exe: &std::path::Path, source: &str) -> EmitOutcome {
    use std::process::Command;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cdz-run-rust-emit-{}-{seq}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return EmitOutcome::Harness(format!("create temp dir: {e}"));
    }
    let _guard = RemoveOnDrop::dir(dir.clone());
    let src = dir.join("prog.sexp");
    if let Err(e) = std::fs::write(&src, source) {
        return EmitOutcome::Harness(format!("write source: {e}"));
    }
    let mut cmd = Command::new(exe);
    cmd.arg("compile")
        .arg(&src)
        .args(["-o", "-", "--target", "rust"]);
    // VALUE-DOC flip (op-seq-210): the run-rust ORACLE renders the boundary value via the rcdzc-emitted
    // Ty-direct `__cdz_doc` (the `(: value type)` codec doc — byte-identical to `cdz run`'s wasm render) rather
    // than cdz-rust-render's type-note string walk. That is enabled by setting `CDZ_VALUE_DOC` for THIS child
    // `cdz compile` (so rcdzc emits the `__cdz_doc` fn + marker); the driver then calls it (gated on the marker
    // below). ON by DEFAULT here (the gate/oracle harness) — a real `cdz compile --target rust` never sets it,
    // so a user's module stays free of the `cadenza_ast` dep. KILL-SWITCH: set `CDZ_VALUE_DOC=0` to fall back
    // to cdz_render_at (A/B / rollback without a revert).
    if std::env::var("CDZ_VALUE_DOC").as_deref() != Ok("0") {
        cmd.env("CDZ_VALUE_DOC", "1");
    }
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) => return EmitOutcome::Harness(format!("spawn compile: {e}")),
    };
    if !out.status.success() {
        return EmitOutcome::Declined;
    }
    EmitOutcome::Module(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Write the `driver` to a per-process temp dir, `rustc -O` it (linking the `cdz-rt`/`cdz-num` rlibs that
/// sit beside `exe`), run the binary, and map the outcome to a verdict STRING. A rustc failure is
/// `error <first-stderr-line>` (a bad artifact = a miscompile); a non-zero RUN is `trap <…>` (a panic =
/// a Cadenza trap); success is `value <stdout>` (the rendered boundary value). The temp dir is removed on
/// every return via an RAII guard.
/// Resolve the directory holding cargo's HASHED dependency rlibs, given the dir the `cdz` bin sits in.
/// Normally `<lib_dir>/deps`. But a `cargo test`-built bin ALREADY lives in `.../deps/`, so the deps dir is
/// `lib_dir` ITSELF — appending `deps` there gives `.../deps/deps` (nonexistent), which is the PR#772 bug
/// (the hashed `libcdz_num-<hash>.rlib` search dir goes missing → E0433). Detect the already-in-`deps` case
/// by the dir's own name so we never double-append.
pub(crate) fn resolve_deps_dir(lib_dir: &std::path::Path) -> std::path::PathBuf {
    if lib_dir.file_name().is_some_and(|n| n == "deps") {
        lib_dir.to_path_buf()
    } else {
        lib_dir.join("deps")
    }
}

/// The `run-rust` backend-rlib search ROOTS, in priority order: the `CDZ_RUST_RLIB_DIR` override (when
/// set) FIRST, then the exe-relative `lib_dir`. The nix `cdz` package sets the override because its
/// `bin/` ships NO rlibs beside the exe (so the exe-relative search alone finds none → `E0433 cannot
/// find crate cdz_num` on every `run-rust`); a plain `cargo build`/`cargo xtask build` leaves it unset
/// and keeps the exe-relative behavior (the rlibs sit beside the `cdz` bin). Pure (the override is
/// passed in, not read from the env) so the precedence is unit-testable.
pub(crate) fn rust_rlib_search_roots(
    lib_dir: &std::path::Path,
    override_dir: Option<std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(d) = override_dir {
        roots.push(d);
    }
    roots.push(lib_dir.to_path_buf());
    roots
}

/// Locate a backend dependency rlib (`cdz_rt`/`cdz_num`) for the `run-rust` link. Prefer the PLAIN
/// top-level `lib<crate>.rlib` in `lib_dir` (a `cargo build`-built workspace has it beside the `cdz` bin);
/// else fall back to the NEWEST hashed `lib<crate>-<hash>.rlib` in `deps_dir` (what `cargo test` produces —
/// the plain name is often absent there, only the hashed one). `deps_dir` is the caller's resolved
/// hashed-rlib dir (`lib_dir/deps`, OR `lib_dir` itself when the bin already lives in `deps/`). Newest-by-
/// mtime so a rebuild's fresh artifact wins over a stale one. `None` if neither exists (the caller then
/// omits the `--extern`, as before — a program that references the crate then fails rustc loudly, which is
/// strictly better than a silent wrong link).
fn find_backend_rlib(
    lib_dir: &std::path::Path,
    deps_dir: &std::path::Path,
    crate_name: &str,
) -> Option<std::path::PathBuf> {
    let plain = lib_dir.join(format!("lib{crate_name}.rlib"));
    if plain.exists() {
        return Some(plain);
    }
    // deps/lib<crate>-<hash>.rlib — pick the most recently modified match.
    let prefix = format!("lib{crate_name}-");
    std::fs::read_dir(deps_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with(&prefix) && n.ends_with(".rlib")
        })
        .max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .map(|e| e.path())
}

fn compile_and_run_rust_driver(exe: &std::path::Path, driver: &str) -> Result<String, String> {
    use std::process::Command;
    let lib_dir = exe
        .parent()
        .ok_or_else(|| "cannot locate the cdz binary's directory".to_string())?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cdz-run-rust-{}-{seq}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let _guard = RemoveOnDrop::dir(dir.clone());
    let src = dir.join("prog.rs");
    let bin = dir.join("prog");
    std::fs::write(&src, driver).map_err(|e| format!("write driver: {e}"))?;

    // rustc, linking the pre-built rlibs beside the cdz binary (built by `cargo build`/`cargo xtask
    // build` into `target/<profile>/`). `--extern` only makes a crate available (not force-linked), so
    // passing both is harmless when the program references neither.
    let mut cmd = Command::new("rustc");
    cmd.args(["-O", "--edition", "2021"])
        .arg(&src)
        .arg("-o")
        .arg(&bin);
    // rlib search ROOTS, in priority order. `CDZ_RUST_RLIB_DIR` (set by the nix `cdz` package — whose
    // `bin/` ships NO rlibs beside the exe, so the exe-relative search alone finds none and every
    // `run-rust` fails `E0433 cannot find crate cdz_num`) is searched FIRST when present; the exe-relative
    // `lib_dir` (a `cargo build` / `cargo xtask build` workspace has the rlibs beside the `cdz` bin) is the
    // fallback, so a plain cargo build is unaffected. Each root also contributes its resolved `deps/`
    // (cargo's hashed-rlib dir); deduped, `-L`'d for every existing dir.
    let roots = rust_rlib_search_roots(
        lib_dir,
        std::env::var_os("CDZ_RUST_RLIB_DIR").map(std::path::PathBuf::from),
    );
    // The `-L dependency=` search path: each root + its `deps/`. `resolve_deps_dir` handles a root that is
    // ITSELF `.../deps` (a `cargo test`-located bin): it stays `deps`, never `deps/deps` (PR#772 review).
    let mut search_dirs: Vec<std::path::PathBuf> = Vec::new();
    for root in &roots {
        for d in [root.clone(), resolve_deps_dir(root)] {
            if d.is_dir() && !search_dirs.contains(&d) {
                search_dirs.push(d);
            }
        }
    }
    for d in &search_dirs {
        cmd.arg("-L").arg(format!("dependency={}", d.display()));
    }
    // Locate each backend rlib ROBUSTLY across the roots (override first, then exe-relative): the plain
    // top-level `lib<crate>.rlib` (a `cargo build` workspace) OR the newest `deps/lib<crate>-<hash>.rlib`
    // (what `cargo test` produces — the plain name may be ABSENT there). EVERY emitted program references
    // `cdz_num` (the always-emitted `Ast` enum, since Ast.Int carries a `cdz_num::Big`), so a missing
    // rlib means a cryptic `E0433 cannot find crate cdz_num` — find-either-anywhere fixes it wherever the
    // artifact lives. `--extern` only MAKES a crate available (not force-linked), so naming an unused one
    // stays harmless.
    //
    // The full set MIRRORS the corpus rust-exec grader (`cdz-rust-run`'s `compile_and_run`): a native
    // VALUE-ENCODE program references `cadenza_ast` (the AST builder) + `num_bigint` (IntValue bridge), and a
    // runtime `String.concat`/`from-bytes` NFC-normalizes via `unicode_normalization` (the `Core::NfcNormalize`
    // emit, FINDING #23 rust parity). Those three are STAGED beside `cdz_rt`/`cdz_num` in the nix
    // `CDZ_RUST_RLIB_DIR` rlib set (`cadenza_ast` plain top-level; `num_bigint` + `unicode_normalization`
    // hashed in its `deps/`, pulled in via cadenza-ast's `std` feature), so `find_backend_rlib`'s
    // plain-then-hashed search resolves each. Without them a `cdz run-rust` differential run of such a program
    // failed `E0433 cannot find crate …` where the corpus grader (which links the full set) passed.
    for crate_name in [
        "cdz_rt",
        "cdz_num",
        "cadenza_ast",
        "num_bigint",
        "unicode_normalization",
    ] {
        if let Some(rlib) = roots
            .iter()
            .find_map(|root| find_backend_rlib(root, &resolve_deps_dir(root), crate_name))
        {
            cmd.arg("--extern")
                .arg(format!("{crate_name}={}", rlib.display()));
        }
    }
    let compile = cmd
        .output()
        .map_err(|e| format!("rustc failed to launch: {e}"))?;
    if !compile.status.success() {
        // The emitted `.rs` did not compile — a BAD ARTIFACT (a rust-backend miscompile the fuzzer files),
        // NOT a decline. Report the first stderr line (often the root error).
        let stderr = String::from_utf8_lossy(&compile.stderr);
        let first = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("rustc failed")
            .trim();
        return Ok(format!("error {first}"));
    }
    // Run the compiled driver. A non-zero exit (a panic) is a TRAP (a Cadenza trap lowered to a Rust
    // panic); the panic MESSAGE is the reason.
    let run = Command::new(&bin)
        .output()
        .map_err(|e| format!("run failed to launch: {e}"))?;
    if !run.status.success() {
        let stderr = String::from_utf8_lossy(&run.stderr);
        return Ok(format!("trap {}", panic_reason(&stderr)));
    }
    let value = String::from_utf8_lossy(&run.stdout).trim().to_string();
    // VALUE-DOC decode: a value-doc driver prints a `CDZDOC:<hex>` marker (the self-describing
    // `(: value type)` binary-AST codec doc rcdzc's `__cdz_doc_<export>` builds). Render it through the
    // CANONICAL printer (`render_binary`, Sexpr/Expr — the SAME path cdz-run + the corpus grader use), so the
    // run-rust verdict is the canonical value surface, not the raw hex. A NON-marker stdout (the ordinary
    // render path, or with the flag off) passes through unchanged. A corrupt marker → an `error` verdict
    // (a bad artifact), never a silent mis-render.
    if let Some(hex) = value.strip_prefix("CDZDOC:") {
        return Ok(match decode_value_doc(hex) {
            Ok(rendered) => format!("value {rendered}"),
            Err(e) => format!("error value-doc decode: {e}"),
        });
    }
    Ok(format!("value {value}"))
}

/// Decode a `CDZDOC:` marker payload (lowercase hex of the binary-AST `(: value type)` doc) to its canonical
/// s-expr surface via `render_binary` — the shared printer cdz-run/the corpus grader use, so the rendered
/// value is byte-identical across the wasm + rust gates. Errors on a malformed hex/AST (→ an `error` verdict).
fn decode_value_doc(hex: &str) -> Result<String, String> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) {
        return Err(format!("marker hex has odd length {}", hex.len()));
    }
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("non-hex digit in marker: {e}"))?;
    // SINGLE-LINE canonical value render (seq-283, #7773) — byte-identical to cdz-run's `render_val`; the
    // general `render_binary` PRETTY path hard-breaks a long `(: value type)` across lines, diverging from
    // cdz-run's one-line render (breaker finding on a long record result).
    cadenza_syntax::convert::render_binary_value_line(&bytes).map_err(|e| format!("{e}"))
}

/// Extract a DETERMINISTIC panic reason from a Rust panic's stderr — the trap message the differential
/// oracle compares. Rust prints `thread 'main' panicked at <FILE>:<LINE>:<COL>:` followed by the payload
/// message on the NEXT line. The `<FILE>:<LINE>:<COL>` is a per-run temp path (`/tmp/cdz-run-rust-…/prog.rs`),
/// so returning THAT line makes the reason vary run-to-run (Copilot PR #547). Return the payload MESSAGE
/// (the line after "panicked at …") instead — stable across runs and the actual reason. Falls back to the
/// first non-empty line if the format is unexpected.
pub(crate) fn panic_reason(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().collect();
    // Modern format: the line AFTER `… panicked at …:` is the payload message.
    if let Some(i) = lines.iter().position(|l| l.contains("panicked at")) {
        if let Some(msg) = lines.get(i + 1) {
            let m = msg.trim();
            if !m.is_empty() {
                return m.to_string();
            }
        }
        // Older format: `… panicked at '<payload>', <file>:<line>` — the quoted payload AFTER "panicked
        // at" (search from there, so the `'` in `thread 'main'` before it isn't mistaken for the open quote).
        if let Some(pa) = lines[i].find("panicked at") {
            let tail = &lines[i][pa..];
            if let Some(start) = tail.find('\'')
                && let Some(end) = tail.rfind('\'')
                && end > start
            {
                return tail[start + 1..end].to_string();
            }
        }
    }
    lines
        .iter()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("panic")
        .to_string()
}
