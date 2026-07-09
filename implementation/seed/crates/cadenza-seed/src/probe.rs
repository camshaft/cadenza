//! A reusable probe harness: compile one Cadenza program with `cdz-rustc` and report, in one
//! structured value, exactly where it landed — declined, rejected (with code), emitted invalid
//! bytes, ran to a value, or trapped. This is the systematic replacement for hand-running
//! `emit` + `wasm-tools` on every change: a test asserts a `Probe` outcome, and a compiler
//! emission/validation regression surfaces as a failing assertion rather than a manual eyeball.

use cdz_compiler::{ast, codegen};

use crate::host::{self, RunOutcome};

/// The outcome of probing one program, most-to-least "finished".
#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    /// The program did not parse.
    ParseError(String),
    /// The compiler declined (a construct it does not yet lower) — an honest backlog `todo`.
    Declined(String),
    /// The compiler rejected the program as ill-typed, with its diagnostic code.
    Rejected(String),
    /// The compiler emitted bytes that do NOT form a valid component (a compiler bug).
    InvalidComponent(String),
    /// A valid component that ran to a value, rendered to the host's comparison string.
    Value(String),
    /// A valid component that trapped at run time.
    Trap,
}

/// Probe a program's source text through the whole pipeline.
pub fn probe(src: &str) -> Probe {
    let node = match ast::read(src) {
        Ok(n) => n,
        Err(e) => return Probe::ParseError(e.to_string()),
    };
    probe_node(&node)
}

/// Probe an already-parsed program node (bare expressions are wrapped as a nullary `main`).
pub fn probe_node(node: &cdz_compiler::Node) -> Probe {
    let program = crate::corpus::as_program(node);
    match codegen::compile_program(&program) {
        Err(d) => match d.code() {
            Some(code) => Probe::Rejected(code.to_string()),
            None => Probe::Declined(d.0),
        },
        Ok(bytes) => {
            if let Err(e) = host::validate_component(&bytes) {
                return Probe::InvalidComponent(first_line(&e.to_string()));
            }
            match host::run_component(&bytes, &[]) {
                Ok((RunOutcome::Value(v), _)) => Probe::Value(v),
                Ok((RunOutcome::Trap(_), _)) => Probe::Trap,
                Ok((RunOutcome::Suspended(_), _)) => Probe::Trap,
                Err(e) => Probe::InvalidComponent(format!("run failed: {}", first_line(&e.to_string()))),
            }
        }
    }
}

/// The outcome of probing a `compile`-ENTRY program (a `(def (compile inputs) …)` exporting the
/// build-tool `compile` seam), driven over `input` bytes: where the WRAPPER landed, and — on a
/// successful run — the compiler component's own `CompileOutcome` (the produced bytes, or the
/// diagnostics it reported). This guards the compile-export ABIs (bytes / result / kinded-artifact)
/// end-to-end: build the component, run its `compile` over an input, decode the return value.
#[derive(Debug, Clone, PartialEq)]
pub enum CompileProbe {
    /// Did not parse / declined / rejected (with code) / emitted invalid bytes / trapped at run.
    ParseError(String),
    Declined(String),
    Rejected(String),
    InvalidComponent(String),
    RunError(String),
    /// `compile` ran and produced component bytes (the byte count), no error diagnostic.
    Ok(usize),
    /// `compile` ran and reported diagnostics (their `(code, message)` pairs), no component.
    Diagnostics(Vec<(String, String)>),
}

/// Probe a `compile`-entry program end-to-end: compile the SOURCE to a compiler component, then run
/// that component's `compile` export over `input` and report the decoded `CompileOutcome`.
pub fn probe_compile(src: &str, input: &[u8]) -> CompileProbe {
    let node = match ast::read(src) {
        Ok(n) => n,
        Err(e) => return CompileProbe::ParseError(e.to_string()),
    };
    let bytes = match codegen::compile_program(&node) {
        Err(d) => {
            return match d.code() {
                Some(code) => CompileProbe::Rejected(code.to_string()),
                None => CompileProbe::Declined(d.0),
            }
        }
        Ok(b) => b,
    };
    if let Err(e) = host::validate_component(&bytes) {
        return CompileProbe::InvalidComponent(first_line(&e.to_string()));
    }
    match host::run_compiler_component(&bytes, input) {
        Ok(host::CompileOutcome::Ok(out)) => CompileProbe::Ok(out.len()),
        Ok(host::CompileOutcome::Diagnostics(ds)) => CompileProbe::Diagnostics(ds),
        Err(e) => CompileProbe::RunError(first_line(&e)),
    }
}

/// The compiled component's byte length, or None if it did not compile to a valid component.
/// Useful for reproducibility / size-regression checks.
pub fn component_len(src: &str) -> Option<usize> {
    let node = ast::read(src).ok()?;
    let program = crate::corpus::as_program(&node);
    let bytes = codegen::compile_program(&program).ok()?;
    host::validate_component(&bytes).ok()?;
    Some(bytes.len())
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}
