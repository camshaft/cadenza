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
//! * An **`ArtifactError`** (the Rust side emitted un-compilable source — `cdz run-rust` → `error …`)
//!   is a build-blocking miscompile that is ALWAYS a finding, surfaced regardless of the other side —
//!   even against a wasm trap (a trap-vs-trap would otherwise agree and hide it).
//!
//! A crash on EITHER backend is out of scope here — [`crate::oracle::compile_catching`] already mines
//! both backends for panics. This oracle assumes a non-crashing compile and asks only "do the two
//! agree on the VALUE?".

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// One backend's outcome for a program, reduced to the cases the pairing rules compare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Side {
    /// Ran to a value, rendered to canonical text (bare scalar / `(tuple …)` / …).
    Value(String),
    /// Trapped at run time (message kept for the note; not used in the comparison).
    Trap(String),
    /// The front-end rejected the program, or this backend does not emit it yet — NOT comparable,
    /// treated as coverage-not-yet. `detail` is a short reason for the triage note.
    Declined(String),
    /// The backend emitted an artifact that FAILED TO BUILD (`cdz run-rust` → `error …`: the emitted
    /// `.rs` did not compile under rustc). This is a genuine backend MISCOMPILE — the compiler
    /// reported success at the emit seam but produced un-compilable source. Unlike a `Trap`, this is
    /// ALWAYS surfaced (never swallowed by a trap-vs-trap agreement) — see [`compare`]. Only the Rust
    /// side can produce it; the wasm side's structurally-invalid output is the invalid-wasm oracle's job.
    ArtifactError(String),
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
    /// A backend emitted un-compilable source (`ArtifactError`) — a build-blocking miscompile,
    /// surfaced regardless of the other side's outcome (even if the other also trapped).
    Artifact,
}

impl MismatchKind {
    pub fn tag(self) -> &'static str {
        match self {
            MismatchKind::Value => "value",
            MismatchKind::Liveness => "liveness",
            MismatchKind::Artifact => "artifact",
        }
    }
}

