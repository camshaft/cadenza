//! The seed toolchain CLI — generation 0 of Cadenza.
//!
//! The seed is a **reference compiler** (`cdz-rustc`): it lowers a Cadenza program's
//! canonical AST to a real WebAssembly component and runs it on the host. There is no
//! reference interpreter; the behavioral oracle is the conformance corpus, and the
//! independence of the judgment comes from a second implementation of the compiler authored
//! in Cadenza that must agree (constitution XIV as amended 2026-07-04;
//! spec/learnings/2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md).
//!
//! Subcommands:
//!   behavior-gate [<corpus-dir>]   compile every realized corpus case with cdz-rustc, run
//!                                  the component, and confirm its observable behavior equals
//!                                  the recorded result. Passes/todos/skips; fails only on a
//!                                  contradiction of the record.
//!   emit <program.cdz>             compile one program file and dump the component bytes +
//!                                  validity (a debug probe).
//!   ignite [<program.cdz>]         compile a program to a content-addressed component, run
//!                                  it, and re-compile to confirm byte-identical reproduction.

// The host/corpus/probe live in the `cadenza-seed` library crate; the compiler core is in
// `cdz-compiler`. Alias both so the CLI's existing `ast::`/`codegen::`/`host::`/`corpus::`
// references resolve unchanged.
use cadenza_seed::{corpus, host};
use cdz_compiler::{ast, codegen};

use std::process::ExitCode;

fn main() -> ExitCode {
    init_trace();
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("behavior-gate");
    match cmd {
        "behavior-gate" => {
            let dir = args.get(2).map(|s| s.as_str()).unwrap_or("../../spec/semantics");
            run_behavior_gate(dir)
        }
        "emit" => {
            let path = args.get(2).map(|s| s.as_str()).unwrap_or("/tmp/probe.cdz");
            run_emit(path)
        }
        "ignite" => {
            let path = args.get(2).map(|s| s.as_str());
            run_ignition(path)
        }
        // Prove cdz-rustc-as-a-component: compile a program through BOTH the native compiler
        // and the compiler COMPONENT, and confirm the emitted component bytes are identical.
        // This is the two-implementations-of-one-compiler agreement at the tooling level, and
        // the systematic check that the wasm build of cdz-rustc is faithful.
        "component-check" => {
            let component = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or("crates/cdz-compiler-component/target/wasm32-unknown-unknown/release/cdz_compiler_component.wasm");
            let dir = args.get(3).map(|s| s.as_str()).unwrap_or("../../spec/semantics");
            run_component_check(component, dir)
        }
        // Emit a `(def (compile b) …)` program as a `compile : list<u8> -> list<u8>` component, then
        // (a) with `--emit-component <path>`, PERSIST that component to disk (so `component-check
        // <path> <corpus>` grades the Cadenza-authored compiler at the byte level — the real
        // self-hosting gate, SPEC-BACKLOG #28); and/or (b) with an input `.cdz`, DRIVE it over that
        // program's canonical AST bytes and print the outcome (the dev-desk build+run check, GAP 3l).
        "compile-run" => {
            // `--emit-component <path>` may appear anywhere after the program; the first remaining
            // positional (not consumed by the flag) is the optional input program.
            let emit_to = args
                .iter()
                .position(|a| a == "--emit-component")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.as_str());
            let positionals: Vec<&str> = args[2..]
                .iter()
                .map(|s| s.as_str())
                .filter(|s| *s != "--emit-component" && Some(*s) != emit_to)
                .collect();
            let prog = positionals.first().copied().unwrap_or("/tmp/compiler.cdz");
            let input = positionals.get(1).copied();
            run_compile_entry(prog, input, emit_to)
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            eprintln!(
                "usage: cadenza-seed [behavior-gate <dir> | emit <program> | ignite <program> \
                 | component-check <component.wasm> <corpus-dir>]"
            );
            ExitCode::from(2)
        }
    }
}

