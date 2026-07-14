//! The oracle: does compiling this program reveal a bug?
//!
//! The compiler's contract is "errors are DATA": every legitimate "no" comes back as a
//! `Diagnostic` (a coded rejection) or an uncoded decline, and `compile_component` never returns a
//! `Result::Err` by panicking — it returns the error. So the oracle is sharp:
//!
//! * `Ok(component)` that VALIDATES → [`Verdict::Compiled`]   (not a bug)
//! * `Ok(component)` that FAILS to validate → [`Verdict::InvalidWasm`] (**a bug — a miscompile**)
//! * `Err(diagnostic)`              → [`Verdict::Declined`]   (not a bug — expected output)
//! * an unwinding **panic**         → [`Verdict::Crash`]      (**a bug**)
//!
//! A panic means an internal invariant (`.unwrap()` / `.expect(` / `unreachable!` / `panic!` / an
//! index or overflow) fired — the compiler is never supposed to do that on any input. We catch it
//! with [`std::panic::catch_unwind`]. This works even though `compile_component` runs the compile
//! on a dedicated 64 MB guard-stack worker thread, because that wrapper RE-RAISES a worker panic on
//! the caller via `resume_unwind` — so the caller's `catch_unwind` observes it. The same guard
//! stack means a deep-but-finite recursion hits the semantic depth limit and DECLINES rather than
//! overflowing, so a legitimately deep program is not a false crash.
//!
//! **Wasm-output validation.** A compile that returns `Ok` only means the backend didn't panic — it
//! says nothing about whether the emitted component is well-formed. We therefore run every emitted
//! component through `wasmparser`'s component validator (the SAME `WasmFeatures::all()` check
//! rcdzc's own tests assert emitted components pass). A component that fails to validate is a
//! **backend miscompile** — the compiler produced structurally-invalid wasm and reported success.
//! This catches an entire bug class the crash/hang oracle is blind to, with no execution required.
//!
//! Timeouts (a runaway loop in the compiler) can't be caught by `catch_unwind` — unwinding never
//! begins. Those are the driver's job: it runs a suspect program in a subprocess under a wall-clock
//! budget ([`crate::driver`]). This module is purely the in-process compile oracle.
//!
//! ## Differential oracle (planned)
//!
//! A further oracle will compile a program to two backends (`Target::Wasm` and `Target::Rust`), run
//! both, and compare the canonical result strings; a disagreement is a miscompile of a DIFFERENT
//! kind (valid wasm, wrong value). That needs the wasmtime host + `rustc`, so it lives behind the
//! subprocess path, layered on later.

use std::any::Any;
use std::backtrace::Backtrace;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, Once, OnceLock};

use crate::generator::Program;

/// The outcome of running one generated program through the compile path.
#[derive(Clone, Debug)]
pub enum Verdict {
    /// Compiled cleanly to a component. Not a bug. (We keep only the length — the bytes are
    /// discarded; we're fuzzing the compiler, not collecting components.)
    Compiled { component_len: usize },
    /// The compiler rejected or declined the program AS DATA — the expected, correct "no". Not a
    /// bug. `code` is the `CDZ####` for a coded rejection, `None` for an uncoded decline.
    Declined {
        code: Option<String>,
        message: String,
    },
    /// The generated source did not parse. Not a COMPILER finding (the generator should only emit
    /// parseable text); surfaced separately so the driver can count it as a generator-quality
    /// signal rather than a crash.
    ParseError(String),
    /// The compiler reported success but the emitted component FAILED wasm validation. **A bug** —
    /// a backend miscompile that produced structurally-invalid wasm. `detail` is the validator's
    /// error (used for dedup + the triage note).
    InvalidWasm {
        detail: String,
        component_len: usize,
    },
    /// A panic escaped the compile path. **A bug.**
    Crash(CrashInfo),
}

/// Everything captured about a crash — enough to dedup it (by [`site`](CrashInfo::site)) and to
/// write an actionable triage note.
#[derive(Clone, Debug)]
pub struct CrashInfo {
    /// The panic origin as `file:line:col`. Thanks to `#[track_caller]`, for `.unwrap()`/`.expect(`
    /// this is the CALL site in the compiler source (not libcore), which is exactly the crash site
    /// we dedup on. `None` only if the runtime withheld a location.
    pub site: Option<String>,
    /// The panic message (payload downcast to a string).
    pub message: String,
    /// A captured backtrace at the panic point (best-effort; `force_capture`, so present even
    /// without `RUST_BACKTRACE`). Used only in the triage note.
    pub backtrace: String,
}

impl Verdict {
    /// True iff this verdict is a filable finding (a crash or invalid-wasm miscompile — timeouts are
    /// produced by the driver).
    pub fn is_finding(&self) -> bool {
        matches!(self, Verdict::Crash(_) | Verdict::InvalidWasm { .. })
    }
}