/// Compare the wasm and rust outcomes for one program per the pairing rules. Pure — the two sides are
/// produced by [`run_wasm`] / [`run_rust`]; splitting it out keeps the rules unit-testable without a
/// compiler or a subprocess.
pub fn compare(wasm: &Side, rust: &Side) -> Diff {
    match (wasm, rust) {
        // An ArtifactError (un-compilable emitted source) is a build-blocking MISCOMPILE that must be
        // surfaced NO MATTER what the other side did — even a trap-vs-artifact-error, which the
        // Trap-vs-Trap agreement arm below would otherwise swallow (PR#552 soundness). Checked FIRST,
        // BEFORE the decline arm, because a genuine artifact miscompile must not be masked by the other
        // side happening to decline either. (The wasm side never yields ArtifactError — see `Side`.)
        (Side::ArtifactError(e), other) => Diff::Mismatch {
            kind: MismatchKind::Artifact,
            wasm: format!("wasm {}", describe_side(other)),
            rust: format!("artifact-error {e}"),
        },
        (other, Side::ArtifactError(e)) => Diff::Mismatch {
            kind: MismatchKind::Artifact,
            wasm: format!("wasm {}", describe_side(other)),
            rust: format!("artifact-error {e}"),
        },
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

/// A short label for a [`Side`] in an artifact-error mismatch note (the OTHER side, whatever it was).
fn describe_side(s: &Side) -> String {
    match s {
        Side::Value(v) => format!("value {v}"),
        Side::Trap(t) => format!("trap {t}"),
        Side::Declined(d) => format!("declined {d}"),
        Side::ArtifactError(e) => format!("artifact-error {e}"),
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
    run_wasm_bytes(&bytes, store)
}

/// Run a BINARY-AST blob through the WASM backend — the next-gen entropy path's analog of [`run_wasm`].
/// DECODE-GATE first (strict + total `codec::decode_detailed`: malformed / truncated / non-tree bytes
/// → [`Side::Declined`], never a false mismatch or a panic), re-encode canonical, then compile + run
/// exactly as [`run_wasm`]. This is how a binary-AST-entropy program's rcdzc OUTPUT (value / trap) is
/// captured to run the wasm backend — and, in the L2 differential, as the rcdzc-output side of a Lean
/// trial. A blob that does not decode is a malformed entropy input (not comparable), not a bug.
pub fn run_wasm_ast(ast_bytes: &[u8], store: &std::path::Path) -> Side {
    let arenas = match cadenza_syntax::codec::decode_detailed(ast_bytes) {
        Ok(a) => a,
        Err(e) => return Side::Declined(format!("decode: {e:?}")),
    };
    let bytes = cadenza_syntax::codec::encode(&arenas);
    run_wasm_bytes(&bytes, store)
}

/// Compile already-encoded binary-AST `bytes` to a component and run it in-process — the shared tail
/// of [`run_wasm`] (text path) and [`run_wasm_ast`] (binary-AST-entropy path).
fn run_wasm_bytes(bytes: &[u8], store: &std::path::Path) -> Side {
    // Compile to a component. A rejection/decline (errors-as-data) → not comparable.
    let component = match rcdzc::compile_component(bytes) {
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
        // NORMALIZE to the bare value. `cdz-run` renders a COMPOUND (and, depending on the ABI, a
        // scalar) result as the full `(: <value> <Type>)` value-form, while `cdz run-rust` renders the
        // bare `<value>`. Comparing the two raw would flag every string/tuple as a false "mismatch"
        // (the values agree; only the type annotation differs). Strip the `(: … <Type>)` wrapper so both
        // sides are the same canonical bare form — exactly the accept-either-form rule the corpus gate
        // uses (`expected_value`).
        Ok(cdz_run::Outcome::Value(v)) => Side::Value(strip_value_annotation(v.trim())),
        Ok(cdz_run::Outcome::Trap(t)) => Side::Trap(t),
        // A run harness error (invalid component, unresolvable import) — not a value disagreement;
        // don't file it as a mismatch. (An INVALID component is already the invalid-wasm oracle's job.)
        Err(e) => Side::Declined(format!("wasm run failed: {e}")),
    }
}

/// Strip the `(: <value> <Type>)` value-form wrapper down to the bare `<value>`, matching what
/// `cdz run-rust` (and a scalar `cdz-run` result) prints. A payload that is NOT a value-form is
/// returned unchanged. Mirrors the corpus gate's `expected_value`: take the FIRST balanced token after
/// `(:` — a `(…)` group, a `"…"` string (which may contain spaces), or a bare atom up to the next space.
fn strip_value_annotation(payload: &str) -> String {
    let Some(rest) = payload.strip_prefix("(:") else {
        return payload.to_string();
    };
    let rest = rest.trim();
    let bytes = rest.as_bytes();
    match bytes.first() {
        // A parenthesized value — take the balanced `(…)` group.
        Some(b'(') => {
            let mut depth = 0i32;
            for (i, &b) in bytes.iter().enumerate() {
                match b {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            return rest[..=i].to_string();
                        }
                    }
                    _ => {}
                }
            }
            rest.to_string()
        }
        // A quoted string value (may contain internal spaces) — take up to the matching close quote,
        // honoring a `\"` escape so an embedded quote does not end the token early.
        Some(b'"') => {
            let mut escaped = false;
            for (i, &b) in bytes.iter().enumerate().skip(1) {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    return rest[..=i].to_string();
                }
            }
            rest.to_string()
        }
        // A bare atom — up to the next space (dropping the trailing `)` when there is no space).
        _ => match rest.find(char::is_whitespace) {
            Some(idx) => rest[..idx].to_string(),
            None => rest.trim_end_matches(')').to_string(),
        },
    }
}