/// Initialize the compilation-decision trace subscriber (ask-50). Only compiled in under
/// `--features trace`; a default build makes this a no-op, so a plain `cargo build` / gate run is
/// byte-for-byte identical to today (no `tracing` in the graph). Even in a `trace` build, tracing is
/// OFF unless `CADENZA_TRACE` is set (env-filtered), and it writes to STDERR — never stdout, which
/// carries the extracted component bytes / `ran →`/`compile →` lines the harness parses (the
/// stray-output regressions ask-44/ask-47 make this non-negotiable). Filter examples:
/// `CADENZA_TRACE=debug` (all), `CADENZA_TRACE=cdz::decline=debug` (just declines).
#[cfg(feature = "trace")]
fn init_trace() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("CADENZA_TRACE").unwrap_or_else(|_| EnvFilter::new("off"));
    let _ = fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

#[cfg(not(feature = "trace"))]
fn init_trace() {}

fn run_behavior_gate(dir: &str) -> ExitCode {
    let results = match corpus::run_corpus(dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to read corpus at {dir}: {e}");
            return ExitCode::from(2);
        }
    };
    let (mut passed, mut todo, mut skipped, mut failed) = (0, 0, 0, 0);
    for r in &results {
        match &r.status {
            corpus::CaseStatus::Passed => {
                passed += 1;
                println!("  PASS    {}", r.description);
            }
            corpus::CaseStatus::Todo(reason) => {
                todo += 1;
                println!("  todo    {}  [{}]", r.description, reason);
            }
            corpus::CaseStatus::Skipped(cap) => {
                skipped += 1;
                println!("  skip    {}  (needs {cap})", r.description);
            }
            corpus::CaseStatus::Failed { expected, observed } => {
                failed += 1;
                println!("  FAIL    {}\n            expected: {expected}\n            observed: {observed}", r.description);
            }
        }
    }
    println!("\nbehavior gate: {passed} passed, {todo} todo, {skipped} skipped, {failed} failed");
    if failed == 0 {
        println!("BEHAVIOR-GATE: PASS ({passed} agree, {todo} still to compile)");
        ExitCode::SUCCESS
    } else {
        println!("BEHAVIOR-GATE: FAIL ({failed} contradict the recorded semantics)");
        ExitCode::FAILURE
    }
}

