//! The differential oracle: run the SAME program on two backends and compare the values.
//!
//! The crash/hang oracle ([`crate::oracle`]) catches a compiler that PANICS, and the wasm-validity
//! oracle catches one that emits structurally-INVALID wasm. Neither catches the subtlest miscompile:
//! the backend emits *valid* wasm (or *compilable* Rust) that computes the **wrong value**. That bug
//! is invisible in isolation — you need a second, independent implementation of the same semantics to
//! notice the disagreement. The compiler HAS one: the two emit backends share the front-end but
//! diverge below the emit seam (`backend/wasm/*` vs `backend/rust/*`), so a lowering bug on one side
//! that the other doesn't share shows up as a VALUE disagreement.
//!
//! This oracle runs a program both ways and compares the canonical result strings:
//!
//! * **wasm** — compile with [`rcdzc::compile_component`], run the component IN-PROCESS with
//!   [`cdz_run::run`] (resolving the value-heap runtime by content address from the store, exactly as
//!   `cdz run` does), and take the rendered [`cdz_run::Outcome`].
//! * **rust** — shell `cdz run-rust` (source on stdin → one verdict line on stdout), which emits
//!   `--target rust`, `rustc`-compiles + runs it, and renders the result with the SAME
//!   `cdz-rust-render` crate the wasm path's `cdz-run` uses — so a `value` on each side is
//!   byte-comparable.
//!
//! ## What is (and isn't) a finding
//!
//! Both sides map to a [`Side`] outcome. The pairing rules — the whole point of the oracle:
//!
//! | wasm \ rust | Value(a)                       | Trap(_)            | Declined      |
//! |-------------|--------------------------------|--------------------|---------------|
//! | Value(b)    | **MISMATCH if a≠b** (finding)  | **MISMATCH**       | agree (skip)  |
//! | Trap(_)     | **MISMATCH**                   | agree (skip)†      | agree (skip)  |
//! | Declined    | agree (skip)                   | agree (skip)       | agree (skip)  |
//!
//! * A **value disagreement** (both ran to a value, values differ) is the headline finding — a
//!   valid-artifact wrong-value miscompile.
//! * A **liveness disagreement** (one ran to a value, the other trapped) is also a finding: one
//!   backend computes a result where the other faults.
//! * A **`Declined` on EITHER side is never a mismatch.** The Rust backend supports a strict subset
//!   (compound results, host effects, etc. decline there), and the shared front-end declines the same
//!   unimplemented constructs on both — a decline means "not comparable here", i.e. coverage-not-yet,
//!   not a bug. This keeps the oracle SOUND: it only ever fires when both sides genuinely produced a
//!   comparable outcome.
//! * †Trap-vs-trap is treated as AGREEMENT regardless of message. Both backends trapping is the
//!   correct behavior; the trap *reason* text differs by backend (a wasm trap string vs a Rust panic
//!   message) and is not meaningfully comparable, so we do not diff it. (A future refinement could
//!   compare a normalized trap KIND; today any-trap-vs-any-trap agrees.)
//!
//! A crash on EITHER backend is out of scope here — [`crate::oracle::compile_catching`] already mines
//! both backends for panics. This oracle assumes a non-crashing compile and asks only "do the two
//! agree on the VALUE?".

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// One backend's outcome for a program, reduced to the three cases the pairing rules compare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Side {
    /// Ran to a value, rendered to canonical text (bare scalar / `(tuple …)` / …).
    Value(String),
    /// Trapped at run time (message kept for the note; not used in the comparison).
    Trap(String),
    /// The front-end rejected the program, or this backend does not emit it yet — NOT comparable,
    /// treated as coverage-not-yet. `detail` is a short reason for the triage note.
    Declined(String),
}

/// The verdict of comparing the two sides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Diff {
    /// The two backends disagree — a miscompile. `wasm`/`rust` are the rendered outcomes, `kind`
    /// distinguishes a value disagreement from a liveness (value-vs-trap) one.
    Mismatch {
        kind: MismatchKind,
        wasm: String,
        rust: String,
    },
    /// The backends agree (same value, both trapped, or at least one declined) — not a finding.
    Agree,
    /// The comparison could not run (a harness failure driving `cdz run-rust`, e.g. the binary was
    /// not found). Distinct from a compiler outcome — the caller logs it, it is never filed.
    Unavailable(String),
}

/// Which flavor of disagreement fired — drives the finding's signature + note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MismatchKind {
    /// Both backends ran to a value, and the values differ.
    Value,
    /// One backend ran to a value, the other trapped (a liveness disagreement).
    Liveness,
}

