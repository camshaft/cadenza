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
    /// A panic escaped the compile path. **A bug.** The crash may come from EITHER emit backend: the
    /// primary WebAssembly-component path, or the Rust-source backend (`compile` with `Target::Rust`).
    /// A Rust-backend-only panic has its message prefixed `[rust-backend]` so it dedups + triages
    /// distinctly from the same-site wasm crash.
    Crash(CrashInfo),
}

/// Everything captured about a crash — enough to dedup it (by [`site`](CrashInfo::site)) and to
/// write an actionable triage note.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// Read the panic this run captured out of the slot into a [`CrashInfo`]. Call ONLY after a
/// `catch_unwind` returned `Err` (the slot was cleared before the guarded call, so a present value
/// is THIS run's panic). `prefix`, when non-empty, is prepended to the message so a crash unique to
/// one emit backend dedups + triages distinctly from a same-site crash on the other.
fn capture_crash(prefix: &str) -> CrashInfo {
    let cap = slot().lock().unwrap().take().unwrap_or(Captured {
        site: None,
        message: "<panic with no captured info>".to_string(),
        backtrace: String::new(),
    });
    let message = if prefix.is_empty() {
        cap.message
    } else {
        format!("{prefix} {}", cap.message)
    };
    CrashInfo {
        site: cap.site,
        message,
        backtrace: cap.backtrace,
    }
}

/// Why a panic-guarded in-process COMPONENT compile did not yield a component (see
/// [`compile_component_catching`]).
pub enum ComponentFail {
    /// An errors-as-data DECLINE — the diagnostic code (used for the not-comparable reason). Expected
    /// output (a rejected program), never a bug.
    Declined(Option<String>),
    /// A compiler PANIC — captured with its site. This is a CRASH finding: the differential sweeps file
    /// it (like the crash oracle) and CONTINUE, instead of the unguarded native panic aborting the run.
    Crashed(CrashInfo),
}

/// Compile binary-AST `bytes` to a component IN-PROCESS, catching a compiler PANIC — for the differential
/// sweeps, which need the COMPONENT on success (unlike [`compile_catching`], which reports only a Verdict).
/// The wasm/rust `differential` and `run_ast_corpus` sweeps otherwise call `rcdzc::compile_component`
/// through the hang-watchdog `guard` but WITHOUT `catch_unwind`, so a compiler panic (e.g. a nullary
/// `(Set.of)` index-OOB at infer/node.rs) propagated straight up and ABORTED the whole campaign. Routing
/// the compile through this guard turns a panic into a filed crash finding + a continued sweep. Returns
/// `Ok(component)` on a clean compile, `Err(Declined(code))` on an errors-as-data decline, or
/// `Err(Crashed(info))` on a compiler panic (site captured via the same hook `compile_catching` uses).
pub fn compile_component_catching(bytes: &[u8]) -> Result<Vec<u8>, ComponentFail> {
    install_panic_hook();
    // Clear the slot so, on a crash, we read THIS compile's panic and not a stale one.
    *slot().lock().unwrap() = None;
    match panic::catch_unwind(AssertUnwindSafe(|| rcdzc::compile_component(bytes))) {
        Ok(Ok(component)) => Ok(component),
        Ok(Err(diag)) => Err(ComponentFail::Declined(diag.code)),
        Err(_) => Err(ComponentFail::Crashed(capture_crash(""))),
    }
}

/// Compile one program source in-process, catching any panic. This is the crash oracle.
///
/// TWO emit backends are driven per program. The primary WebAssembly-component path
/// ([`rcdzc::compile_component`]) yields the reported verdict — Compiled / InvalidWasm / Declined /
/// Crash. Then, whenever the wasm path did NOT itself crash, the **Rust-source backend** is driven
/// too (`compile` with [`rcdzc::Target::Rust`]) purely as a second crash oracle: a panic there is a
/// bug the wasm path can't reach (the backends share the front-end but diverge below the emit seam,
/// so `backend/rust/*` is fuzzed nowhere else). A Rust-backend DECLINE is expected output and is
/// ignored here — we only escalate a Rust-backend PANIC, and only if the wasm path was otherwise
/// clean, so the wasm verdict (the richer one, with the InvalidWasm miscompile oracle) always wins a
/// tie.
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
    compile_bytes_catching(&bytes)
}