/// Run one program through the RUST backend by shelling `cdz run-rust` (source on stdin → one verdict
/// line). Maps that verdict to a [`Side`]:
///
/// * `value <sexpr>` → [`Side::Value`]   (same render as `cdz-run`, so byte-comparable to the wasm value)
/// * `trap <msg>`    → [`Side::Trap`]
/// * `declined`      → [`Side::Declined`] (front reject / rust-not-yet — coverage-not-yet)
/// * `error <msg>`   → [`Side::ArtifactError`] (emitted `.rs` failed rustc — a build-blocking
///   miscompile that `compare` ALWAYS surfaces, even against a wasm trap — see [`Side::ArtifactError`]).
///
/// `cdz` is the path to the `cdz` binary (its dir must also hold the `libcdz_rt`/`libcdz_num` rlibs
/// `cdz run-rust` links).
///
/// Exit contract (per `cdz run-rust`, PR#547): exit 0 with a verdict LINE on stdout for a run outcome;
/// exit NON-ZERO (no verdict line, message on stderr) for a HARNESS/USAGE error — a file/stdin read
/// failure OR a usage error (e.g. a program with multiple exports and no `--call`). A non-zero exit is
/// therefore NOT a comparable run: it maps to [`Side::Declined`] (a non-comparable side — the oracle
/// stays SOUND and simply skips this program), NOT to a `Diff::Unavailable`. `Unavailable` (the `Err`
/// return) is reserved for a genuine INFRASTRUCTURE failure where the oracle itself could not run —
/// we couldn't even spawn the binary, write its stdin, or reap it. That distinction matters: a usage
/// error is per-program (skip it), an infrastructure failure means the whole sweep is misconfigured.
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
        // Non-zero exit = a harness/usage error for THIS program (no verdict line), not an
        // infrastructure failure. Classify it as a non-comparable Declined side so the oracle stays
        // sound (never mismatches on it) and simply skips the program — do NOT disable the oracle
        // (`Unavailable`) for what is a per-program condition.
        return Ok(Side::Declined(format!(
            "run-rust usage/harness error: {}",
            first_line(&String::from_utf8_lossy(&out.stderr))
        )));
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

/// The first non-empty line of `s`, trimmed (for a concise `Declined` reason from multi-line stderr).
fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
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
        // A non-compiling emitted artifact (rustc rejected the emitted `.rs`) — a build-blocking
        // MISCOMPILE. Its own `Side::ArtifactError` so `compare` ALWAYS surfaces it, even against a
        // wasm trap (a `Side::Trap` here would be swallowed by the trap-vs-trap agreement — PR#552).
        Side::ArtifactError(e.trim().to_string())
    } else {
        // An unrecognized line — treat conservatively as declined (not comparable), never a mismatch.
        Side::Declined(format!("unrecognized run-rust verdict: {verdict}"))
    }
}

/// Outcome tally of running an AST seed corpus through the wasm backend (see [`run_ast_corpus_sweep`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AstSweepStats {
    /// `.ast` seeds run.
    pub seeds: usize,
    /// Seeds that produced a value.
    pub values: usize,
    /// Seeds that trapped at run time.
    pub traps: usize,
    /// Seeds the front-end/backend declined, or that didn't decode / lacked a runtime in the store.
    pub declined: usize,
}

/// Run every `*.ast` seed in `seeds_dir` through the WASM backend ([`run_wasm_ast`]), tallying
/// value / trap / declined outcomes. This is the operator's "run the wasm backend on the
/// semantics-corpus AST seeds" end-to-end: S1 decode-gate → re-encode → compile → S3 wasm run, over
/// the S2 seed corpus. It never files anything — it's a throughput/health probe (and the substrate the
/// L2 Lean differential will pipeline over). Seeds are visited in sorted order for reproducibility.
pub fn run_ast_corpus_sweep(
    seeds_dir: &std::path::Path,
    store: &std::path::Path,
) -> std::io::Result<AstSweepStats> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(seeds_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ast"))
        .collect();
    paths.sort();

    let mut stats = AstSweepStats::default();
    for path in &paths {
        let bytes = std::fs::read(path)?;
        stats.seeds += 1;
        match run_wasm_ast(&bytes, store) {
            Side::Value(_) => stats.values += 1,
            Side::Trap(_) => stats.traps += 1,
            Side::Declined(_) | Side::ArtifactError(_) => stats.declined += 1,
        }
    }
    Ok(stats)
}

// ── the Lean L2 differential (S4b) ────────────────────────────────────────────────────────────────
//
// The async-batched differential the operator asked for: run programs under the WASM backend, capture
// rcdzc's output, hand the oracle a BATCH of trials `(batch (trial <program> (args) <output>) …)`, and
// judge each (holds / mismatch / skip). A `mismatch` = the Lean oracle's re-derived value disagrees with
// rcdzc's — a candidate miscompile the wasm-validity + crash oracles are blind to. Lean is a THIRD
// differential Side (an independent implementation of the semantics), so this catches wrong-value
// miscompiles just like the wasm-vs-rust oracle, but against a formally-modelled reference.

