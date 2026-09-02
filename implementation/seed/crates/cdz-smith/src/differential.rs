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
//!   backend computes a result where the other faults — EXCEPT a stack-exhaustion / resource trap
//!   ([`is_resource_trap`]), which is a tolerated RESOURCE divergence (the backends have different
//!   native stack limits, so deep non-tail recursion returns on one and traps gracefully on the
//!   other), not a semantic liveness bug.
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
    /// The rust backend's build ENVIRONMENT is broken for THIS run — the emitted `.rs` is fine but the
    /// staging externs (`cdz_num`/`cdz_rt` rlibs) are not on the rustc link path, so EVERY program reds
    /// with the same `E0433: cannot find crate/module cdz_num`. That is an env/harness failure, NOT a
    /// compiler outcome, so it must NOT file a differential finding — it maps to [`Diff::Unavailable`] so
    /// a campaign with a broken link env fails LOUD (counted "unavailable") rather than silently agreeing
    /// (hiding the broken env) or filing false `Artifact` buckets (the S79/breaker false-positive class).
    Unavailable(String),
    /// The in-process compile PANICKED — a compiler crash (e.g. a nullary `(Set.of)` index-OOB in
    /// `infer/node.rs`). Caught by [`crate::oracle::compile_component_catching`] so it is FILED as a crash
    /// finding + the sweep continues, instead of the unguarded native panic aborting the whole run. Only
    /// the in-process wasm side produces it (the rust side compiles in a subprocess).
    CompilePanic(crate::oracle::CrashInfo),
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
    /// The in-process compile PANICKED — a compiler CRASH (filed as [`crate::finding::Category::Crash`],
    /// like the crash oracle, then the sweep continues). Distinct from a value/liveness `Mismatch`: no
    /// value was produced, the compiler itself faulted.
    CompileCrash(crate::oracle::CrashInfo),
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

/// A stack-exhaustion / resource trap (as opposed to a semantic trap). The two backends have different
/// native call-stack limits — wasm traps `call stack exhausted` at ~15k non-tail frames, rust panics
/// `has overflowed its stack` ~10x deeper — so on deep non-tail recursion one may return a value while
/// the other traps GRACEFULLY at its own limit. That value-vs-trap split is a tolerated RESOURCE
/// divergence, not a liveness miscompile. Matched narrowly on the stack-exhaustion phrasings so a
/// SEMANTIC trap (`divide by zero`, arithmetic `overflow`, `unreachable`) is NOT swallowed.
fn is_resource_trap(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("call stack exhausted")
        || m.contains("stack overflow")
        || m.contains("overflowed its stack")
}