impl MismatchKind {
    pub fn tag(self) -> &'static str {
        match self {
            MismatchKind::Value => "value",
            MismatchKind::Liveness => "liveness",
        }
    }
}

/// Compare the wasm and rust outcomes for one program per the pairing rules. Pure — the two sides are
/// produced by [`run_wasm`] / [`run_rust`]; splitting it out keeps the rules unit-testable without a
/// compiler or a subprocess.
pub fn compare(wasm: &Side, rust: &Side) -> Diff {
    match (wasm, rust) {
        // A decline on EITHER side means "not comparable here" — never a mismatch (soundness).
        (Side::Declined(_), _) | (_, Side::Declined(_)) => Diff::Agree,
        // Both ran to a value: agree iff the canonical strings are identical.
        (Side::Value(a), Side::Value(b)) => {
            if a == b {
                Diff::Agree
            } else {
                Diff::Mismatch {
                    kind: MismatchKind::Value,
                    wasm: a.clone(),
                    rust: b.clone(),
                }
            }
        }
        // Both trapped — correct behavior on both; the reason text is not backend-comparable.
        (Side::Trap(_), Side::Trap(_)) => Diff::Agree,
        // One value, one trap — a liveness disagreement.
        (Side::Value(v), Side::Trap(t)) => Diff::Mismatch {
            kind: MismatchKind::Liveness,
            wasm: format!("value {v}"),
            rust: format!("trap {t}"),
        },
        (Side::Trap(t), Side::Value(v)) => Diff::Mismatch {
            kind: MismatchKind::Liveness,
            wasm: format!("trap {t}"),
            rust: format!("value {v}"),
        },
    }
}

/// Run one program through the WASM backend IN-PROCESS: compile to a component with `rcdzc`, then run
/// it with `cdz-run` (resolving the value-heap runtime by content address from `store`). A front-end
/// reject / backend decline → [`Side::Declined`]; a value → [`Side::Value`]; a run-time trap →
/// [`Side::Trap`].
///
/// `store` is the content-addressed runtime store (`<store>/<hash>.wasm`), normally
/// `<repo>/target/cadenza-store`. A component that imports no runtime (a pure scalar) needs no store
/// entry; one that does and can't resolve it yields `Declined` (a harness/environment gap, not a
/// compiler bug — we don't file it).
pub fn run_wasm(source: &str, store: &std::path::Path) -> Side {
    // Parse + encode to the binary AST the compiler consumes (the same bridge `compile_catching` uses).
    let arenas = match cadenza_syntax::sexpr::read(source) {
        Ok(a) => a,
        // Unparseable generated text is a generator-quality issue, not comparable — treat as declined.
        Err(e) => return Side::Declined(format!("parse error: {}", e.0)),
    };
    let bytes = cadenza_syntax::codec::encode(&arenas);

    // Compile to a component. A rejection/decline (errors-as-data) → not comparable.
    let component = match rcdzc::compile_component(&bytes) {
        Ok(c) => c,
        Err(diag) => {
            return Side::Declined(
                diag.code
                    .clone()
                    .unwrap_or_else(|| "wasm-decline".to_string()),
            );
        }
    };

    // Resolve the value-heap runtime by content address, if the component imports one.
    let runtime = match cdz_run::required_runtime(&component) {
        Ok(Some(req)) => {
            let path = store.join(format!("{}.wasm", req.hash));
            match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                // Can't resolve the runtime → environment gap, not a compiler bug. Don't file.
                Err(e) => {
                    return Side::Declined(format!(
                        "runtime {} not in store {}: {e}",
                        req.hash,
                        store.display()
                    ));
                }
            }
        }
        Ok(None) => None,
        Err(e) => return Side::Declined(format!("required-runtime read failed: {e}")),
    };

    let opts = cdz_run::RunOpts {
        runtime,
        runtime_cache_dir: Some(store.to_path_buf()),
        ..Default::default()
    };
    match cdz_run::run(&component, &opts) {
        Ok(cdz_run::Outcome::Value(v)) => Side::Value(v.trim().to_string()),
        Ok(cdz_run::Outcome::Trap(t)) => Side::Trap(t),
        // A run harness error (invalid component, unresolvable import) — not a value disagreement;
        // don't file it as a mismatch. (An INVALID component is already the invalid-wasm oracle's job.)
        Err(e) => Side::Declined(format!("wasm run failed: {e}")),
    }
}