fn run_emit(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    // Read the whole source unit as a PROGRAM: the reader synthesizes the implicit-module `(do …)`
    // wrapper when the file has more than one top-level form (defs/types/exports), so a program is a
    // single node with no explicit `(module …)`.
    let node = match ast::read_program(&src) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("parse error: {e}");
            return ExitCode::from(2);
        }
    };
    // Compile through the selected compiler (old oracle by default; rcdzc under `CADENZA_COMPILER=v2`)
    // and report the FULL diagnostics list — a compilation may reject at several independent sites, so
    // print every diagnostic rather than stopping at the first (compiler-pipeline.md §Phases Recover
    // From Errors). A produced component runs; diagnostics print alongside regardless of success.
    let out = cadenza_seed::compiler::compile(&node);
    for d in &out.diagnostics {
        print_diagnostic(d);
    }
    match out.component() {
        Some(bytes) => {
            println!("{} bytes: {:?}", bytes.len(), bytes);
            let _ = std::fs::write("/tmp/cadenza-emit.wasm", bytes);
            match host::validate_component(bytes) {
                Ok(()) => {
                    println!("VALID component");
                    match host::run_component(bytes, &[]) {
                        Ok((outcome, state)) => {
                            println!("ran → {outcome:?}");
                            // Surface the leak oracle when the composed runtime carried the counter
                            // (the `debug-counters` build): `live=0` proves the program reclaimed
                            // every heap object; `live=N>0` is a leak. Absent on the default runtime.
                            if let Some(live) = state.live_after_run {
                                println!("live-objects → {live}");
                            }
                        }
                        Err(e) => println!("run error: {e}"),
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    println!("INVALID: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        // No component: a decline/reject. The diagnostics above already named every failure site.
        None => {
            if out.diagnostics.is_empty() {
                println!("declined: compiler produced no component");
            }
            ExitCode::SUCCESS
        }
    }
}

/// Print one diagnostic in the CLI's existing form: a coded reject reads `rejected CDZ####: …`, an
/// uncoded decline reads `declined: …` (matching the old `Decline` Display), so downstream harnesses
/// that scan for those prefixes are unaffected.
fn print_diagnostic(d: &rcdzc::Diagnostic) {
    match &d.code {
        Some(code) => println!("rejected {code}: {}", d.message),
        None => println!("declined: {}", d.message),
    }
}

/// Emit a `(def (compile b) …)` program as a `compile` component and run it over `input`'s canonical
/// AST bytes (or the empty input if none). The GAP-3l dev check: a `bytes → bytes` compiler entry
/// builds AND drives end-to-end (the harness path, composing the value-heap runtime).
fn run_compile_entry(prog_path: &str, input_path: Option<&str>, emit_to: Option<&str>) -> ExitCode {
    let src = match std::fs::read_to_string(prog_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {prog_path}: {e}");
            return ExitCode::from(2);
        }
    };
    let node = match ast::read(&src) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("parse error: {e}");
            return ExitCode::from(2);
        }
    };
    let component = match codegen::compile_program(&node) {
        Ok(b) => b,
        Err(d) => {
            println!("declined: {}", d.0);
            return ExitCode::SUCCESS;
        }
    };
    if let Err(e) = host::validate_component(&component) {
        println!("INVALID compile component: {e}");
        return ExitCode::FAILURE;
    }
    println!("VALID compile component ({} bytes)", component.len());

    // `--emit-component <path>`: PERSIST the built `cadenza:compiler/compile` component so
    // `component-check <path> spec/semantics` can grade the Cadenza-authored compiler at the byte
    // level (SPEC-BACKLOG #28 — the real self-hosting gate). This is the whole point of the flag; a
    // write failure is fatal.
    if let Some(path) = emit_to {
        if let Err(e) = std::fs::write(path, &component) {
            eprintln!("cannot write component to {path}: {e}");
            return ExitCode::FAILURE;
        }
        println!("wrote compile component → {path}");
        // With no input program to drive, persisting IS the job — done.
        if input_path.is_none() {
            return ExitCode::SUCCESS;
        }
    }

    // The input the compiler consumes: the canonical AST bytes of the second program (or empty).
    let input_bytes: Vec<u8> = match input_path {
        Some(p) => match std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|s| {
            ast::read(&s).map_err(|e| format!("{e}")).map(|n| ast::encode(&n))
        }) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("cannot read/encode input {p}: {e}");
                return ExitCode::from(2);
            }
        },
        None => Vec::new(),
    };

    match host::run_compiler_component(&component, &input_bytes) {
        Ok(host::CompileOutcome::Ok(bytes)) => {
            println!("compile → Ok ({} bytes): {:?}", bytes.len(), bytes);
            ExitCode::SUCCESS
        }
        Ok(host::CompileOutcome::Diagnostics(diags)) => {
            println!("compile → Diagnostics: {diags:?}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("compile run error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_ignition(path: Option<&str>) -> ExitCode {
    // Default ignition programs: two structurally-different modules differing only in a
    // constant, so a conforming compilation reflects the program, not a transcript.
    let (src_a, src_b): (String, String) = match path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(s) => (s, String::new()),
            Err(e) => {
                eprintln!("cannot read {p}: {e}");
                return ExitCode::from(2);
            }
        },
        None => (
            "(module answer (def (main) 42))".to_string(),
            "(module answer (def (main) 7))".to_string(),
        ),
    };

    println!("== Ignition: cdz-rustc compiles a program to a component and runs it ==\n");

    let a = match compile_and_run(&src_a) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("IGNITION FAIL: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("compiled A: {} bytes → ran → {:?}", a.0.len(), a.1);

    // Reproducibility: recompiling A is byte-identical.
    let a2 = match compile_and_run(&src_a) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("IGNITION FAIL: recompile A: {e}");
            return ExitCode::FAILURE;
        }
    };
    if a.0 != a2.0 {
        eprintln!("IGNITION FAIL: recompilation of A is not byte-identical");
        return ExitCode::FAILURE;
    }
    println!("  ✓ recompiling A is byte-identical ({} bytes)", a2.0.len());

    if !src_b.is_empty() {
        let b = match compile_and_run(&src_b) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("IGNITION FAIL: {e}");
                return ExitCode::FAILURE;
            }
        };
        println!("compiled B: {} bytes → ran → {:?}", b.0.len(), b.1);
        let diff = a.0.iter().zip(&b.0).filter(|(x, y)| x != y).count()
            + (a.0.len() as isize - b.0.len() as isize).unsigned_abs();
        if a.0 == b.0 {
            eprintln!("IGNITION FAIL: A and B are identical — compilation ignored the program");
            return ExitCode::FAILURE;
        }
        println!("  ✓ A and B differ in {diff} byte(s) — behavior comes from the compiled program");
    }

    println!("\nIGNITION: PASS");
    ExitCode::SUCCESS
}