/// Bridge one wasm [`Side`] into the rcdzc-output a Lean trial carries. `Value` renders → `(value <ast>)`
/// (via [`crate::lean::RcdzcOutput::value_from_render`]); `Trap` → `(trap <kind>)`. A `Declined` /
/// `ArtifactError` (or a value whose render doesn't parse) is NOT comparable → `None` (the trial is skipped).
fn side_to_rcdzc_output(side: Side) -> Option<crate::lean::RcdzcOutput> {
    match side {
        Side::Value(v) => crate::lean::RcdzcOutput::value_from_render(&v),
        Side::Trap(t) => Some(crate::lean::RcdzcOutput::Trap(t)),
        Side::Declined(_) | Side::ArtifactError(_) => None,
    }
}

/// Outcome tally of a Lean differential sweep.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LeanDiffStats {
    /// Trials the oracle judged (comparable programs).
    pub trials: usize,
    /// The oracle's value/trap matched rcdzc's — no bug.
    pub holds: usize,
    /// The oracle disagreed with rcdzc — a candidate miscompile (collected in `mismatches`).
    pub mismatches: usize,
    /// The oracle skipped (a construct it does not model yet) — a coverage gap, not a bug.
    pub skips: usize,
    /// Programs that produced no comparable wasm output (declined / artifact-error / unparsable render).
    pub not_comparable: usize,
}

/// Run each program source under the WASM backend, batch the comparable (program, rcdzc-output) trials,
/// and judge each batch with `oracle-check --batch-stream`. Tallies holds/mismatch/skip; every
/// `Mismatch` pushes `(source, oracle-detail)` into `mismatches` (a candidate rcdzc bug). Batching is
/// the async unit the operator's pipeline overlaps (a fresh `oracle-check` per batch judges while the
/// next batch compiles). `sources` should be TERMINATING programs (e.g. `generator::generate`'s
/// structurally-terminating grammar) — the in-process wasm run has no hang guard.
pub fn lean_differential_sweep(
    sources: &[String],
    store: &std::path::Path,
    oracle_bin: &std::path::Path,
    batch_size: usize,
    mismatches: &mut Vec<(String, String)>,
) -> std::io::Result<LeanDiffStats> {
    let mut stats = LeanDiffStats::default();
    let mut batch_srcs: Vec<String> = Vec::new();
    let mut batch_trials: Vec<crate::lean::Trial> = Vec::new();
    let batch_size = batch_size.max(1);

    for src in sources {
        let output = match side_to_rcdzc_output(run_wasm(src, store)) {
            Some(o) => o,
            None => {
                stats.not_comparable += 1;
                continue;
            }
        };
        // The trial carries the FULL program (the oracle re-roots + executes it with empty args).
        let Ok(program) = cadenza_syntax::sexpr::read(src) else {
            stats.not_comparable += 1;
            continue;
        };
        batch_srcs.push(src.clone());
        batch_trials.push(crate::lean::Trial::main_0(program, output));
        if batch_trials.len() >= batch_size {
            judge_and_tally(
                oracle_bin,
                &batch_srcs,
                &batch_trials,
                &mut stats,
                mismatches,
            )?;
            batch_srcs.clear();
            batch_trials.clear();
        }
    }
    if !batch_trials.is_empty() {
        judge_and_tally(
            oracle_bin,
            &batch_srcs,
            &batch_trials,
            &mut stats,
            mismatches,
        )?;
    }
    Ok(stats)
}

/// Judge one batch of trials and fold the verdicts into `stats` (+ collect mismatches by source).
fn judge_and_tally(
    oracle_bin: &std::path::Path,
    srcs: &[String],
    trials: &[crate::lean::Trial],
    stats: &mut LeanDiffStats,
    mismatches: &mut Vec<(String, String)>,
) -> std::io::Result<()> {
    let verdicts = crate::lean::judge_batch(oracle_bin, trials)?;
    for (src, verdict) in srcs.iter().zip(verdicts) {
        stats.trials += 1;
        match verdict {
            crate::lean::Verdict::Holds => stats.holds += 1,
            crate::lean::Verdict::Skip(_) => stats.skips += 1,
            crate::lean::Verdict::Mismatch(detail) => {
                stats.mismatches += 1;
                mismatches.push((src.clone(), detail));
            }
        }
    }
    Ok(())
}