/// The **binary-AST-entropy** oracle: treat the fuzzer's `&[u8]` as a binary-AST module.
///
/// This is the entry the next-gen engine drives — entropy IS the binary AST (`cadenza-ast` codec
/// bytes), seeded from real semantics-corpus encodings and mutated by libFuzzer, rather than s-expr
/// text run through the parser. Driving generation from the binary AST reaches the compiler with
/// well-formed, structurally-dense programs far more often than mutating text (the operator's
/// "generates a lot better").
///
/// We first DECODE-GATE the bytes. `codec::decode` is strict + total: it never panics — it cleanly
/// rejects malformed / truncated / non-tree bytes as a typed [`cadenza_syntax::codec::DecodeError`].
/// So a mutated blob that isn't a well-formed AST is classified as [`Verdict::ParseError`] (the
/// entropy-quality analog of a text parse failure) and never reaches the compiler as a spurious
/// decline — keeping [`Verdict::Declined`] meaning "the compiler rejected a WELL-FORMED program".
/// A blob that DOES decode is re-encoded to its canonical form (mutation can leave a well-formed but
/// non-canonical encoding; the compiler consumes the canonical form its own front-end produces) and
/// compiled through the exact same path as the text oracle — so a crash / invalid-wasm here is the
/// same finding class, both emit backends driven.
pub fn compile_catching_ast(ast_bytes: &[u8]) -> Verdict {
    install_panic_hook();

    let arenas = match cadenza_syntax::codec::decode_detailed(ast_bytes) {
        Ok(a) => a,
        Err(e) => return Verdict::ParseError(format!("decode: {e:?}")),
    };
    let bytes = cadenza_syntax::codec::encode(&arenas);
    compile_bytes_catching(&bytes)
}

/// Compile already-encoded binary-AST bytes in-process, catching any panic. The shared crash /
/// invalid-wasm oracle behind BOTH the text-source path ([`compile_catching`], which parses + encodes
/// first) and the binary-AST-entropy path ([`compile_catching_ast`], which decode-gates first): both
/// reduce to "here are the canonical bytes the compiler consumes".
///
/// TWO emit backends are driven per program (see [`compile_catching`] for the full rationale): the
/// primary WebAssembly-component path yields the reported verdict, and — only when that path did not
/// itself crash — the Rust-source backend is driven purely as a second crash oracle.
fn compile_bytes_catching(bytes: &[u8]) -> Verdict {
    // Clear the slot so, on a crash, we read THIS run's panic and not a stale one.
    *slot().lock().unwrap() = None;

    let result = panic::catch_unwind(AssertUnwindSafe(|| rcdzc::compile_component(bytes)));

    let wasm_verdict = match result {
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
        Err(_) => Verdict::Crash(capture_crash("")),
    };

    // The wasm path already crashed — that's the finding; don't also drive the Rust backend (it
    // would likely hit the same front-end fault and just add noise). Otherwise, fuzz the Rust
    // backend as a second crash oracle.
    if matches!(wasm_verdict, Verdict::Crash(_)) {
        return wasm_verdict;
    }
    if let Some(crash) = compile_rust_catching(bytes) {
        return Verdict::Crash(crash);
    }
    wasm_verdict
}

/// Drive the **Rust-source backend** for one program, catching a panic. Returns `Some(crash)` iff a
/// panic escaped (a bug); `None` for any non-panic outcome (a clean emit OR an expected decline —
/// both are fine, we're only mining the Rust backend for CRASHES here). See [`compile_catching`].
fn compile_rust_catching(bytes: &[u8]) -> Option<CrashInfo> {
    // Clear the slot again so we read the Rust-backend panic, not the (already consumed) wasm one.
    *slot().lock().unwrap() = None;

    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        rcdzc::host::run_with_compiler_stack(|| {
            rcdzc::compile(
                &[rcdzc::abi::Artifact::new(
                    rcdzc::abi::Artifact::KIND_AST,
                    "main",
                    bytes.to_vec(),
                )],
                &[rcdzc::Target::Rust],
            )
        })
    }));

    match result {
        // A produced artifact OR a decline (diagnostics, no artifact) — both are non-findings.
        Ok(_) => None,
        Err(_) => Some(capture_crash("[rust-backend]")),
    }
}