/// Validate an emitted component the way rcdzc's own tests do — `WasmFeatures::all()`, whole-module.
/// `Ok(())` = well-formed; `Err(msg)` = the validator's rejection (a backend miscompile).
pub fn validate_component(bytes: &[u8]) -> Result<(), String> {
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(bytes)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ── panic capture ───────────────────────────────────────────────────────────────────────────
//
// The panic hook fires on whichever thread panics (here, the compile worker thread), BEFORE the
// unwind that `catch_unwind` later observes on the caller. So a process-global hook that stashes
// the location/message/backtrace into a slot is how we recover the site: `catch_unwind` tells us
// THAT it panicked, the slot tells us WHERE. The hook is silent by default (fuzzing crashes a lot;
// we don't want the default hook spraying every one to stderr); set `CDZ_SMITH_PANIC_PASSTHROUGH`
// to also forward to the previous hook when debugging.

struct Captured {
    site: Option<String>,
    message: String,
    backtrace: String,
}

fn slot() -> &'static Mutex<Option<Captured>> {
    static SLOT: OnceLock<Mutex<Option<Captured>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Install the capturing panic hook (idempotent). Called by [`compile_catching`]; tests that don't
/// exercise the oracle keep the default hook.
pub fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let passthrough = std::env::var_os("CDZ_SMITH_PANIC_PASSTHROUGH").is_some();
        let prev = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let site = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
            let message = payload_string(info.payload());
            let backtrace = Backtrace::force_capture().to_string();
            *slot().lock().unwrap() = Some(Captured {
                site,
                message,
                backtrace,
            });
            if passthrough {
                prev(info);
            }
        }));
    });
}

fn payload_string(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Compile one program source in-process, catching any panic. This is the crash oracle.
pub fn compile_catching(source: &str) -> Verdict {
    install_panic_hook();

    // Parse the generated s-expr → the two-arena AST → the binary-AST bytes the compiler consumes.
    // This is the exact bridge rcdzc's own testkit uses. A parse failure is a generator-quality
    // signal, not a compiler finding.
    let arenas = match cadenza_syntax::sexpr::read(source) {
        Ok(a) => a,
        Err(e) => return Verdict::ParseError(e.0),
    };
    let bytes = cadenza_syntax::codec::encode(&arenas);

    // Clear the slot so, on a crash, we read THIS run's panic and not a stale one.
    *slot().lock().unwrap() = None;

    let result = panic::catch_unwind(AssertUnwindSafe(|| rcdzc::compile_component(&bytes)));

    match result {
        Ok(Ok(component)) => match validate_component(&component) {
            Ok(()) => Verdict::Compiled {
                component_len: component.len(),
            },
            // Compiled "successfully" but the bytes don't validate — a backend miscompile.
            Err(detail) => Verdict::InvalidWasm {
                detail,
                component_len: component.len(),
            },
        },
        Ok(Err(diag)) => Verdict::Declined {
            code: diag.code.clone(),
            message: diag.message.clone(),
        },
        Err(_) => {
            let cap = slot().lock().unwrap().take().unwrap_or(Captured {
                site: None,
                message: "<panic with no captured info>".to_string(),
                backtrace: String::new(),
            });
            Verdict::Crash(CrashInfo {
                site: cap.site,
                message: cap.message,
                backtrace: cap.backtrace,
            })
        }
    }
}

/// Convenience: run a generated [`Program`].
pub fn compile_program(program: &Program) -> Verdict {
    compile_catching(&program.source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trivial_program_compiles() {
        let v = compile_catching("(do (def (main) 42) (export main))");
        match v {
            Verdict::Compiled { .. } => {}
            other => panic!("expected Compiled, got {other:?}"),
        }
    }

    #[test]
    fn a_real_compile_produces_validating_wasm() {
        // A genuinely-compiling program must reach Compiled (validation passed) — i.e. the compiler
        // emits well-formed wasm, so validation does NOT spuriously flag good output.
        let v = compile_catching("(do (def (main) (+ 1 2)) (export main))");
        assert!(
            matches!(v, Verdict::Compiled { .. }),
            "expected Compiled, got {v:?}"
        );
    }

    #[test]
    fn garbage_bytes_do_not_validate() {
        // The validator rejects non-wasm — the mechanism behind the InvalidWasm verdict works.
        assert!(validate_component(b"not a wasm component").is_err());
        assert!(validate_component(&[]).is_err());
    }

    #[test]
    fn an_ill_typed_program_declines_not_crashes() {
        // Adding an Int to a String is a type error → a Diagnostic, never a panic.
        let v = compile_catching(r#"(do (def (main) (+ 1 "x")) (export main))"#);
        assert!(
            matches!(v, Verdict::Declined { .. }),
            "expected a clean decline, got {v:?}"
        );
    }

    #[test]
    fn unparseable_source_is_a_parse_error_not_a_crash() {
        let v = compile_catching("(do (def (main) ");
        assert!(matches!(v, Verdict::ParseError(_)), "got {v:?}");
    }

    /// The capture machinery works: a deliberate panic routed through `compile_component`'s
    /// worker-thread wrapper is observed as a Crash with a site. (We can't force `rcdzc` to panic
    /// on demand, so we validate the hook+catch_unwind plumbing directly here.)
    #[test]
    fn panic_capture_records_a_site() {
        install_panic_hook();
        *slot().lock().unwrap() = None;
        let r = panic::catch_unwind(AssertUnwindSafe(|| {
            rcdzc::run_with_compiler_stack(|| panic!("synthetic crash for the oracle test"))
        }));
        assert!(r.is_err(), "the panic should have unwound to us");
        let cap = slot()
            .lock()
            .unwrap()
            .take()
            .expect("hook captured the panic");
        assert!(
            cap.message.contains("synthetic crash"),
            "message: {}",
            cap.message
        );
        assert!(cap.site.is_some(), "a site should be recorded");
    }
}