/// Compare the wasm and rust outcomes for one program per the pairing rules. Pure — the two sides are
/// produced by [`run_wasm`] / [`run_rust`]; splitting it out keeps the rules unit-testable without a
/// compiler or a subprocess.
pub fn compare(wasm: &Side, rust: &Side) -> Diff {
    match (wasm, rust) {
        // A COMPILER PANIC on either side is a crash finding — checked FIRST so it is never masked by
        // the other side's outcome (a decline/agreement must not swallow a real compiler crash).
        (Side::CompilePanic(c), _) | (_, Side::CompilePanic(c)) => Diff::CompileCrash(c.clone()),
        // An ENV/link failure (staging externs missing → E0433 cdz_num/cdz_rt) is NOT a compiler outcome
        // and makes the comparison MEANINGLESS for this program — surface it as `Unavailable` (counted,
        // never filed) so a broken link env fails loud instead of filing false Artifact buckets. Checked
        // FIRST, before the ArtifactError arm, so an env red is never mis-classified as a miscompile.
        // (Only the rust side yields `Unavailable`; the wasm side never does.)
        (Side::Unavailable(e), _) | (_, Side::Unavailable(e)) => Diff::Unavailable(e.clone()),
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
        // Both ran to a value: agree iff the canonical strings are identical — OR denote the same value in
        // different render DIALECTS (INTERIM: wasm/cdz-run emit the M2 native `#ctor(…)` compound render,
        // cdz-rust-render still emits the pre-M2 paren-led `(ctor …)`; see [`renders_agree`]).
        (Side::Value(a), Side::Value(b)) => {
            if a == b || renders_agree(a, b) {
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
        // One value, one trap — a liveness disagreement — EXCEPT a stack-exhaustion / resource trap,
        // which is a TOLERATED resource divergence, not a liveness bug: the two backends have different
        // native stack limits (wasm traps "call stack exhausted" at ~15k non-tail frames; rust "has
        // overflowed its stack" ~10x deeper), so on deep non-tail recursion one returns a value while the
        // other traps GRACEFULLY at its own limit. Both fail safely; it is not a semantic split. (breaker
        // datapoint 2026-09.) A SEMANTIC trap (divide-by-zero, arithmetic overflow, unreachable) has a
        // distinct message and still surfaces as a Liveness mismatch.
        (Side::Value(v), Side::Trap(t)) => {
            if is_resource_trap(t) {
                Diff::Agree
            } else {
                Diff::Mismatch {
                    kind: MismatchKind::Liveness,
                    wasm: format!("value {v}"),
                    rust: format!("trap {t}"),
                }
            }
        }
        (Side::Trap(t), Side::Value(v)) => {
            if is_resource_trap(t) {
                Diff::Agree
            } else {
                Diff::Mismatch {
                    kind: MismatchKind::Liveness,
                    wasm: format!("trap {t}"),
                    rust: format!("value {v}"),
                }
            }
        }
    }
}

/// A short label for a [`Side`] in an artifact-error mismatch note (the OTHER side, whatever it was).
fn describe_side(s: &Side) -> String {
    match s {
        Side::Value(v) => format!("value {v}"),
        Side::Trap(t) => format!("trap {t}"),
        Side::Declined(d) => format!("declined {d}"),
        Side::ArtifactError(e) => format!("artifact-error {e}"),
        Side::Unavailable(e) => format!("unavailable {e}"),
        Side::CompilePanic(c) => format!("compile-panic {}", c.message),
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
    run_wasm_with_args(source, store, &[])
}

/// [`run_wasm`] but CALLING the exported entry with `args` (each coerced to a param type by `cdz-run`),
/// so a program whose `main` TAKES parameters (a runtime-`n` / heap entry) can be VALUE-checked instead
/// of failing the 0-arg call → `Declined`. The `args` must match the Lean trial's `(args …)` value-ASTs
/// (same values, string-rendered here vs value-AST there) so the two sides run the same call. Empty
/// `args` is exactly [`run_wasm`].
pub fn run_wasm_with_args(source: &str, store: &std::path::Path, args: &[String]) -> Side {
    // Parse + encode to the binary AST the compiler consumes (the same bridge `compile_catching` uses).
    let arenas = match cadenza_syntax::sexpr::read(source) {
        Ok(a) => a,
        // Unparseable generated text is a generator-quality issue, not comparable — treat as declined.
        Err(e) => return Side::Declined(format!("parse error: {}", e.0)),
    };
    let bytes = cadenza_syntax::codec::encode(&arenas);
    run_wasm_bytes(&bytes, store, args, source)
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
    // Render the decoded AST back to source so a compile-HANG on this blob files a reproducible
    // hang-witness (the binary-AST-entropy path has no source string of its own).
    let report_source = cadenza_syntax::printer::print(&arenas, 100);
    run_wasm_bytes(&bytes, store, &[], &report_source)
}

/// Compile already-encoded binary-AST `bytes` to a component and run it in-process with call `args` — the
/// shared tail of [`run_wasm`] (text path) and [`run_wasm_ast`] (binary-AST-entropy path). `args` are the
/// values passed to the exported entry (empty for a `main`/0-arg export; `cdz-run` coerces each to a param
/// type). See [`run_wasm_with_args`]. `report_source` is the program source to file if the COMPILE HANGS —
/// the wasm RUN is already epoch-bounded by `cdz_run` (a runaway loop TRAPS), but `rcdzc::compile_component`
/// is an unguarded native call, so it runs under [`crate::compile_guard::guard`]: a sweep that installed the
/// watchdog captures a compile non-termination as a `Timeout` hang-witness and aborts, instead of wedging.
fn run_wasm_bytes(
    bytes: &[u8],
    store: &std::path::Path,
    args: &[String],
    report_source: &str,
) -> Side {
    // Compile to a component under the compile-hang watchdog. A rejection/decline (errors-as-data) →
    // not comparable; a HANG (native loop) is captured + aborted by the watchdog (no-op if uninstalled).
    let component = match crate::compile_guard::guard(report_source, || {
        crate::oracle::compile_component_catching(bytes)
    }) {
        Ok(c) => c,
        Err(crate::oracle::ComponentFail::Declined(code)) => {
            return Side::Declined(code.unwrap_or_else(|| "wasm-decline".to_string()));
        }
        // A compiler PANIC — surface it so the sweep FILES a crash finding + continues, instead of the
        // unguarded native panic aborting the whole run.
        Err(crate::oracle::ComponentFail::Crashed(info)) => return Side::CompilePanic(info),
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
        args: args.to_vec(),
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

/// True if two rendered VALUE strings denote the SAME value despite render-DIALECT differences. INTERIM
/// stopgap for the post-M2 render split: wasm (`cdz-run`) emits the M2 native `#ctor(…)` compound render
/// (canonical), while `cdz-rust-render` still emits the pre-M2 paren-led `(ctor …)` — so a `#tuple(1 2)`
/// vs `(tuple 1 2)` are the same value but differ textually, false-mismatching every compound value.
/// Normalizes the `#ctor(` heads to `(ctor `, parses BOTH, and compares the CANONICAL codec encodings
/// (whitespace / empty-compound-spacing agnostic). A genuine value difference still fails (the parsed
/// structures differ → different codec bytes). Falls back to `false` if either side does not parse.
/// (v-rust-backend owns migrating cdz-rust-render to canonical `#ctor(…)`; drop this once it lands.)
/// Strip a value-doc `(: <value> <Type>)` ascription down to its bare `<value>` render. `cdz run-rust`
/// (the differential's rust side) renders via the Ty-direct value-doc as of cdz #7673 — the ascribed
/// `(: v T)` form — while `cdz run` (wasm) renders the bare `v`; an ascription is render metadata, not a
/// value, so the comparison must see through it. Extracts the FIRST balanced sub-expression after the
/// `(:` head (the value), leaving the trailing `<Type>` and the closing paren. Returns the input UNCHANGED
/// if it is not an ascription (no `(:` head) or if the value scan does not balance — so a non-ascribed or
/// malformed render is never corrupted.
fn strip_ascription(s: &str) -> String {
    let t = s.trim();
    let Some(rest) = t.strip_prefix("(:") else {
        return s.to_string();
    };
    let rest = rest.trim_start();
    // Read one balanced sub-expression: the value ends at the first depth-0 whitespace (a bare atom) or
    // when the group that started it closes (a `(…)`/`#ctor(…)`/`{…}` compound). `#ctor(` opens on the `(`.
    let mut depth: i32 = 0;
    let mut end = None;
    for (i, c) in rest.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth < 0 {
                    // Hit the ascription's own closing paren before any value — malformed; don't strip.
                    return s.to_string();
                }
            }
            c if c.is_whitespace() && depth == 0 => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    match end {
        Some(i) if depth == 0 => rest[..i].to_string(),
        _ => s.to_string(), // no separating whitespace / unbalanced → not a clean ascription; leave as-is
    }
}

fn renders_agree(a: &str, b: &str) -> bool {
    // Strip a value-doc `(: <value> <Type>)` ascription first: as of cdz #7673 (op-seq-210, default-on),
    // `cdz run-rust` renders the boundary value via the Ty-direct value-doc — the ASCRIBED `(: v T)` form
    // — while `cdz run` (wasm) still renders the BARE `v`. Same value, different render dialect; comparing
    // the bare values keeps the oracle sound (an ascription is metadata, not a value difference — the lean
    // oracle strips it the same way). Applied to BOTH sides so it is robust whichever ascribes.
    let (a, b) = (strip_ascription(a), strip_ascription(b));
    let (na, nb) = (normalize_render_dialect(&a), normalize_render_dialect(&b));
    if na == nb {
        return true;
    }
    match (
        cadenza_syntax::sexpr::read(&na),
        cadenza_syntax::sexpr::read(&nb),
    ) {
        (Ok(pa), Ok(pb)) => {
            cadenza_syntax::codec::encode(&pa) == cadenza_syntax::codec::encode(&pb)
        }
        _ => false,
    }
}

/// Rewrite the M2 native `#<ctor>(` compound-render heads to the paren-led `(<ctor> ` form, so a native
/// `#ctor(…)` render and a legacy `(ctor …)` render of the same value normalize to a common, parseable
/// dialect. Only an `#<ident>(` sequence is rewritten; a bare `#` (or `#foo` not followed by `(`) is left
/// as-is. See [`renders_agree`].
fn normalize_render_dialect(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut rest = s;
    while let Some(hash) = rest.find('#') {
        out.push_str(&rest[..hash]);
        let after = &rest[hash + 1..];
        let idlen = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'))
            .unwrap_or(after.len());
        if idlen > 0 && after[idlen..].starts_with('(') {
            out.push('(');
            out.push_str(&after[..idlen]);
            out.push(' ');
            rest = &after[idlen + 1..];
        } else {
            out.push('#');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Strip the `(: <value> <Type>)` value-form wrapper down to the bare `<value>`, matching what
/// `cdz run-rust` (and a scalar `cdz-run` result) prints. A payload that is NOT a value-form is
/// returned unchanged. Mirrors the corpus gate's `expected_value`: take the FIRST balanced token after
/// `(:` — a `(…)` group, an M2 native `#ctor(…)` compound render, a `"…"` string (which may contain
/// spaces), or a bare atom up to the next space.
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
        // An M2 native COMPOUND value render: `#<ctor>(<balanced …>)` — e.g. `#tuple(1 2)`, `#list(1 2 3)`,
        // `#record((= a 1) (= b 2))`, `#set(…)`, `#map(…)`. The `#ctor` head is followed by a balanced
        // `(…)` group that MAY contain spaces and nested compounds, so take `#ctor` THROUGH the matching
        // close paren (the bare-atom arm below would wrongly truncate at the first inner space → `#tuple(1`).
        Some(b'#') => match rest.find('(') {
            Some(open) => {
                let mut depth = 0i32;
                for (i, &b) in bytes.iter().enumerate().skip(open) {
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
            // A bare `#name` head with no parens — up to the next space (drop a trailing `)`).
            None => match rest.find(char::is_whitespace) {
                Some(idx) => rest[..idx].to_string(),
                None => rest.trim_end_matches(')').to_string(),
            },
        },
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

/// True if a `cdz run-rust` `error …` verdict is the known staging-extern LINK failure: rustc `E0433`
/// (cannot find crate/module) naming the `cdz_num`/`cdz_rt` runtime rlibs that must be co-located on the
/// link path (a per-worktree build-env condition, not the emitted source). Such a verdict is an ENV
/// failure, not a compiler miscompile — see [`Side::Unavailable`]. Every program reds identically when the
/// rlibs are absent, so misclassifying it as `ArtifactError` files N false `Artifact` findings per campaign.
fn is_staging_extern_link_failure(err: &str) -> bool {
    err.contains("E0433") && (err.contains("cdz_num") || err.contains("cdz_rt"))
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
        let e = e.trim();
        if is_staging_extern_link_failure(e) {
            // NOT a miscompile: the emitted `.rs` is fine but the staging externs (`cdz_num`/`cdz_rt`
            // rlibs) are absent from the rustc link path, so EVERY program reds identically. An env
            // failure → `Unavailable` (never a finding) so a broken link env fails LOUD, not as false
            // Artifact buckets (breaker's + the S79 false-positive class).
            Side::Unavailable(e.to_string())
        } else {
            // A non-compiling emitted artifact (rustc rejected the emitted `.rs`) — a build-blocking
            // MISCOMPILE. Its own `Side::ArtifactError` so `compare` ALWAYS surfaces it, even against a
            // wasm trap (a `Side::Trap` here would be swallowed by the trap-vs-trap agreement — PR#552).
            Side::ArtifactError(e.to_string())
        }
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
            // The wasm side never yields `Unavailable` (an env/link failure is rust-side only), but the
            // match must stay exhaustive — fold it in with the other non-value outcomes. A `CompilePanic`
            // (a compiler crash on a corpus seed) is caught rather than aborting; folded here too (this
            // plumbing smoke only tallies — the differential/crash oracle is the crash-mining path).
            Side::Declined(_)
            | Side::ArtifactError(_)
            | Side::Unavailable(_)
            | Side::CompilePanic(_) => stats.declined += 1,
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
        // `Unavailable` is rust-side only (this bridges the wasm side), but keep the match exhaustive.
        // A `CompilePanic` (compiler crash) is not a comparable value → skip the trial.
        Side::Declined(_)
        | Side::ArtifactError(_)
        | Side::Unavailable(_)
        | Side::CompilePanic(_) => None,
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
    /// Comparable trials the oracle process could NOT judge — it errored on them (e.g. an AST leaf kind
    /// the oracle can't DECODE yet, like a post-flag-day codec gap). Isolated per-program so one such
    /// trial does not abort the sweep; an oracle-CAPABILITY gap (not a bug, distinct from a modelled `skip`).
    pub oracle_undecodable: usize,
}

/// Run each program source under the WASM backend, batch the comparable (program, rcdzc-output) trials,
/// and judge each batch with `oracle-check --batch-stream`. Tallies holds/mismatch/skip; every
/// `Mismatch` pushes `(source, oracle-detail)` into `mismatches` (a candidate rcdzc bug). Batching is
/// the async unit the operator's pipeline overlaps (a fresh `oracle-check` per batch judges while the
/// next batch compiles). `sources` should be TERMINATING programs (e.g. `generator::generate`'s
/// structurally-terminating grammar) — the in-process wasm run has no hang guard.
/// The `Int64` arguments a runtime-`n` main may be given — chosen PER-PROGRAM (by a stable hash of `src`)
/// to spread across BOUNDARY values: base cases (0, 1), a mid value (7), a max in-range shift (63), an
/// OUT-OF-RANGE shift (64 — now TRAPS at construction post strict-arg-eval, so it agrees rather than
/// false-mismatching), an overflow-inducing magnitude (1000000), and a negative (-1). Spreading `n` this
/// way exercises the shift/overflow/base-case boundaries of the runtime-`n` grammar instead of a single
/// mid-range value. Both the wasm call and the Lean trial use the SAME picked value.
const RUNTIME_N_ARGS: &[&str] = &["0", "1", "7", "63", "64", "1000000", "-1"];

/// Pick a runtime-`n` arg for `src`: a stable per-source index into [`RUNTIME_N_ARGS`] (FNV-1a of the
/// bytes) so different programs get different boundary values while each program stays deterministic
/// (the wasm side and the Lean trial must agree on which `n` was used).
fn runtime_n_arg(src: &str) -> &'static str {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in src.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    RUNTIME_N_ARGS[(h % RUNTIME_N_ARGS.len() as u64) as usize]
}

/// The call args to supply for `src`'s exported `main`, so a PARAM'd main is VALUE-checkable instead of
/// failing the 0-arg call → not-comparable. Matches the exact main-param forms `build_program` emits:
/// runtime-`n` → one `Int64`; a heap param (`v0`, left UNUSED in the body, so ANY valid arg of the type
/// lets the call succeed and the result depends only on the body) → one `String`/`(List Int64)`/
/// `(Option Int64)` value. A param-less main → none. `Bytes` is OMITTED (no `cdz-run` `coerce_one` arg
/// form) so a Bytes-param main stays 0-arg / not-comparable exactly as before (no regression). The arg
/// here (a string) and the Lean trial's `(args …)` (the same value as a value-AST) must denote the SAME
/// value so both sides run the identical call.
fn main_call_args(src: &str) -> Vec<String> {
    if src.contains("(def (main (: n Int64))") {
        vec![runtime_n_arg(src).to_string()]
    } else if src.contains("(def (main (: v0 String))") {
        vec!["\"x\"".to_string()]
    } else if src.contains("(def (main (: v0 (List Int64)))") {
        vec!["(list 1 2)".to_string()]
    } else if src.contains("(def (main (: v0 (Option Int64)))") {
        vec!["(Some 1)".to_string()]
    } else {
        Vec::new()
    }
}

pub fn lean_differential_sweep(
    sources: &[String],
    store: &std::path::Path,
    oracle_bin: &std::path::Path,
    batch_size: usize,
    mismatches: &mut Vec<(String, String)>,
    declines: &mut Vec<(String, String)>,
) -> std::io::Result<LeanDiffStats> {
    let mut stats = LeanDiffStats::default();
    let mut batch_srcs: Vec<String> = Vec::new();
    let mut batch_trials: Vec<crate::lean::Trial> = Vec::new();
    let batch_size = batch_size.max(1);

    for src in sources {
        // Supply main's call args (runtime-`n` Int64 / heap String/List/Option; empty for param-less) so a
        // PARAM'd main is VALUE-checkable — both the wasm call and the Lean trial use the SAME value. A
        // param-less (or unsupported Bytes-param) main gets none = the old 0-arg path. See [`main_call_args`].
        let call_args = main_call_args(src);
        let side = run_wasm_with_args(src, store, &call_args);
        // Capture DECLINES — a shape the front-end/backend does not compile yet — for the breaker
        // decline→corpus gap hand-off (operator directive). A decline is EXPECTED output (never a bug),
        // but it is a coverage GAP worth pinning; `(source, reason)` is deduped by signature at the CLI.
        if let Side::Declined(reason) = &side {
            declines.push((src.clone(), reason.clone()));
        }
        let output = match side_to_rcdzc_output(side) {
            Some(o) => o,
            None => {
                stats.not_comparable += 1;
                continue;
            }
        };
        // The trial carries the FULL program + the SAME call args (as value-ASTs) so the oracle runs the
        // IDENTICAL call the wasm side did (empty args = the oracle's 0-arg re-root, i.e. `main_0`).
        let Ok(program) = cadenza_syntax::sexpr::read(src) else {
            stats.not_comparable += 1;
            continue;
        };
        let arg_asts: Result<Vec<_>, _> = call_args
            .iter()
            .map(|a| cadenza_syntax::sexpr::read(a))
            .collect();
        let Ok(args) = arg_asts else {
            stats.not_comparable += 1;
            continue;
        };
        batch_srcs.push(src.clone());
        batch_trials.push(crate::lean::Trial {
            program,
            args,
            output,
        });
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

/// A dedup signature for a decline reason — the `CDZNNNN` diagnostic code if present, else a normalized
/// short prefix of the reason. Groups the many declining programs a campaign hits into distinct GAP
/// classes for the breaker decline→corpus hand-off (one repro per signature, not one per instance).
pub fn decline_signature(reason: &str) -> String {
    // Prefer an embedded `CDZ<digits>` diagnostic code.
    if let Some(pos) = reason.find("CDZ") {
        let code: String = reason[pos..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if code.len() > 3 && code.as_bytes()[3].is_ascii_digit() {
            return code;
        }
    }
    // Else a normalized short prefix of the reason. First strip BACKTICK-quoted spans (the variable parts
    // — an op/type/identifier name like `o` / `Bytes`), so a gap CLASS dedups regardless of the specific
    // op or type: "the host operation `o` … result … `Bytes`" and "… `p` … `(List Int64)`" reduce to ONE
    // signature (the gap = "host op result crosses no boundary"), not one per op/type instance.
    let stripped = strip_backtick_spans(reason);
    let prefix: String = stripped
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    let norm: String = prefix
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(48)
        .collect();
    norm.to_ascii_lowercase()
}

/// Remove `` `…` `` backtick-quoted spans from `s` (the variable identifier/type names in a decline
/// reason), leaving the fixed wording — so [`decline_signature`] groups by gap class, not by instance.
fn strip_backtick_spans(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tick = false;
    for ch in s.chars() {
        match ch {
            '`' => in_tick = !in_tick,
            _ if !in_tick => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Judge one batch of trials and fold the verdicts into `stats` (+ collect mismatches by source).
fn judge_and_tally(
    oracle_bin: &std::path::Path,
    srcs: &[String],
    trials: &[crate::lean::Trial],
    stats: &mut LeanDiffStats,
    mismatches: &mut Vec<(String, String)>,
) -> std::io::Result<()> {
    match crate::lean::judge_batch(oracle_bin, trials) {
        Ok(verdicts) => {
            for (src, verdict) in srcs.iter().zip(verdicts) {
                fold_verdict(src, verdict, stats, mismatches);
            }
        }
        Err(_batch_err) => {
            // The batch failed as a WHOLE — the oracle process errored mid-stream (a `--batch-stream`
            // exit is all-or-nothing: e.g. a single program carrying an AST leaf kind the oracle can't
            // DECODE yet aborts the stream, losing every verdict). Isolate: RE-JUDGE each trial on its
            // own so one undecodable program does not sink the rest of the sweep. A single-trial batch
            // that STILL errors = that one program is beyond the oracle's current decode/model
            // capability → count it as an oracle-capability gap (`oracle_undecodable`) and continue.
            // (Once the oracle gains the missing kind, the fast batched path succeeds and this is unused.)
            for (src, trial) in srcs.iter().zip(trials) {
                match crate::lean::judge_batch(oracle_bin, std::slice::from_ref(trial)) {
                    Ok(mut verdicts) => {
                        if let Some(verdict) = verdicts.pop() {
                            fold_verdict(src, verdict, stats, mismatches);
                        }
                    }
                    Err(_) => stats.oracle_undecodable += 1,
                }
            }
        }
    }
    Ok(())
}

/// Fold ONE judged trial's verdict into the running tally (+ collect a mismatch by source). Shared by the
/// fast batched path and the per-trial isolation fallback in [`judge_and_tally`].
fn fold_verdict(
    src: &str,
    verdict: crate::lean::Verdict,
    stats: &mut LeanDiffStats,
    mismatches: &mut Vec<(String, String)>,
) {
    stats.trials += 1;
    match verdict {
        crate::lean::Verdict::Holds => stats.holds += 1,
        crate::lean::Verdict::Skip(_) => stats.skips += 1,
        crate::lean::Verdict::Mismatch(detail) => {
            // Every mismatch is a candidate rcdzc bug and is filed. This INCLUDES the runtime-fault
            // trap-KIND class (oracle=specific-kind vs rcdzc=generic `unreachable`): the operator ruled
            // (Option 2) the oracle's specific kind is AUTHORITATIVE and rcdzc's `unreachable` is an
            // imprecision to close compiler-side — so it is a real rcdzc-side finding, no longer
            // suppressed. (The compound-`=` short-circuit that once over-forced is fixed oracle-side in
            // v-lean-oracle #4893; float-literal mismatches are trustworthy since #4818.)
            stats.mismatches += 1;
            mismatches.push((src.to_string(), detail));
        }
    }
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
    fn a_compile_panic_is_a_compile_crash_and_is_never_masked() {
        let info = crate::oracle::CrashInfo {
            site: Some("crates/rcdzc/src/infer/node.rs:1830:28".into()),
            message: "index out of bounds: the len is 0 but the index is 0".into(),
            backtrace: String::new(),
        };
        // A compiler panic on the wasm side → CompileCrash, regardless of what the rust side did — even a
        // decline (which normally means "not comparable / agree") must NOT swallow a real compiler crash.
        assert_eq!(
            compare(
                &Side::CompilePanic(info.clone()),
                &Side::Declined("x".into())
            ),
            Diff::CompileCrash(info.clone())
        );
        assert_eq!(
            compare(&Side::Value("1".into()), &Side::CompilePanic(info.clone())),
            Diff::CompileCrash(info)
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

    /// A value-vs-(STACK-EXHAUSTION trap) split is a TOLERATED resource divergence (different backend
    /// stack limits), NOT a liveness mismatch (breaker datapoint) — on either side. A SEMANTIC trap
    /// (divide-by-zero) is still a liveness mismatch.
    #[test]
    fn value_vs_stack_exhaustion_trap_is_tolerated() {
        for t in [
            "call stack exhausted",
            "wasm `unreachable` — call stack exhausted",
            "thread 'main' has overflowed its stack",
        ] {
            assert!(
                matches!(
                    compare(&Side::Value("7".into()), &Side::Trap(t.into())),
                    Diff::Agree
                ),
                "wasm-value vs resource-trap {t:?} should AGREE"
            );
            assert!(
                matches!(
                    compare(&Side::Trap(t.into()), &Side::Value("7".into())),
                    Diff::Agree
                ),
                "resource-trap {t:?} vs rust-value should AGREE"
            );
        }
        // A SEMANTIC trap is NOT tolerated — still a liveness mismatch.
        assert!(matches!(
            compare(
                &Side::Value("7".into()),
                &Side::Trap("integer divide by zero".into())
            ),
            Diff::Mismatch {
                kind: MismatchKind::Liveness,
                ..
            }
        ));
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

    /// The staging-extern LINK failure (E0433 cdz_num/cdz_rt) is an ENV condition, not a miscompile:
    /// `parse_rust_verdict` classifies it `Unavailable` (not `ArtifactError`), and `compare` maps it to
    /// `Diff::Unavailable` (never a finding), while a GENUINE artifact error still surfaces as a mismatch.
    /// Guards the S121 fix (breaker's env-red-masquerading-as-findings class).
    #[test]
    fn staging_extern_link_failure_is_unavailable_not_a_finding() {
        // The known env red — every program reds identically with it when the rlibs aren't staged.
        let env_verdict = "error error[E0433]: cannot find module or crate `cdz_num` in this scope";
        assert!(
            matches!(parse_rust_verdict(env_verdict), Side::Unavailable(_)),
            "E0433 cdz_num link failure must be Unavailable, not ArtifactError"
        );
        assert!(is_staging_extern_link_failure(
            "error[E0433]: cannot find crate `cdz_rt`"
        ));
        // A GENUINE emit miscompile (a type error in the emitted source) stays an ArtifactError.
        assert!(matches!(
            parse_rust_verdict("error error[E0308]: mismatched types"),
            Side::ArtifactError(_)
        ));
        assert!(!is_staging_extern_link_failure(
            "error[E0308]: mismatched types"
        ));
        // compare: an env-Unavailable rust side (against any wasm outcome) is NEVER a finding.
        assert!(matches!(
            compare(
                &Side::Value("3".into()),
                &Side::Unavailable("E0433 cdz_num".into())
            ),
            Diff::Unavailable(_)
        ));
        assert!(matches!(
            compare(
                &Side::Trap("t".into()),
                &Side::Unavailable("E0433 cdz_num".into())
            ),
            Diff::Unavailable(_)
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
        // M2 native compound renders `#ctor(…)` must NOT be cut at the first inner space (the S107 bug:
        // `#tuple(1 2)` was truncated to `#tuple(1`). Take `#ctor` through the matching close paren.
        assert_eq!(
            strip_value_annotation("(: #tuple(1 2) (Tuple Int64 Int64))"),
            "#tuple(1 2)"
        );
        assert_eq!(
            strip_value_annotation("(: #list(1 2 3) (List Int64))"),
            "#list(1 2 3)"
        );
        // A nested compound (record with an inner space + nested parens) stays intact.
        assert_eq!(
            strip_value_annotation("(: #record((= a 1) (= b 2)) (Record (: a Int64) (: b Int64)))"),
            "#record((= a 1) (= b 2))"
        );
        // An empty native compound `#set()`.
        assert_eq!(strip_value_annotation("(: #set() (Set Int64))"), "#set()");
    }

    /// The interim render-dialect normalizer: the M2 native `#ctor(…)` render (wasm) and the pre-M2
    /// paren-led `(ctor …)` render (cdz-rust-render) of the SAME value AGREE, while a genuine value
    /// difference still DISAGREES (must not be masked). Guards the S114 wasm-vs-rust stopgap.
    #[test]
    fn renders_agree_bridges_the_m2_dialect_split_without_masking_real_diffs() {
        // Same value, different dialect → AGREE.
        assert!(renders_agree("#tuple(1 2)", "(tuple 1 2)"));
        assert!(renders_agree("#list(1 2 3)", "(list 1 2 3)"));
        assert!(renders_agree("#set(1 2 3)", "(set 1 2 3)"));
        // Nested + the empty-compound spacing split (`#tuple()` vs `(tuple)`).
        assert!(renders_agree(
            "#tuple(true #tuple())",
            "(tuple true (tuple))"
        ));
        // Identical scalars → AGREE (fast path).
        assert!(renders_agree("42", "42"));
        // A GENUINE value difference must NOT be masked → DISAGREE.
        assert!(!renders_agree("#tuple(1 2)", "(tuple 1 3)"));
        assert!(!renders_agree("#list(1 2 3)", "(list 1 2)"));
        assert!(!renders_agree("41", "42"));
    }

    #[test]
    fn renders_agree_sees_through_the_value_doc_ascription() {
        // cdz #7673: `cdz run-rust` ascribes `(: v T)`, wasm renders bare `v`. Same value → AGREE.
        assert!(renders_agree("0", "(: 0 Int64)"));
        assert!(renders_agree("14.43", "(: 14.43 Float32)"));
        assert!(renders_agree(
            "#list(#tuple(1 1))",
            "(: #list(#tuple(1 1)) (List (Tuple Int64 Int64)))"
        ));
        assert!(renders_agree("(: 5 Int64)", "(: 5 Int64)")); // both ascribed
        assert!(renders_agree(
            "(: #tuple(1 2) (Tuple Int64 Int64))",
            "#tuple(1 2)"
        ));
        // A GENUINE value difference must NOT be masked even under ascription.
        assert!(!renders_agree("(: 5 Int64)", "(: 6 Int64)"));
        assert!(!renders_agree("5", "(: 6 Int64)"));
        assert!(!renders_agree(
            "(: #list(1 2) (List Int64))",
            "(: #list(1 3) (List Int64))"
        ));
    }

    #[test]
    fn strip_ascription_extracts_the_value_or_leaves_non_ascriptions() {
        assert_eq!(strip_ascription("(: 0 Int64)"), "0");
        assert_eq!(strip_ascription("(: 14.43 Float32)"), "14.43");
        assert_eq!(
            strip_ascription("(: #list(#tuple(1 1)) (List (Tuple Int64 Int64)))"),
            "#list(#tuple(1 1))"
        );
        // Not an ascription → unchanged.
        assert_eq!(strip_ascription("#tuple(1 2)"), "#tuple(1 2)");
        assert_eq!(strip_ascription("42"), "42");
        assert_eq!(strip_ascription("(tuple 1 2)"), "(tuple 1 2)");
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
                // `cdz run-rust` value-doc-ascribes as of cdz #7673 (`(: "ayg" String)`); assert it ran to
                // a value and the wasm-vs-rust comparison AGREES (the ascription is stripped).
                assert!(
                    matches!(rust, Side::Value(_)),
                    "rust ran to a value: {rust:?}"
                );
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
                // `cdz run-rust` renders via the value-doc as of cdz #7673 — the ascribed `(: 3 Int64)`,
                // not the bare `3` — so assert the meaningful invariant: it ran to a value and the
                // wasm-vs-rust comparison AGREES (the ascription is stripped by `renders_agree`).
                assert!(
                    matches!(rust, Side::Value(_)),
                    "rust ran to a value: {rust:?}"
                );
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

    /// `decline_signature` groups declines by their CDZ code when present, else a normalized reason
    /// prefix — so the breaker hand-off gets ONE repro per distinct gap class, not one per instance.
    #[test]
    fn decline_signature_groups_by_code_then_prefix() {
        assert_eq!(
            decline_signature("CDZ0304: shift count out of range"),
            "CDZ0304"
        );
        assert_eq!(decline_signature("declined CDZ0101 near stuff"), "CDZ0101");
        // Same code from two different programs → same signature (deduped together).
        assert_eq!(
            decline_signature("CDZ0304 here"),
            decline_signature("CDZ0304 elsewhere entirely")
        );
        // No code → a normalized, spaceless, lowercased prefix; a bare "CDZ" with no digits is NOT a code.
        let s = decline_signature("not lowered yet: some construct");
        assert!(!s.is_empty() && s == s.to_ascii_lowercase() && !s.contains(' '));
        assert_ne!(decline_signature("CDZ without digits"), "CDZ");
        // Backtick-quoted op/type names are STRIPPED, so a host-boundary gap CLASS dedups regardless of
        // the specific op (`o` vs `p`) or result type (`Bytes` vs `(List Int64)`) — one gap, one repro.
        let g_o_bytes =
            decline_signature("the host operation `o` has a result of type `Bytes`, which …");
        let g_p_list = decline_signature(
            "the host operation `p` has a result of type `(List Int64)`, which …",
        );
        assert_eq!(
            g_o_bytes, g_p_list,
            "same host-result gap class must share a signature"
        );
        // A genuinely different gap keeps a different signature.
        assert_ne!(
            g_o_bytes,
            decline_signature("delegating more than one host effect is not yet emitted")
        );
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
        let mut declines = Vec::new();
        let stats =
            lean_differential_sweep(&sources, store, &oracle, 8, &mut mismatches, &mut declines)
                .expect("sweep runs");
        assert_eq!(stats.trials, 2, "both scalars are comparable");
        assert_eq!(
            stats.mismatches, 0,
            "benign scalars must not mismatch: {mismatches:?}"
        );
        assert!(mismatches.is_empty());
    }

    /// SWEEP INTEGRATION (S125): a runtime-`n` main — which was NOT-COMPARABLE (the 0-arg call fails) — is
    /// now VALUE-CHECKED because the sweep supplies its `Int64` arg on both sides. `(+ n 1)` @ n=7 → 8 on
    /// both → 1 trial, 1 hold, 0 mismatch, 0 not-comparable. Skips unless the real oracle is discoverable.
    #[test]
    fn a_runtime_n_main_is_value_checked_by_the_sweep() {
        let Some(oracle) = crate::lean::discover_oracle_check() else {
            eprintln!("skipping: no oracle-check (nix build .#oracle-lean)");
            return;
        };
        let sources = vec!["(do (def (main (: n Int64)) (+ n 1)) (export main))".to_string()];
        let store = std::path::Path::new("/nonexistent-store"); // pure Int64 arith needs no runtime
        let mut mismatches = Vec::new();
        let mut declines = Vec::new();
        let stats =
            lean_differential_sweep(&sources, store, &oracle, 8, &mut mismatches, &mut declines)
                .expect("sweep runs");
        assert_eq!(
            stats.trials, 1,
            "the runtime-n main is now comparable (arg supplied), not skipped"
        );
        assert_eq!(stats.holds, 1, "(+ n 1) @ n=7 = 8 must HOLD");
        assert_eq!(stats.mismatches, 0);
        assert_eq!(
            stats.not_comparable, 0,
            "no longer not-comparable now that the arg is supplied"
        );
    }

    /// `main_call_args` supplies the right arg per entry shape (validated at scale by a campaign — a
    /// heap-param main needs the staged runtime store, so this is a pure-string-mapping unit test only).
    #[test]
    fn main_call_args_supplies_one_arg_per_param_shape() {
        // runtime-n → exactly ONE Int64 arg, drawn from the boundary set (per-source pick).
        let n_args = main_call_args("(do (def (main (: n Int64)) (+ n 1)) (export main))");
        assert_eq!(n_args.len(), 1);
        assert!(
            RUNTIME_N_ARGS.contains(&n_args[0].as_str()),
            "runtime-n arg {n_args:?} must be one of the boundary values"
        );
        assert_eq!(
            main_call_args("(do (def (main (: v0 String)) 0) (export main))"),
            vec!["\"x\"".to_string()]
        );
        assert_eq!(
            main_call_args("(do (def (main (: v0 (List Int64))) 0) (export main))"),
            vec!["(list 1 2)".to_string()]
        );
        assert_eq!(
            main_call_args("(do (def (main (: v0 (Option Int64))) 0) (export main))"),
            vec!["(Some 1)".to_string()]
        );
        // Param-less → none; an unsupported Bytes param → none (stays 0-arg / not-comparable, no regression).
        assert!(main_call_args("(do (def (main) 42) (export main))").is_empty());
        assert!(main_call_args("(do (def (main (: v0 Bytes)) 0) (export main))").is_empty());
    }

    /// FEASIBILITY (S124): a PARAM'd main can be VALUE-checked by SUPPLYING an arg — `run_wasm_with_args`
    /// passes the call arg to the wasm side, and the Lean trial carries the SAME value in `(args …)`, so
    /// the oracle re-runs the same call. `(def (main (: n Int64)) (+ n 1))` @ n=5 → 6 on BOTH sides → HOLDS.
    /// This proves the arg-passing pipeline end-to-end (the foundation for recovering the ~½ of programs
    /// that are param'd mains, currently not-comparable). Skips unless the real oracle is discoverable.
    #[test]
    fn a_param_main_value_checks_with_a_supplied_arg() {
        let Some(oracle) = crate::lean::discover_oracle_check() else {
            eprintln!("skipping: no oracle-check (nix build .#oracle-lean)");
            return;
        };
        let src = "(do (def (main (: n Int64)) (+ n 1)) (export main))";
        let store = std::path::Path::new("/nonexistent-store"); // pure Int64 arith needs no runtime
        let side = run_wasm_with_args(src, store, &["5".to_string()]);
        let Side::Value(v) = &side else {
            panic!("runtime-n main with arg 5 should run to a value, got {side:?}");
        };
        let output = crate::lean::RcdzcOutput::value_from_render(v)
            .expect("the wasm value renders + parses");
        let program = cadenza_syntax::sexpr::read(src).expect("program parses");
        let arg = cadenza_syntax::sexpr::read("5").expect("arg parses");
        let trial = crate::lean::Trial {
            program,
            args: vec![arg],
            output,
        };
        let verdicts =
            crate::lean::judge_batch(&oracle, &[trial]).expect("oracle judges the batch");
        assert_eq!(verdicts.len(), 1);
        assert!(
            matches!(verdicts[0], crate::lean::Verdict::Holds),
            "a runtime-n main (+ n 1) @ n=5 must HOLD (wasm 6 == oracle 6) — the oracle binds trial args, \
             got {:?}",
            verdicts[0]
        );
    }

    /// A failing oracle process must be ISOLATED, not fatal to the sweep (S103 resilience). A stand-in
    /// "oracle" that always exits non-zero (`/bin/false`) — like an oracle that can't DECODE a program's
    /// AST leaf kind — makes every `--batch-stream` call error; the sweep must fall back to per-program
    /// judging, classify each still-failing trial as `oracle_undecodable`, and COMPLETE (return `Ok`)
    /// rather than aborting. Version-INDEPENDENT (uses a stub, not the real oracle), so it does not couple
    /// to the oracle artifact version the NOTE below warns against.
    #[test]
    fn a_failing_oracle_is_isolated_not_fatal_to_the_sweep() {
        let oracle = std::path::Path::new("/bin/false");
        if !oracle.exists() {
            eprintln!("skipping: no /bin/false on this platform");
            return;
        }
        let sources = vec![
            "(do (def (main) (+ 1 2)) (export main))".to_string(),
            "(do (def (main) 42) (export main))".to_string(),
        ];
        let store = std::path::Path::new("/nonexistent-store"); // pure scalars need no runtime
        let mut mismatches = Vec::new();
        let mut declines = Vec::new();
        let stats =
            lean_differential_sweep(&sources, store, oracle, 8, &mut mismatches, &mut declines)
                .expect("a failing oracle must NOT error the sweep — it isolates per program");
        assert_eq!(
            stats.trials, 0,
            "no trial graded (the stub oracle always fails)"
        );
        assert_eq!(
            stats.oracle_undecodable, 2,
            "both comparable trials classify as oracle-undecodable, not fatal"
        );
        assert_eq!(stats.mismatches, 0);
        assert!(mismatches.is_empty());
    }

    // NOTE: cdz-smith deliberately does NOT unit-test the oracle's float-literal (or any value-domain)
    // behavior — that couples this suite to v-lean-oracle's external `oracle-check` ARTIFACT VERSION (a
    // pre-#4818 oracle would fail such a test; a post-fix one would pass). The oracle's f64-rounding
    // semantics are v-lean-oracle's to test (their #4818 #guard). cdz-smith tests its own WIRE + sweep
    // logic (the pure tests above + `lean_differential_sweep_holds_for_benign_scalars`, whose Int64
    // programs hold on any oracle version). Float-literal holds are validated by CAMPAIGN runs against a
    // freshly-built oracle, not a standing test.
}