/// Run one program through the RUST backend by shelling `cdz run-rust` (source on stdin → one verdict
/// line). Maps that verdict to a [`Side`]:
///
/// * `value <sexpr>` → [`Side::Value`]   (same render as `cdz-run`, so byte-comparable to the wasm value)
/// * `trap <msg>`    → [`Side::Trap`]
/// * `declined`      → [`Side::Declined`] (front reject / rust-not-yet — coverage-not-yet)
/// * `error <msg>`   → [`Side::Trap`]   (a bad artifact: emitted `.rs` failed rustc. This IS a
///   miscompile, but the crash oracle owns "the compiler produced garbage"; here we surface it as a
///   non-value outcome so a wasm `value` vs rust `error` reads as a liveness mismatch and gets filed.)
///
/// `cdz` is the path to the `cdz` binary (its dir must also hold the `libcdz_rt`/`libcdz_num` rlibs
/// `cdz run-rust` links). Exit is 0 for any verdict; a NON-zero exit is a harness read-failure that
/// produced no verdict → [`Diff::Unavailable`] via the returned `Err`.
pub fn run_rust(cdz: &std::path::Path, source: &str) -> Result<Side, String> {
    use std::io::Write;

    let mut child = Command::new(cdz)
        .arg("run-rust")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn `{} run-rust` failed: {e}", cdz.display()))?;
    if let Some(mut stdin) = child.stdin.take()
        && let Err(e) = stdin.write_all(source.as_bytes())
    {
        return Err(format!(
            "writing program to `cdz run-rust` stdin failed: {e}"
        ));
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("waiting on `cdz run-rust` failed: {e}"))?;
    if !out.status.success() {
        // The one reserved non-verdict path: a harness read-failure. Not a compiler outcome.
        return Err(format!(
            "`cdz run-rust` exited non-zero (harness failure): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // The verdict is the last non-empty stdout line (contract: one line; be robust to a trailing
    // newline / an incidental leading line).
    let stdout = String::from_utf8_lossy(&out.stdout);
    let verdict = stdout
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    Ok(parse_rust_verdict(verdict))
}

/// Map a `cdz run-rust` verdict line to a [`Side`]. Split out so the grammar is unit-testable without
/// spawning the binary. See [`run_rust`] for the `error`→`Trap` rationale.
pub fn parse_rust_verdict(verdict: &str) -> Side {
    if verdict == "declined" {
        Side::Declined("rust-decline".to_string())
    } else if let Some(v) = verdict.strip_prefix("value ") {
        Side::Value(v.trim().to_string())
    } else if let Some(t) = verdict.strip_prefix("trap ") {
        Side::Trap(t.trim().to_string())
    } else if let Some(e) = verdict.strip_prefix("error ") {
        // A non-compiling emitted artifact. Surface as a non-value outcome so wasm-value vs
        // rust-error reads as a liveness mismatch (a filable miscompile).
        Side::Trap(format!("rust-artifact-error: {}", e.trim()))
    } else {
        // An unrecognized line — treat conservatively as declined (not comparable), never a mismatch.
        Side::Declined(format!("unrecognized run-rust verdict: {verdict}"))
    }
}

/// The full differential check for one program: run both backends and compare. `store` is the runtime
/// store for the wasm run; `cdz` is the `cdz` binary for the rust run. A harness failure driving the
/// rust side becomes [`Diff::Unavailable`] (logged, never filed).
pub fn differential(source: &str, store: &std::path::Path, cdz: &std::path::Path) -> Diff {
    let wasm = run_wasm(source, store);
    // Cheap short-circuit: a wasm decline is never comparable, so skip the (expensive) rustc run.
    if let Side::Declined(_) = wasm {
        return Diff::Agree;
    }
    let rust = match run_rust(cdz, source) {
        Ok(s) => s,
        Err(e) => return Diff::Unavailable(e),
    };
    compare(&wasm, &rust)
}

/// Best-effort discovery of the `cdz` binary for the rust side: honor `CDZ_SMITH_CDZ`, else look for
/// `cdz` beside a workspace `target/{release,debug}/`. Returns `None` if none is found (the caller
/// then reports the differential oracle as unavailable this run rather than filing spurious findings).
pub fn discover_cdz() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CDZ_SMITH_CDZ") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    // cdz-smith lives at <repo>/implementation/seed/crates/cdz-smith; the unified `cdz` binary +
    // its rlibs land in <repo>/target/{release,debug}/ (the workspace target, NOT the seed one).
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.ancestors().nth(4)?;
    for profile in ["release", "debug"] {
        let cand = repo.join(format!("target/{profile}/cdz"));
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── pairing rules (pure `compare`) ───────────────────────────────────────────────────────

    #[test]
    fn identical_values_agree() {
        assert_eq!(
            compare(&Side::Value("3".into()), &Side::Value("3".into())),
            Diff::Agree
        );
    }

    #[test]
    fn differing_values_are_a_value_mismatch() {
        let d = compare(&Side::Value("3".into()), &Side::Value("4".into()));
        match d {
            Diff::Mismatch {
                kind: MismatchKind::Value,
                wasm,
                rust,
            } => {
                assert_eq!(wasm, "3");
                assert_eq!(rust, "4");
            }
            other => panic!("expected a value mismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_decline_on_either_side_never_mismatches() {
        // Rust declines (subset) while wasm produced a value — coverage-not-yet, NOT a bug.
        assert_eq!(
            compare(
                &Side::Value("3".into()),
                &Side::Declined("rust-decline".into())
            ),
            Diff::Agree
        );
        assert_eq!(
            compare(&Side::Declined("wasm".into()), &Side::Value("3".into())),
            Diff::Agree
        );
        // Even a value-vs-value that would differ is suppressed if one side declined.
        assert_eq!(
            compare(&Side::Declined("x".into()), &Side::Trap("boom".into())),
            Diff::Agree
        );
    }

    #[test]
    fn both_trap_agree_regardless_of_message() {
        assert_eq!(
            compare(
                &Side::Trap("integer divide by zero".into()),
                &Side::Trap("attempt to divide by zero".into())
            ),
            Diff::Agree
        );
    }

    #[test]
    fn value_vs_trap_is_a_liveness_mismatch() {
        let d = compare(&Side::Value("7".into()), &Side::Trap("overflow".into()));
        assert!(
            matches!(
                d,
                Diff::Mismatch {
                    kind: MismatchKind::Liveness,
                    ..
                }
            ),
            "got {d:?}"
        );
        let d2 = compare(&Side::Trap("overflow".into()), &Side::Value("7".into()));
        assert!(
            matches!(
                d2,
                Diff::Mismatch {
                    kind: MismatchKind::Liveness,
                    ..
                }
            ),
            "got {d2:?}"
        );
    }

    // ── verdict parsing ──────────────────────────────────────────────────────────────────────

    #[test]
    fn parse_verdict_covers_the_grammar() {
        assert_eq!(parse_rust_verdict("value 42"), Side::Value("42".into()));
        assert_eq!(
            parse_rust_verdict("value (tuple 1 2)"),
            Side::Value("(tuple 1 2)".into())
        );
        assert!(matches!(parse_rust_verdict("declined"), Side::Declined(_)));
        assert_eq!(
            parse_rust_verdict("trap integer overflow"),
            Side::Trap("integer overflow".into())
        );
        // `error` surfaces as a Trap so wasm-value-vs-rust-error is a filable liveness mismatch.
        match parse_rust_verdict("error E0308 mismatched types") {
            Side::Trap(m) => assert!(m.contains("rust-artifact-error")),
            other => panic!("expected Trap, got {other:?}"),
        }
        // An unrecognized line is conservatively a decline (never a spurious mismatch).
        assert!(matches!(parse_rust_verdict("weird"), Side::Declined(_)));
    }

    // ── end-to-end: the two backends agree on a trivial arithmetic program ───────────────────

    /// A real, in-process differential on a scalar program: `cdz-run` (wasm) and `cdz run-rust` must
    /// agree on `1 + 2 = 3`. Skips (does not fail) when the `cdz` binary or runtime store is absent —
    /// the unit gate runs in environments without a built `cdz`; the fuzz-cycle runs it for real.
    #[test]
    fn a_scalar_program_agrees_across_backends() {
        let Some(cdz) = discover_cdz() else {
            eprintln!("skipping: no `cdz` binary discovered (set CDZ_SMITH_CDZ)");
            return;
        };
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo = manifest.ancestors().nth(4).unwrap();
        let store = repo.join("target/cadenza-store");

        let program = "(do (def (main) (+ 1 2)) (export main))";
        let wasm = run_wasm(program, &store);
        // A pure scalar imports no runtime, so this side must work even without a store.
        assert_eq!(wasm, Side::Value("3".into()), "wasm side");

        match run_rust(&cdz, program) {
            Ok(rust) => {
                assert_eq!(rust, Side::Value("3".into()), "rust side");
                assert_eq!(compare(&wasm, &rust), Diff::Agree);
            }
            Err(e) => eprintln!("skipping rust side: {e}"),
        }
    }
}