/// Convenience: run a generated [`Program`].
pub fn compile_program(program: &Program) -> Verdict {
    compile_catching(&program.source)
}

/// Compile a set of MODULE library files linked with an ENTRY program, catching panics — the
/// MULTI-MODULE analog of [`compile_catching`], for the cross-module / WIT-binding decline surface (where
/// the per-cell import/export gaps live). Each `(name, source)` in `modules` becomes a `KIND_AST` artifact
/// named `name` (an importable module); `entry_src` is the `"main"` artifact, which may `(import "name" …)`
/// from a module. [`rcdzc::compile`] links them and emits the wasm component. The reported [`Verdict`]
/// mirrors the single-program path — a coded rejection is preferred over an uncoded decline (the safety
/// ordering `compile_component` uses). WASM-only (no second Rust-backend crash oracle). A source that
/// fails to PARSE is a generator-quality [`Verdict::ParseError`], not a compiler finding.
pub fn compile_modules_catching(modules: &[(String, String)], entry_src: &str) -> Verdict {
    install_panic_hook();

    // Parse + encode each module + the entry into KIND_AST artifacts (the exact bridge the single path
    // uses). The module's artifact NAME is what an `(import "name" …)` resolves against.
    let mut artifacts = Vec::with_capacity(modules.len() + 1);
    for (name, src) in modules {
        let arenas = match cadenza_syntax::sexpr::read(src) {
            Ok(a) => a,
            Err(e) => return Verdict::ParseError(e.0),
        };
        artifacts.push(rcdzc::abi::Artifact::new(
            rcdzc::abi::Artifact::KIND_AST,
            name,
            cadenza_syntax::codec::encode(&arenas),
        ));
    }
    let entry = match cadenza_syntax::sexpr::read(entry_src) {
        Ok(a) => a,
        Err(e) => return Verdict::ParseError(e.0),
    };
    artifacts.push(rcdzc::abi::Artifact::new(
        rcdzc::abi::Artifact::KIND_AST,
        "main",
        cadenza_syntax::codec::encode(&entry),
    ));
    // A KIND_ENTRY artifact names which file is the package entry (its bytes are the entry file's name) —
    // the linker needs it to know where `main` lives and produce a component (see rcdzc link tests).
    artifacts.push(rcdzc::cli::entry_artifact("main"));

    // Clear the crash slot so we read THIS run's panic (see `compile_bytes_catching`).
    *slot().lock().unwrap() = None;
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        rcdzc::host::run_with_compiler_stack(|| rcdzc::compile(&artifacts, &[rcdzc::Target::Wasm]))
    }));

    let out = match result {
        Ok(out) => out,
        Err(_) => return Verdict::Crash(capture_crash("[multi-module]")),
    };
    wasm_output_verdict(&out)
}

/// Interpret a [`rcdzc::CompileOutput`] (from a multi-artifact `rcdzc::compile(&…, &[Target::Wasm])`) into
/// a [`Verdict`], the way `compile_component` does: the wasm component artifact → `Compiled` (or
/// `InvalidWasm` if it fails validation); no artifact → `Declined`, preferring a CODED error over an
/// uncoded one (the safety ordering). Shared by the multi-module + wit-world compile paths.
fn wasm_output_verdict(out: &rcdzc::CompileOutput) -> Verdict {
    match out.artifact(rcdzc::Target::Wasm.artifact_kind()) {
        Some(component) => match validate_component(component) {
            Ok(()) => Verdict::Compiled {
                component_len: component.len(),
            },
            Err(detail) => Verdict::InvalidWasm {
                detail,
                component_len: component.len(),
            },
        },
        None => {
            let coded = out
                .diagnostics
                .iter()
                .find(|d| d.severity == rcdzc::Severity::Error && d.code.is_some());
            let any_err = out
                .diagnostics
                .iter()
                .find(|d| d.severity == rcdzc::Severity::Error);
            match coded.or(any_err) {
                Some(d) => Verdict::Declined {
                    code: d.code.clone(),
                    message: d.message.clone(),
                },
                None => Verdict::Declined {
                    code: None,
                    message: "compilation produced no component".into(),
                },
            }
        }
    }
}