/// The full differential check for one program: run both backends and compare. `store` is the runtime
/// store for the wasm run; `cdz` is the `cdz` binary for the rust run. A non-zero `run-rust` exit
/// (per-program usage/harness error) is a non-comparable [`Side::Declined`] (→ `Diff::Agree`, skipped);
/// only an INFRASTRUCTURE failure that prevented the run entirely (spawn/write/reap) becomes
/// [`Diff::Unavailable`] (logged, never filed) — see [`run_rust`].
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

/// Greedily minimize a program that triggers a differential MISMATCH, preserving that the shrunk
/// program STILL mismatches (of the SAME [`MismatchKind`]). Mirrors `finding::shrink*` but its
/// predicate re-runs the full two-backend `differential` (each accepted step re-derives spans on the
/// smaller program). Bounded passes so a pathological input can't loop; each accepted deletion
/// strictly shrinks the source. A `Diff::Unavailable` mid-shrink stops accepting (we keep the best so
/// far) — we never trade a confirmed mismatch for an un-rerunnable candidate.
pub fn shrink_differential(
    source: &str,
    kind: MismatchKind,
    store: &std::path::Path,
    cdz: &std::path::Path,
) -> String {
    let mut best = source.to_string();
    for _ in 0..12 {
        let mut improved = false;
        let spans = crate::finding::balanced_spans(&best);
        for (lo, hi) in spans.into_iter().rev() {
            if lo == 0 && hi == best.len() {
                continue; // never delete the whole program
            }
            let mut candidate = String::with_capacity(best.len() - (hi - lo));
            candidate.push_str(&best[..lo]);
            candidate.push_str(&best[hi..]);
            let candidate = candidate.trim().to_string();
            if candidate.len() >= best.len() {
                continue;
            }
            // Keep the deletion only if it still mismatches the SAME way.
            if let Diff::Mismatch { kind: k, .. } = differential(&candidate, store, cdz)
                && k == kind
            {
                best = candidate;
                improved = true;
                break; // re-derive spans on the smaller program
            }
        }
        if !improved {
            break;
        }
    }
    best
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
        // `error` is its OWN ArtifactError side (not a Trap) so it is never swallowed by trap-vs-trap.
        match parse_rust_verdict("error E0308 mismatched types") {
            Side::ArtifactError(m) => assert!(m.contains("E0308")),
            other => panic!("expected ArtifactError, got {other:?}"),
        }
        // An unrecognized line is conservatively a decline (never a spurious mismatch).
        assert!(matches!(parse_rust_verdict("weird"), Side::Declined(_)));
    }

    #[test]
    fn an_artifact_error_is_a_mismatch_even_against_a_trap() {
        // The PR#552 soundness gap: a build-blocking rust miscompile (ArtifactError) must be surfaced
        // even when the wasm side ALSO traps — a Side::Trap here would agree and hide it.
        let d = compare(
            &Side::Trap("integer overflow".into()),
            &Side::ArtifactError("E0308".into()),
        );
        assert!(
            matches!(
                d,
                Diff::Mismatch {
                    kind: MismatchKind::Artifact,
                    ..
                }
            ),
            "artifact error vs trap must be an Artifact mismatch, got {d:?}"
        );
        // …and even against a wasm value, and in either position.
        assert!(matches!(
            compare(&Side::ArtifactError("x".into()), &Side::Value("3".into())),
            Diff::Mismatch {
                kind: MismatchKind::Artifact,
                ..
            }
        ));
        // But an artifact error vs a DECLINE is still surfaced (the miscompile is real regardless).
        assert!(matches!(
            compare(
                &Side::Declined("x".into()),
                &Side::ArtifactError("y".into())
            ),
            Diff::Mismatch {
                kind: MismatchKind::Artifact,
                ..
            }
        ));
    }

    // ── value-annotation stripping (the false-positive fix) ──────────────────────────────────

    #[test]
    fn strip_value_annotation_matches_the_bare_rust_render() {
        // A bare scalar has no wrapper — unchanged.
        assert_eq!(strip_value_annotation("3"), "3");
        // A string value-form → the bare quoted string (the exact false positive that motivated this).
        assert_eq!(strip_value_annotation("(: \"ayg\" String)"), "\"ayg\"");
        // A string with INTERNAL SPACES must not be cut at the first space.
        assert_eq!(
            strip_value_annotation("(: \"hello world\" String)"),
            "\"hello world\""
        );
        // A compound (tuple) value-form → the bare `(tuple …)` group, not cut at its inner space.
        assert_eq!(
            strip_value_annotation("(: (tuple 1 \"x\") (Tuple Int64 String))"),
            "(tuple 1 \"x\")"
        );
        // A bare-atom value-form (`(: 42 Int64)`) → `42`.
        assert_eq!(strip_value_annotation("(: 42 Int64)"), "42");
        // A non-value-form payload is returned unchanged.
        assert_eq!(strip_value_annotation("(tuple 1 2)"), "(tuple 1 2)");
    }

    /// A non-zero `run-rust` exit (a usage/harness error for one program) must classify as a
    /// non-comparable `Side::Declined`, NOT bubble as an `Err` (→ `Diff::Unavailable`). Driving a
    /// program `cdz run-rust` rejects at the usage layer would need a multi-export program; instead we
    /// point `run_rust` at a stand-in binary that always exits non-zero (`false`) and assert the
    /// contract: a non-zero exit → `Ok(Declined)`, so the oracle stays sound and skips rather than
    /// disabling itself. (Soundness fix, PR#551 #3.)
    #[test]
    fn a_nonzero_run_rust_exit_is_declined_not_unavailable() {
        let false_bin = std::path::Path::new("/bin/false");
        if !false_bin.exists() {
            eprintln!("skipping: no /bin/false");
            return;
        }
        match run_rust(false_bin, "(do (def (main) 1) (export main))") {
            Ok(Side::Declined(_)) => {} // the sound outcome
            other => panic!("a non-zero exit must be Ok(Declined), got {other:?}"),
        }
    }

    /// The end-to-end case that a scalar-only test missed: a STRING result. Both backends must AGREE
    /// after normalization (`cdz-run` prints `(: "ayg" String)`, `cdz run-rust` prints `"ayg"`).
    #[test]
    fn a_string_program_agrees_across_backends_after_normalization() {
        let Some(cdz) = discover_cdz() else {
            eprintln!("skipping: no `cdz` binary discovered (set CDZ_SMITH_CDZ)");
            return;
        };
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo = manifest.ancestors().nth(4).unwrap();
        let store = repo.join("target/cadenza-store");

        let program = r#"(do (def (main) "ayg") (export main))"#;
        let wasm = run_wasm(program, &store);
        assert_eq!(
            wasm,
            Side::Value("\"ayg\"".into()),
            "wasm side (normalized)"
        );
        match run_rust(&cdz, program) {
            Ok(rust) => {
                assert_eq!(rust, Side::Value("\"ayg\"".into()), "rust side");
                assert_eq!(compare(&wasm, &rust), Diff::Agree);
            }
            Err(e) => eprintln!("skipping rust side: {e}"),
        }
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

    // ── the binary-AST-entropy wasm-run path (`run_wasm_ast`) ─────────────────────────────────────

    /// Encode a source program to canonical binary-AST bytes — the shape the entropy path consumes.
    fn ast_bytes_of(source: &str) -> Vec<u8> {
        let arenas = cadenza_syntax::sexpr::read(source).expect("test source parses");
        cadenza_syntax::codec::encode(&arenas)
    }

    /// Run the WASM backend directly from a BINARY-AST blob: a pure scalar imports no runtime, so it
    /// runs to its value with NO store — proving the decode-gate → re-encode → compile → run path is
    /// equivalent to the text `run_wasm` for a real program. This is the operator's "run the wasm
    /// backend" on binary-AST entropy.
    #[test]
    fn run_wasm_ast_runs_a_scalar_blob_without_a_store() {
        let bytes = ast_bytes_of("(do (def (main) (+ 1 2)) (export main))");
        let side = run_wasm_ast(&bytes, std::path::Path::new("/nonexistent-store"));
        assert_eq!(side, Side::Value("3".into()));
    }

    /// A malformed entropy blob is DECLINED by the decode-gate (not a panic, not a false mismatch) —
    /// the strict + total codec keeps the differential sound on arbitrary/mutated bytes.
    #[test]
    fn run_wasm_ast_declines_garbage_bytes() {
        let side = run_wasm_ast(
            b"not a binary ast",
            std::path::Path::new("/nonexistent-store"),
        );
        assert!(matches!(side, Side::Declined(_)), "got {side:?}");
    }

    /// The corpus sweep runs every `*.ast` seed and tallies outcomes. Two pure-scalar seeds → two
    /// values, no store needed — exercising S2-seed → S3-wasm-run end to end. A garbage `.ast`
    /// declines (decode-gate) without derailing the sweep.
    #[test]
    fn run_ast_corpus_sweep_tallies_scalar_seeds() {
        let dir = std::env::temp_dir().join(format!("cdz-smith-sweep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.ast"),
            ast_bytes_of("(do (def (main) (+ 1 2)) (export main))"),
        )
        .unwrap();
        std::fs::write(
            dir.join("b.ast"),
            ast_bytes_of("(do (def (main) 42) (export main))"),
        )
        .unwrap();
        std::fs::write(dir.join("bad.ast"), b"not a binary ast").unwrap();

        let stats = run_ast_corpus_sweep(&dir, std::path::Path::new("/nonexistent-store")).unwrap();
        assert_eq!(stats.seeds, 3);
        assert_eq!(stats.values, 2);
        assert_eq!(stats.declined, 1);
        assert_eq!(stats.traps, 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── the Lean L2 differential (`lean_differential_sweep`) ──────────────────────────────────────

    /// The wasm-Side → Lean-trial-output bridge: a value render → `(value …)`, a trap → `(trap …)`, a
    /// decline / artifact-error / unparsable render → not comparable (`None`). Pure — no wasm / oracle.
    #[test]
    fn side_to_rcdzc_output_bridges_each_side() {
        use crate::lean::RcdzcOutput;
        assert!(matches!(
            side_to_rcdzc_output(Side::Value("42".into())),
            Some(RcdzcOutput::Value(_))
        ));
        assert!(matches!(
            side_to_rcdzc_output(Side::Trap("div-by-zero".into())),
            Some(RcdzcOutput::Trap(_))
        ));
        assert!(side_to_rcdzc_output(Side::Declined("x".into())).is_none());
        assert!(side_to_rcdzc_output(Side::ArtifactError("E0308".into())).is_none());
        // A value whose render doesn't parse as an AST is not comparable.
        assert!(side_to_rcdzc_output(Side::Value("(( unbalanced".into())).is_none());
    }

    /// END-TO-END Lean differential against the REAL `oracle-check` (skips unless `CDZ_SMITH_ORACLE_CHECK`
    /// points at an AST-envelope oracle — `nix build .#oracle-lean`). Two benign scalar programs (which
    /// import no runtime, so no store is needed) must HOLD against the oracle, with no mismatches.
    #[test]
    fn lean_differential_sweep_holds_for_benign_scalars() {
        let Some(oracle) = crate::lean::discover_oracle_check() else {
            eprintln!(
                "skipping: no oracle-check (nix build .#oracle-lean; set CDZ_SMITH_ORACLE_CHECK)"
            );
            return;
        };
        let sources = vec![
            "(do (def (main) (+ 1 2)) (export main))".to_string(),
            "(do (def (main) 42) (export main))".to_string(),
        ];
        let store = std::path::Path::new("/nonexistent-store"); // pure scalars need no runtime
        let mut mismatches = Vec::new();
        let stats = lean_differential_sweep(&sources, store, &oracle, 8, &mut mismatches)
            .expect("sweep runs");
        assert_eq!(stats.trials, 2, "both scalars are comparable");
        assert_eq!(
            stats.mismatches, 0,
            "benign scalars must not mismatch: {mismatches:?}"
        );
        assert!(mismatches.is_empty());
    }
}