fn compile_and_run(src: &str) -> Result<(Vec<u8>, host::RunOutcome), String> {
    let node = ast::read(src).map_err(|e| format!("parse: {e}"))?;
    let bytes = codegen::compile_program(&node).map_err(|d| format!("compile: {}", d.0))?;
    host::validate_component(&bytes).map_err(|e| format!("invalid component: {e}"))?;
    let (outcome, _) = host::run_component(&bytes, &[]).map_err(|e| format!("run: {e}"))?;
    Ok((bytes, outcome))
}

/// Prove the compiler COMPONENT (cdz-rustc built to wasm) agrees with the native compiler:
/// for every realized corpus program, compile through both and confirm the outcomes are
/// identical — same component bytes on success, same diagnostic code on rejection/decline.
/// This is the two-implementations-of-one-compiler agreement at the tooling level, and the
/// systematic proof that the wasm build of cdz-rustc is faithful to the native reference.
fn run_component_check(component_path: &str, dir: &str) -> ExitCode {
    let component_bytes = match std::fs::read(component_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read compiler component {component_path}: {e}");
            eprintln!("(build it: cd crates/cdz-compiler-component && cargo component build --release --target wasm32-unknown-unknown)");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = host::validate_component(&component_bytes) {
        eprintln!("compiler component is not valid: {e}");
        return ExitCode::FAILURE;
    }

    let loads = match corpus::load_cases(dir) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot read corpus at {dir}: {e}");
            return ExitCode::from(2);
        }
    };

    let (mut agree, mut disagree, mut skipped, mut declined, mut soft) = (0, 0, 0, 0, 0);
    for load in &loads {
        let case = match load {
            corpus::CaseLoad::Parsed(c) => c,
            corpus::CaseLoad::Malformed { .. } => continue,
        };
        if corpus::first_unrealized(&case.needs).is_some() {
            skipped += 1;
            continue;
        }
        let program = corpus::as_program(&case.input);
        // Native path.
        let native = codegen::compile_program(&program);
        // Component path: supply the program's canonical binary AST bytes.
        let ast_bytes = ast::encode(&program);
        let via_component = match host::run_compiler_component(&component_bytes, &ast_bytes) {
            Ok(o) => o,
            Err(e) => {
                println!("  DISAGREE {}  (component run error: {e})", case.description);
                disagree += 1;
                continue;
            }
        };
        if outcomes_match(&native, &via_component) {
            agree += 1;
        } else if let (Ok(native_bytes), host::CompileOutcome::Ok(comp_bytes)) = (&native, &via_component) {
            // The compiler produced a component whose bytes DIFFER from native's. Byte-difference is
            // NOT itself a miscompile — a decline stub and an honest-but-differently-shaped component
            // both differ from native's bytes. Classify by RUNTIME BEHAVIOR, not byte identity or
            // entry-func syntax (ask-33): run BOTH compiled programs and compare what they DO.
            //   - a bare-`unreachable` entry is a fast-path decline (no need to run);            [decline]
            //   - else run both: component TRAPS where native produces a VALUE → a HIDDEN DECLINE
            //     (compiler.cdz emitted setup-then-trap / a call to a trapping stub — an honest
            //     frontier, not a wrong answer);                                                 [decline]
            //   - both run to a value, EQUAL → SOFT (byte-differ, same observable behavior);        [soft]
            //   - both run to a value, DIFFERENT → the real MISCOMPILE (runs to a wrong value);  [disagree]
            //   - component runs to a value where native TRAPS → the component computed something
            //     native does not → a real disagreement.                                        [disagree]
            // So `disagree` becomes "runs to an observably-wrong result", the actionable frontier
            // (SPEC-BACKLOG #26/#33 — run-the-artifact, not the entry-func proxy).
            if host::is_decline_stub(comp_bytes) {
                declined += 1;
            } else {
                let native_run = host::run_component(native_bytes, &[]).map(|(o, _)| o);
                let comp_run = host::run_component(comp_bytes, &[]).map(|(o, _)| o);
                match (native_run, comp_run) {
                    (Ok(host::RunOutcome::Value(nv)), Ok(host::RunOutcome::Value(cv))) => {
                        if nv == cv {
                            soft += 1;
                        } else {
                            disagree += 1;
                            println!("  DISAGREE {}  native ran → {nv:?}  component ran → {cv:?}", case.description);
                        }
                    }
                    // Component traps where native produces a value → a hidden decline (honest frontier).
                    (Ok(host::RunOutcome::Value(_)), Ok(host::RunOutcome::Trap(_))) => {
                        declined += 1;
                    }
                    // Native traps but the component runs to a value → the component computed something
                    // native does not: a real disagreement.
                    (Ok(host::RunOutcome::Trap(_)), Ok(host::RunOutcome::Value(cv))) => {
                        disagree += 1;
                        println!("  DISAGREE {}  native TRAPS  component ran → {cv:?}", case.description);
                    }
                    // BOTH TRAP. The trap-CAUSE discriminator (ask-26): a decline and a semantic trap are
                    // indistinguishable by the trap observable alone. But we only reach here because
                    // `is_decline_stub(comp_bytes)` was FALSE (a bare-`unreachable` decline was already
                    // classified `decline` above) — so the component RAN REAL LOGIC and then trapped, exactly
                    // as native did. Two non-stub components that both trap where the program's semantic is a
                    // trap (`(/ 5 0)`, an out-of-range byte) are a genuine SEMANTIC-TRAP AGREEMENT — the
                    // compiler executed the trapping semantic, not declined. Counting it `agree` (not
                    // `decline`) means a WRONG trapping check (off-by-one range, no-trap-on-valid) can no
                    // longer hide as a coincidental decline: it would run to a value / a different outcome and
                    // fall into a disagree/soft arm above. (The in-range companion cases in the corpus are the
                    // other half — they must produce a VALUE, so a decline that traps on everything fails them
                    // and shows as a decline there.)
                    (Ok(host::RunOutcome::Trap(_)), Ok(host::RunOutcome::Trap(_))) => {
                        agree += 1;
                    }
                    // A run couldn't be evaluated (a component with no scalar `run()`, a host error, or a
                    // Suspended host-call): neither produced a comparable observable → treat as a decline,
                    // not a miscompile.
                    _ => declined += 1,
                }
            }
        } else if let (Err(_), host::CompileOutcome::Ok(comp_bytes)) = (&native, &via_component) {
            // NATIVE REJECTS/DECLINES the program, but the component emitted bytes. The ask-33
            // decline-discriminator applies SYMMETRICALLY here — it previously ran only on the
            // native=Ok path, so an honest decline on the native-REJECTS branch fell straight to the
            // final `else` → `disagree`, WITHOUT ever checking whether the component's `Ok` is a bare-
            // `unreachable` decline stub. That inflated the disagree count and over-stated the
            // remaining type-checker work (ask-30): a compiler that emits a non-functional stub for an
            // ill-typed program has HONESTLY DECLINED to reject it, not MIS-ACCEPTED it.
            //   - a bare-`unreachable` stub → the compiler emitted no real logic → honest decline; [decline]
            //   - else RUN it: a component that TRAPS ran real logic but produced NO value — again an
            //     honest frontier (it did not compile the ill-typed program to a working answer); [decline]
            //   - a component that runs to a VALUE compiled a program native REJECTS into a working
            //     program → the REAL ask-30 mis-accept (an ill-typed program silently accepted). [disagree]
            if host::is_decline_stub(comp_bytes) {
                declined += 1;
            } else {
                match host::run_component(comp_bytes, &[]).map(|(o, _)| o) {
                    Ok(host::RunOutcome::Value(cv)) => {
                        disagree += 1;
                        println!("  DISAGREE {}  native={}  component ran → {cv:?} (compiled a program native rejects)",
                            case.description, describe_native(&native));
                    }
                    // Traps, no runnable `run()`, or a host error → produced no wrong value → honest decline.
                    _ => declined += 1,
                }
            }
        } else {
            disagree += 1;
            println!("  DISAGREE {}  native={} component={}", case.description,
                describe_native(&native), describe_component(&via_component));
        }
    }
    println!("\ncomponent-check: {agree} agree, {disagree} disagree, {soft} soft, {declined} decline, {skipped} skip");
    if disagree == 0 {
        // `soft` = byte-differ but the compiled program runs to the SAME value as native's — an
        // observable agreement, not a miscompile (the compiler emitted different-but-correct bytes).
        // `decline` = the compiler honestly could not handle it (its output traps where native runs,
        // or it is a bare-`unreachable` stub). Only a `disagree` (runs to a wrong value) fails the gate.
        println!("COMPONENT-CHECK: PASS — the wasm compiler component agrees with the native compiler on {agree} programs ({soft} soft-agree, {declined} honest declines)");
        ExitCode::SUCCESS
    } else {
        println!("COMPONENT-CHECK: FAIL ({disagree} disagree — run to an observably-wrong result)");
        ExitCode::FAILURE
    }
}

/// Do the native and component compile outcomes match? Ok ⇔ identical bytes; rejection ⇔ same
/// leading diagnostic code; a native plain decline ⇔ a component decline diagnostic.
fn outcomes_match(
    native: &Result<Vec<u8>, codegen::Decline>,
    component: &host::CompileOutcome,
) -> bool {
    match (native, component) {
        (Ok(a), host::CompileOutcome::Ok(b)) => a == b,
        (Err(d), host::CompileOutcome::Diagnostics(diags)) => {
            let ncode = d.code().unwrap_or("CDZ0000");
            diags.first().map_or(false, |(code, _)| code == ncode)
        }
        _ => false,
    }
}

fn describe_native(n: &Result<Vec<u8>, codegen::Decline>) -> String {
    match n {
        Ok(b) => format!("ok({} bytes)", b.len()),
        Err(d) => format!("{}", d),
    }
}

fn describe_component(c: &host::CompileOutcome) -> String {
    match c {
        host::CompileOutcome::Ok(b) => format!("ok({} bytes)", b.len()),
        host::CompileOutcome::Diagnostics(ds) => {
            format!("diagnostics{:?}", ds.iter().map(|(c, _)| c).collect::<Vec<_>>())
        }
    }
}