/// Compile a GUEST program against an imposed WIT WORLD, catching panics — the WIT-WORLD ABI boundary
/// (where the per-cell WIT-binding gaps live: v-inference world-import synth / v-rust-backend emit). The
/// guest is a `KIND_AST` `"main"` artifact; `world_src` is a `(world w (export <iface> (member …)))`
/// s-expr (encoded into a `KIND_WIT_WORLD` artifact); `iface` names the component. `rcdzc::compile` emits
/// the wasm component iff the guest fully + correctly implements the world's interface — otherwise it
/// cleanly DECLINES (a WIT-binding gap: a member type that does not marshal, a partial guest, …). Verdict
/// mirrors the single/multi-module paths. WASM-only. A parse failure of either source is a
/// [`Verdict::ParseError`] (generator-quality), not a compiler finding.
pub fn compile_world_catching(guest_src: &str, iface: &str, world_src: &str) -> Verdict {
    install_panic_hook();

    let guest = match cadenza_syntax::sexpr::read(guest_src) {
        Ok(a) => a,
        Err(e) => return Verdict::ParseError(e.0),
    };
    let world = match cadenza_syntax::sexpr::read(world_src) {
        Ok(a) => a,
        Err(e) => return Verdict::ParseError(e.0),
    };
    let artifacts = [
        rcdzc::abi::Artifact::new(
            rcdzc::abi::Artifact::KIND_AST,
            "main",
            cadenza_syntax::codec::encode(&guest),
        ),
        rcdzc::cli::component_name_artifact(iface),
        rcdzc::abi::Artifact::new(
            rcdzc::link::KIND_WIT_WORLD,
            "wit-world",
            cadenza_syntax::codec::encode(&world),
        ),
    ];

    *slot().lock().unwrap() = None;
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        rcdzc::host::run_with_compiler_stack(|| rcdzc::compile(&artifacts, &[rcdzc::Target::Wasm]))
    }));
    match result {
        Ok(out) => wasm_output_verdict(&out),
        Err(_) => Verdict::Crash(capture_crash("[wit-world]")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize tests that touch the process-global panic-capture [`slot`]. In PRODUCTION the slot
    /// is safe unsynchronized — a fuzzing process drives one compile at a time (libFuzzer forks a
    /// child per input; the PRNG driver is single-threaded). Only the test harness runs these in
    /// parallel threads, where one test's slot-clear could wipe another's captured panic between its
    /// `catch_unwind` and its read. This guard makes the slot-touching tests mutually exclusive.
    fn slot_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: Mutex<()> = Mutex::new(());
        GUARD.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn a_trivial_program_compiles() {
        let _g = slot_guard();
        let v = compile_catching("(do (def (main) 42) (export main))");
        match v {
            Verdict::Compiled { .. } => {}
            other => panic!("expected Compiled, got {other:?}"),
        }
    }

    /// The MULTI-MODULE path links a module library + an importing entry into one component (mirrors
    /// corpus 11-modules "an imported name resolves to a sibling file's exported definition" → 42).
    #[test]
    fn a_module_import_program_compiles() {
        let _g = slot_guard();
        let modules = [(
            "lib".to_string(),
            "(do (def (helper) 40) (export helper))".to_string(),
        )];
        let entry = "(do (import \"lib\" (helper)) (def (main) (+ (helper) 2)) (export main))";
        match compile_modules_catching(&modules, entry) {
            Verdict::Compiled { .. } => {}
            other => panic!("module import must compile, got {other:?}"),
        }
    }

    /// An import with NO matching module cleanly DECLINES (not a crash) — the multi-module path reports a
    /// decline the same way the single path does, so a module decline campaign is a clean gap hunt.
    #[test]
    fn an_unresolved_import_declines_not_crashes() {
        let _g = slot_guard();
        let entry = "(do (import \"absent\" (helper)) (def (main) (helper)) (export main))";
        match compile_modules_catching(&[], entry) {
            Verdict::Declined { .. } => {}
            other => panic!("unresolved import must decline, got {other:?}"),
        }
    }

    /// The WIT-WORLD path: a guest that FULLY implements a single-member interface world compiles to a
    /// component (the `two_member_record_world` shape from rcdzc's link tests, single member).
    #[test]
    fn a_full_wit_world_guest_compiles() {
        let _g = slot_guard();
        let world =
            "(world w (export iface (member f (func (param m (record (a s64))) (result s64)))))";
        let guest = "(module m (def (f (: m (Record (a Int64)))) (. m a)) (export f))";
        match compile_world_catching(guest, "cadenza:demo/iface", world) {
            Verdict::Compiled { .. } => {}
            other => panic!("a full wit-world guest must compile, got {other:?}"),
        }
    }

    /// A guest that does NOT match the world's declared interface member — across several mismatch
    /// SHAPES — is CLEANLY HANDLED: `Compiled` or `Declined`, NEVER a `Crash` or `InvalidWasm`. This
    /// pins the fuzzer's SOUNDNESS invariant on the wit-world path (a future compiler change that made
    /// any of these shapes ICE or emit invalid wasm is a regression this catches), independent of the
    /// still-open world-export-ENFORCEMENT question (should a guest that ignores the declared export's
    /// name/type DECLINE rather than compile?). A 2026-08-29 probe found rcdzc currently does NOT enforce
    /// the declared export's name or type — the missing-`f` and wrong-signature-`f` cases COMPILE, and
    /// only "no exports at all" declines (for a generic "nothing is public" reason). Whether that is a
    /// soundness gap or deferred-to-downstream-validation is with the operator (an `ask` was sent); this
    /// test deliberately asserts only the no-crash/no-invalid-wasm floor, so it does not enshrine the
    /// compile-vs-decline decision either way. Flip the assertion to expect `Declined` on the cases below
    /// once/if the enforcement is added.
    #[test]
    fn a_mismatched_wit_world_guest_is_cleanly_handled() {
        let _g = slot_guard();
        let world =
            "(world w (export iface (member f (func (param m (record (a s64))) (result s64)))))";
        // Each is a guest that FAILS to satisfy the declared `export iface (member f …)` in a distinct
        // way — none may crash or emit invalid wasm.
        let mismatched_guests: &[(&str, &str)] = &[
            // Exports an unrelated `g`; the declared `f` is never implemented.
            (
                "unrelated-export",
                "(module m (def (g (: m (Record (a Int64)))) (. m a)) (export g))",
            ),
            // Implements `f` but with a completely different signature than the world's func type.
            (
                "wrong-signature-f",
                "(module m (def (f (: x Int64)) x) (export f))",
            ),
            // Implements `f` correctly AND exports an extra unrelated `g`.
            (
                "f-plus-extra-g",
                "(module m (def (f (: m (Record (a Int64)))) (. m a)) (def (g) 1) (export f) (export g))",
            ),
        ];
        for (label, guest) in mismatched_guests {
            match compile_world_catching(guest, "cadenza:demo/iface", world) {
                Verdict::Compiled { .. } | Verdict::Declined { .. } => {}
                other => panic!(
                    "mismatched wit-world guest ({label}) must be cleanly handled \
                     (Compiled|Declined, never Crash/InvalidWasm), got {other:?}"
                ),
            }
        }
    }

    #[test]
    fn a_real_compile_produces_validating_wasm() {
        // A genuinely-compiling program must reach Compiled (validation passed) — i.e. the compiler
        // emits well-formed wasm, so validation does NOT spuriously flag good output.
        let _g = slot_guard();
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
        let _g = slot_guard();
        let v = compile_catching(r#"(do (def (main) (+ 1 "x")) (export main))"#);
        assert!(
            matches!(v, Verdict::Declined { .. }),
            "expected a clean decline, got {v:?}"
        );
    }

    #[test]
    fn the_rust_backend_pass_does_not_spuriously_escalate_a_clean_program() {
        // A program that compiles cleanly to wasm must ALSO drive the Rust backend without a panic,
        // so the added second oracle keeps the verdict Compiled (it neither crashes nor is filed on
        // a Rust-backend decline).
        let _g = slot_guard();
        let v = compile_catching("(do (def (main) (+ 1 2)) (export main))");
        assert!(
            matches!(v, Verdict::Compiled { .. }),
            "the Rust-backend pass must not turn a clean program into {v:?}"
        );
    }

    #[test]
    fn a_rust_backend_crash_message_is_prefixed_and_dedups_apart() {
        // The `[rust-backend]` prefix is what makes a Rust-backend-only panic dedup + triage
        // distinctly from a same-site wasm crash; validate the prefixing directly (we can't force
        // rcdzc to panic in only one backend on demand).
        let _g = slot_guard();
        install_panic_hook();
        *slot().lock().unwrap() = None;
        let r = panic::catch_unwind(AssertUnwindSafe(|| {
            rcdzc::run_with_compiler_stack(|| panic!("synthetic rust-backend crash"))
        }));
        assert!(r.is_err());
        let crash = capture_crash("[rust-backend]");
        assert!(
            crash.message.starts_with("[rust-backend] "),
            "message should carry the backend tag: {}",
            crash.message
        );
        assert!(crash.message.contains("synthetic rust-backend crash"));
    }

    #[test]
    fn unparseable_source_is_a_parse_error_not_a_crash() {
        let _g = slot_guard();
        let v = compile_catching("(do (def (main) ");
        assert!(matches!(v, Verdict::ParseError(_)), "got {v:?}");
    }

    // ── binary-AST-entropy path (`compile_catching_ast`) ─────────────────────────────────────────

    /// Encode a source program to canonical binary-AST bytes — the shape the entropy path consumes.
    fn ast_bytes_of(source: &str) -> Vec<u8> {
        let arenas = cadenza_syntax::sexpr::read(source).expect("test source parses");
        cadenza_syntax::codec::encode(&arenas)
    }

    #[test]
    fn a_valid_binary_ast_blob_compiles() {
        // A well-formed binary-AST module reaches the compiler and compiles to validating wasm —
        // i.e. the decode-gate + re-encode + compile path is equivalent to the text path for a real
        // program.
        let _g = slot_guard();
        let bytes = ast_bytes_of("(do (def (main) (+ 1 2)) (export main))");
        let v = compile_catching_ast(&bytes);
        assert!(
            matches!(v, Verdict::Compiled { .. }),
            "expected Compiled from a valid AST blob, got {v:?}"
        );
    }

    #[test]
    fn garbage_bytes_are_a_parse_error_not_a_crash() {
        // The decode-gate is strict + total: arbitrary bytes are rejected cleanly (a bad header /
        // bad tag), classified as the entropy-quality ParseError analog — NEVER a panic or a
        // spurious Declined that would masquerade as a compiler rejection of a real program.
        let _g = slot_guard();
        let v = compile_catching_ast(b"not a binary ast at all");
        assert!(matches!(v, Verdict::ParseError(_)), "got {v:?}");
        assert!(matches!(compile_catching_ast(&[]), Verdict::ParseError(_)));
    }

    #[test]
    fn a_truncated_valid_blob_is_a_parse_error_not_a_crash() {
        // A mid-stream truncation of a valid encoding (the classic libFuzzer mutation) decodes to a
        // `Truncated` error → ParseError, never an over-read panic.
        let _g = slot_guard();
        let full = ast_bytes_of("(do (def (main) (+ 1 2)) (export main))");
        let truncated = &full[..full.len() / 2];
        let v = compile_catching_ast(truncated);
        assert!(matches!(v, Verdict::ParseError(_)), "got {v:?}");
    }

    /// The capture machinery works: a deliberate panic routed through `compile_component`'s
    /// worker-thread wrapper is observed as a Crash with a site. (We can't force `rcdzc` to panic
    /// on demand, so we validate the hook+catch_unwind plumbing directly here.)
    #[test]
    fn panic_capture_records_a_site() {
        let _g = slot_guard();
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
