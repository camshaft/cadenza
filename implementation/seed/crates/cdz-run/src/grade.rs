//! Grade a compiled artifact against a shredded `test-run.ast` — the EXEC phase of the corpus nix
//! caching pipeline (`design/DESIGN-corpus-nix-per-case-caching.md`).
//!
//! The shred writes one `test-run.ast` per case (`cdz corpus records --out-dir`): the case's run/grade
//! metadata as binary AST — description, trials (each an optional `(call export)` + `(arg …)`s + an
//! expected outcome), a host-response tape, and the expected host-call sequence. This module decodes it,
//! runs each RUNNABLE trial against the emitted component with `cdz-run`'s own machinery, and grades the
//! outcome — reproducing EXACTLY the value/trap comparison the `cargo xtask gate` corpus grader does
//! (`expected_value` + `trap_kind` are ported verbatim).
//!
//! EXEC vs BUILD split: this phase grades only what a run of `{artifact, test-run.ast}` can observe — an
//! `(expect-output …)` value, an `(expect-trap …)`, and the observed host-call sequence. An
//! `(expect-error …)` / `(expect-declines …)` outcome is a property of the COMPILE (there is no runnable
//! artifact — the compiler refused), graded at the build phase; and `(warns …)` are compile-stderr
//! diagnostics, likewise build-phase. Those trials are skipped here (a case is normally a single outcome
//! kind, so a pure error/declines case has nothing to run — the build derivation is its whole test).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use cadenza_syntax::ast::{Arenas, Struct, StructId};
use cadenza_syntax::codec;

use crate::{HostResponse, Outcome, RunOpts, run_capturing};

/// A per-case grade verdict — the same three-way the corpus gate uses. `Todo` is a run that happened but
/// could not be classified as pass or fail (a real trap whose reason maps to no canonical kind) — NOT a
/// failure, so it never fails the exec derivation; only `Fail` does.
#[derive(Debug, PartialEq)]
enum Grade {
    Pass,
    Todo(String),
    Fail(String),
}

impl Grade {
    /// Combine verdicts, keeping the WORST (Fail > Todo > Pass) — the case verdict is the worst of its
    /// trials + checks, matching the gate's `grade_ran` (any Fail → Fail, else any Todo → Todo, else Pass).
    fn worse(self, other: Grade) -> Grade {
        match (&self, &other) {
            (Grade::Fail(_), _) => self,
            (_, Grade::Fail(_)) => other,
            (Grade::Todo(_), _) => self,
            (_, Grade::Todo(_)) => other,
            _ => Grade::Pass,
        }
    }
}

/// One decoded trial: an optional call (`export` + argument value-forms) and the expected outcome.
struct GTrial {
    call: Option<GCall>,
    expect: GExpect,
}

struct GCall {
    export: String,
    args: Vec<String>,
}

/// The expected outcome of a trial. `Output`/`Trap` are RUN outcomes (graded against the wasm run);
/// `Error`/`Declines` are COMPILE outcomes (graded against the captured compiler diagnostic — the build
/// phase's `compile.status` + `compile.err`, passed via `--compile-status`/`--compile-diag`).
enum GExpect {
    Output(String),
    Trap(String),
    /// `(expect-error CODE msg?)` — the compiler must REFUSE with exactly `CODE`, and (if given) a
    /// diagnostic message CONTAINING the substring.
    Error(String, Option<String>),
    /// `(expect-declines msg?)` — the compiler must refuse (any code, or codeless); the optional message
    /// substring must appear in the diagnostic.
    Declines(Option<String>),
}

/// A decoded `test-run.ast`: the case's run/grade metadata.
struct TestRun {
    description: String,
    trials: Vec<GTrial>,
    /// The recorded host-call response tape, `(op, value)` in call order — consumed by a program that
    /// delegates an effect to the host.
    host_responses: Vec<(String, String)>,
    /// The expected observed host-call op sequence; verified against the run's actual calls.
    host_calls: Vec<String>,
    /// The WARNING diagnostics a compiles-clean case pins (`(warns CODE msg?)…`) — each `(code, optional
    /// message-substring)`. A PRESENCE check against the compiler's warnings (`--compile-diag`), orthogonal
    /// to the primary outcome.
    warns: Vec<(String, Option<String>)>,
}

/// Grade `component` against `test_run_ast`, running each runnable trial. `component_name` (a case's
/// `(component-name …)`, when it imposed a world) qualifies the call as `<iface>#<export>`, matching the
/// gate. Returns the process exit code: `0` if every runnable trial passed or was `Todo` (or there were
/// none — a build-graded case), `1` on the first `Fail`. The verdict + description go to stdout so the
/// aggregate step can collect them; a `Fail` reason also goes to stderr.
pub fn grade(
    component_bytes: Option<&[u8]>,
    test_run_ast: &[u8],
    runtime: Option<Vec<u8>>,
    runtime_cache_dir: Option<PathBuf>,
    component_name: Option<&str>,
    compile_status: i32,
    compile_diag: &str,
) -> Result<ExitCode> {
    let test_run = decode_test_run(test_run_ast).context("decoding test-run.ast")?;
    let host_responses: Vec<HostResponse> = test_run
        .host_responses
        .iter()
        .map(|(op, value)| HostResponse {
            op: op.clone(),
            value: value.clone(),
        })
        .collect();
    let compiled = compile_status == 0;

    let mut worst: Grade = Grade::Pass;
    let mut ran_a_trial = false;
    // The observed host-call sequence of the first value-producing trial — the gate checks host_calls
    // against exactly this (a compiled program's host effects are the same on every trial).
    let mut first_observed: Option<Vec<String>> = None;

    for trial in &test_run.trials {
        // COMPILE-OUTCOME expectations (error/declines) are graded against the captured diagnostic, not a
        // run — no wasm needed. Do it here so this one bin grades every outcome kind (the gate's grade_trial
        // for the Declined arm, ported).
        match &trial.expect {
            GExpect::Error(code, msg) => {
                worst = worst.worse(grade_compile_error(compiled, compile_diag, code, msg.as_deref()));
                if matches!(worst, Grade::Fail(_)) {
                    break;
                }
                continue;
            }
            GExpect::Declines(msg) => {
                worst = worst.worse(grade_compile_declines(compiled, compile_diag, msg.as_deref()));
                if matches!(worst, Grade::Fail(_)) {
                    break;
                }
                continue;
            }
            GExpect::Output(_) | GExpect::Trap(_) => {}
        }

        // RUN outcome (output/trap). If the compiler DECLINED a value/trap case, the gate grades it Todo (a
        // not-yet-implemented feature), never Fail — match that, and there is no wasm to run.
        if !compiled {
            worst = worst.worse(Grade::Todo(
                "output/trap case the compiler declined (not-yet-implemented; todo like the gate)".into(),
            ));
            continue;
        }
        let Some(component_bytes) = component_bytes else {
            anyhow::bail!("grade: an output/trap case compiled (status 0) but no component was supplied");
        };
        ran_a_trial = true;

        let export = match (&trial.call, component_name) {
            (Some(c), Some(iface)) => Some(format!("{iface}#{}", c.export)),
            (Some(c), None) => Some(c.export.clone()),
            (None, _) => None, // invoke the sole export
        };
        let args = trial
            .call
            .as_ref()
            .map(|c| c.args.clone())
            .unwrap_or_default();
        let opts = RunOpts {
            export,
            args,
            runtime: runtime.clone(),
            runtime_cache_dir: runtime_cache_dir.clone(),
            host_responses: host_responses.clone(),
        };
        let (outcome, observed) = run_capturing(component_bytes, &opts)?;
        if first_observed.is_none() && matches!(outcome, Outcome::Value(_)) {
            first_observed = Some(observed);
        }

        worst = worst.worse(grade_trial(&trial.expect, &outcome));
        if matches!(worst, Grade::Fail(_)) {
            break; // a single failing trial fails the case
        }
    }

    // WARNS (orthogonal presence check): a case may pin warnings the compile must emit. Only checkable on
    // a clean compile (warnings live in the compiler's stderr) — the gate grades warns on the wasm target
    // for a compiles-clean program; mirror that. Each pinned (code, msg?) must match some emitted warning.
    if !matches!(worst, Grade::Fail(_)) && !test_run.warns.is_empty() && compiled {
        let emitted = collect_warnings(compile_diag);
        for (code, msg) in &test_run.warns {
            let hit = emitted
                .iter()
                .any(|(c, m)| c == code && msg.as_deref().is_none_or(|p| m.contains(p)));
            if !hit {
                worst = Grade::Fail(format!(
                    "expected warning {code}{} not emitted; got {emitted:?}",
                    msg.as_deref().map(|p| format!(" (message ~ {p:?})")).unwrap_or_default()
                ));
                break;
            }
        }
    }

    // Host-call SEQUENCE check (only when every trial so far passed/todo): exact ordered equality against
    // the observed calls of the first value-producing trial — a dropped/extra/reordered call is a Fail.
    // `run_capturing` returns each observed entry as `<op>` OR `<op>\t<message>` (a call that carried a
    // STRING argument rides along with its message). The recorded `host-calls` are OPS ONLY, so compare on
    // the op alone — split on the FIRST tab and take the op, exactly as the `xtask gate`'s
    // `observed_host_calls` does. (Without this, a mixed-arity op with a string arg — `io.log("tag")` —
    // observes as `io.log\ttag` and spuriously mismatches the recorded `io.log`.)
    if !matches!(worst, Grade::Fail(_)) && !test_run.host_calls.is_empty() {
        let observed: Vec<String> = first_observed
            .unwrap_or_default()
            .iter()
            .map(|e| e.split('\t').next().unwrap_or(e).to_string())
            .collect();
        if observed != test_run.host_calls {
            worst = Grade::Fail(format!(
                "host-call sequence mismatch: expected {:?}, observed {:?}",
                test_run.host_calls, observed
            ));
        }
    }

    let desc = &test_run.description;
    match worst {
        Grade::Pass if ran_a_trial => {
            println!("PASS\t{desc}");
            Ok(ExitCode::SUCCESS)
        }
        Grade::Pass => {
            // No runnable trial — a pure error/declines case, graded entirely at build.
            println!("PASS (build-graded, no run-time trial)\t{desc}");
            Ok(ExitCode::SUCCESS)
        }
        Grade::Todo(why) => {
            println!("TODO\t{desc}\t{why}");
            Ok(ExitCode::SUCCESS)
        }
        Grade::Fail(why) => {
            println!("FAIL\t{desc}\t{why}");
            eprintln!("grade: FAIL: {desc}: {why}");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Grade one trial's outcome against its expectation — the value/trap half of the gate's `grade_trial`,
/// ported verbatim (string-exact value compare against the bare value OR the full `(: v T)` form;
/// canonical-`trap_kind` trap match; a trap whose reason maps to no kind is `Todo`, never a false pass).
fn grade_trial(expect: &GExpect, outcome: &Outcome) -> Grade {
    match expect {
        GExpect::Output(payload) => {
            let expected_val = expected_value(payload);
            let expected_full = payload.trim().to_string();
            match outcome {
                Outcome::Value(v) if *v == expected_val || *v == expected_full => Grade::Pass,
                Outcome::Value(v) => {
                    Grade::Fail(format!("expected output {payload}, got value {v}"))
                }
                Outcome::Trap(t) => {
                    Grade::Fail(format!("expected output {payload}, but trapped: {t}"))
                }
            }
        }
        GExpect::Trap(reason) => match outcome {
            Outcome::Trap(actual) => match (trap_kind(reason), trap_kind(actual)) {
                (Some(want), Some(got)) if want == got => Grade::Pass,
                _ => Grade::Todo(format!(
                    "trapped ({actual}) but reason kind ≠ expected ({reason})"
                )),
            },
            Outcome::Value(v) => Grade::Fail(format!(
                "expected trap {reason}, got value {v} (miscompile)"
            )),
        },
        // Not reached (compile-outcome expectations are graded before the run), but total for safety.
        GExpect::Error(..) | GExpect::Declines(..) => {
            Grade::Todo("compile-outcome expectation is graded from the diagnostic, not the run".into())
        }
    }
}

/// Grade an `(expect-error CODE msg?)` against the compile outcome — the gate's `grade_trial` error arm,
/// ported. Running an ill-formed program (compiled) is a Fail (miscompile); the right CODE (+ optional
/// message substring) is a Pass; a DIFFERENT code is Todo (the program was still refused).
fn grade_compile_error(compiled: bool, diag: &str, want: &str, msg: Option<&str>) -> Grade {
    if compiled {
        return Grade::Fail(format!(
            "expected compile error {want} but the program COMPILED (miscompile)"
        ));
    }
    let (got, message) = first_error_diag(diag);
    match got {
        Some(code) if code == want => match msg {
            None => Grade::Pass,
            Some(p) if message.contains(p) => Grade::Pass,
            Some(p) => Grade::Fail(format!("error {want} but message {message:?} lacks {p:?}")),
        },
        _ => Grade::Todo(format!("refused, but not with {want} (got {got:?})")),
    }
}

/// Grade an `(expect-declines msg?)` against the compile outcome — the gate's declines arm. ANY refusal
/// passes (coded or codeless); a compiled program is a Fail; the optional message must appear.
fn grade_compile_declines(compiled: bool, diag: &str, msg: Option<&str>) -> Grade {
    if compiled {
        return Grade::Fail("expected the compiler to DECLINE but it COMPILED (miscompile)".into());
    }
    match msg {
        None => Grade::Pass,
        Some(p) => {
            let (_, message) = first_error_diag(diag);
            if message.contains(p) {
                Grade::Pass
            } else {
                Grade::Fail(format!("declined, but message {message:?} lacks {p:?}"))
            }
        }
    }
}

/// The FIRST error diagnostic in a compiler stderr as `(code, message)` — `error [CODE] (node N): msg`
/// (coded) or `error: msg` (codeless). Ported verbatim from the `xtask gate` (`first_error_diag`).
fn first_error_diag(diag: &str) -> (Option<String>, String) {
    for line in diag.lines() {
        if let Some((_, after)) = line.split_once("error [")
            && let Some((code, rest)) = after.split_once(']')
            && !code.trim().is_empty()
        {
            let message = rest.split_once(": ").map(|(_, m)| m).unwrap_or("").trim();
            return (Some(code.trim().to_string()), message.to_string());
        }
        if !line.contains("error [")
            && let Some((_, msg)) = line.split_once("error: ")
        {
            return (None, msg.trim().to_string());
        }
    }
    (None, String::new())
}

/// EVERY `warning [CODE] (node N): message` in a compiler stderr, as `(code, message)` — a clean compile
/// can emit a SET. Ported verbatim from the `xtask gate` (`collect_warnings`).
fn collect_warnings(diag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in diag.lines() {
        if let Some((_, after)) = line.split_once("warning [")
            && let Some((code, rest)) = after.split_once(']')
            && !code.trim().is_empty()
        {
            let message = rest.split_once(": ").map(|(_, m)| m).unwrap_or("").trim();
            out.push((code.trim().to_string(), message.to_string()));
        }
    }
    out
}

/// Decode a `(test-run …)` binary AST into a [`TestRun`]. Mirrors the shred's `test_run_ast` builder
/// (`cdz-corpus`): every text field is a string LEAF, so each is read opaquely.
fn decode_test_run(bytes: &[u8]) -> Result<TestRun> {
    let a = codec::decode(bytes).ok_or_else(|| anyhow::anyhow!("test-run.ast failed to decode"))?;
    let root = a.root;
    if a.head_name(root) != Some("test-run") {
        anyhow::bail!("not a (test-run …) form");
    }
    let mut description = String::new();
    let mut trials = Vec::new();
    let mut host_responses = Vec::new();
    let mut host_calls = Vec::new();
    let mut warns = Vec::new();

    for &clause in children(&a, root) {
        match a.head_name(clause) {
            Some("description") => {
                description = a
                    .as_form(clause, "description")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| str_leaf(&a, id))
                    .unwrap_or_default();
            }
            Some("trials") => {
                for &t in a.as_form(clause, "trials").unwrap_or(&[]) {
                    if let Some(trial) = decode_trial(&a, t) {
                        trials.push(trial);
                    }
                }
            }
            Some("host-responses") => {
                for &r in a.as_form(clause, "host-responses").unwrap_or(&[]) {
                    if let Some(pair) = a.as_form(r, "response")
                        && let (Some(op), Some(v)) = (
                            pair.first().and_then(|&id| str_leaf(&a, id)),
                            pair.get(1).and_then(|&id| str_leaf(&a, id)),
                        )
                    {
                        host_responses.push((op, v));
                    }
                }
            }
            Some("host-calls") => {
                for &c in a.as_form(clause, "host-calls").unwrap_or(&[]) {
                    if let Some(op) = a
                        .as_form(c, "op")
                        .and_then(|t| t.first().copied())
                        .and_then(|id| str_leaf(&a, id))
                    {
                        host_calls.push(op);
                    }
                }
            }
            Some("warns") => {
                for &w in a.as_form(clause, "warns").unwrap_or(&[]) {
                    if let Some(t) = a.as_form(w, "warn")
                        && let Some(code) = t.first().copied().and_then(|id| str_leaf(&a, id))
                    {
                        let msg = t.get(1).copied().and_then(|id| str_leaf(&a, id));
                        warns.push((code, msg));
                    }
                }
            }
            _ => {}
        }
    }

    Ok(TestRun {
        description,
        trials,
        host_responses,
        host_calls,
        warns,
    })
}

/// Decode one `(trial (call export)? (arg v)* <expect>)` form.
fn decode_trial(a: &Arenas, id: StructId) -> Option<GTrial> {
    let items = a.as_form(id, "trial")?;
    let mut export: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    let mut expect: Option<GExpect> = None;
    for &child in items {
        match a.head_name(child) {
            Some("call") => {
                export = a
                    .as_form(child, "call")
                    .and_then(|t| t.first().copied())
                    .and_then(|cid| str_leaf(a, cid));
            }
            Some("arg") => {
                if let Some(v) = a
                    .as_form(child, "arg")
                    .and_then(|t| t.first().copied())
                    .and_then(|aid| str_leaf(a, aid))
                {
                    args.push(v);
                }
            }
            Some("expect-output") => {
                expect = a
                    .as_form(child, "expect-output")
                    .and_then(|t| t.first().copied())
                    .and_then(|vid| str_leaf(a, vid))
                    .map(GExpect::Output);
            }
            Some("expect-trap") => {
                expect = a
                    .as_form(child, "expect-trap")
                    .and_then(|t| t.first().copied())
                    .and_then(|vid| str_leaf(a, vid))
                    .map(GExpect::Trap);
            }
            Some("expect-error") => {
                // `(expect-error CODE msg?)` — first leaf is the code, an optional second is the message.
                if let Some(t) = a.as_form(child, "expect-error") {
                    let code = t.first().copied().and_then(|id| str_leaf(a, id));
                    let msg = t.get(1).copied().and_then(|id| str_leaf(a, id));
                    if let Some(code) = code {
                        expect = Some(GExpect::Error(code, msg));
                    }
                }
            }
            Some("expect-declines") => {
                // `(expect-declines msg?)` — optional message leaf.
                let msg = a
                    .as_form(child, "expect-declines")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| str_leaf(a, id));
                expect = Some(GExpect::Declines(msg));
            }
            _ => {}
        }
    }
    let call = export.map(|export| GCall { export, args });
    Some(GTrial {
        call,
        expect: expect?,
    })
}

/// A form's children after the head (`items[1..]`), or empty for a non-list / headless node.
fn children(a: &Arenas, id: StructId) -> &[StructId] {
    match a.get(id) {
        Struct::List(items) if !items.is_empty() => &items[1..],
        _ => &[],
    }
}

/// Read a string-leaf node's text, or `None` for a non-string node.
fn str_leaf(a: &Arenas, id: StructId) -> Option<String> {
    match a.get(id) {
        Struct::Atom(lid) => match a.leaf(*lid) {
            cadenza_syntax::ast::Leaf::Str(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// The value out of an `output` payload `(: <value> <Type>)` — the text of `<value>`. Ported verbatim
/// from the corpus gate (`xtask`): balanced-paren / quoted-string aware, so a compound value/type
/// (`(: (tuple 0 7) (Tuple Int64 Int64))`) or a string value (`(: "parse error" String)`) is not miscut.
fn expected_value(payload: &str) -> String {
    let inner = payload.trim();
    let Some(rest) = inner.strip_prefix("(:") else {
        return inner.to_string();
    };
    let rest = rest.trim();
    let bytes = rest.as_bytes();
    if bytes.first() == Some(&b'(') {
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
    } else if bytes.first() == Some(&b'"') {
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
    } else {
        match rest.find(char::is_whitespace) {
            Some(idx) => rest[..idx].to_string(),
            None => rest.trim_end_matches(')').to_string(),
        }
    }
}

/// Map a trap reason to its canonical KIND, reconciling corpus vocabulary vs the wasmtime reason.
/// Ported verbatim from the corpus gate (`xtask`) so a `(trap …)` case grades identically here.
fn trap_kind(reason: &str) -> Option<&'static str> {
    let r = reason.to_ascii_lowercase();
    if r.contains("divide by zero")
        || r.contains("division by zero")
        || r.contains("remainder by zero")
    {
        Some("div-by-zero")
    } else if r.contains("out of bounds") || r.contains("out-of-bounds") {
        Some("out-of-bounds")
    } else if r.contains("overflow") {
        Some("overflow")
    } else if r.contains("unreachable") || r.contains("shift count out of range") {
        Some("unreachable")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_value_extracts_bare_scalar_compound_and_string() {
        assert_eq!(expected_value("(: 42 Int64)"), "42");
        assert_eq!(
            expected_value("(: (tuple 0 7) (Tuple Int64 Int64))"),
            "(tuple 0 7)"
        );
        assert_eq!(
            expected_value("(: \"parse error\" String)"),
            "\"parse error\""
        );
        // Not the (: v T) shape → whole payload.
        assert_eq!(expected_value("bare"), "bare");
    }

    #[test]
    fn trap_kind_canonicalizes_backend_vocabulary() {
        assert_eq!(trap_kind("integer divide by zero"), Some("div-by-zero"));
        assert_eq!(trap_kind("remainder by zero"), Some("div-by-zero"));
        assert_eq!(
            trap_kind("out of bounds memory access"),
            Some("out-of-bounds")
        );
        assert_eq!(trap_kind("integer overflow"), Some("overflow"));
        assert_eq!(
            trap_kind("wasm 'unreachable' instruction executed"),
            Some("unreachable")
        );
        assert_eq!(trap_kind("shift count out of range"), Some("unreachable"));
        assert_eq!(trap_kind("something else"), None);
    }

    #[test]
    fn grade_trial_output_matches_bare_or_full_form() {
        let g = grade_trial(
            &GExpect::Output("(: 42 Int64)".into()),
            &Outcome::Value("42".into()),
        );
        assert_eq!(g, Grade::Pass);
        // A compound crosses as the full form.
        let g = grade_trial(
            &GExpect::Output("(: (record (= d (None unit))) (record (d (Option Int64))))".into()),
            &Outcome::Value("(: (record (= d (None unit))) (record (d (Option Int64))))".into()),
        );
        assert_eq!(g, Grade::Pass);
        // Wrong value fails.
        assert!(matches!(
            grade_trial(
                &GExpect::Output("(: 42 Int64)".into()),
                &Outcome::Value("43".into())
            ),
            Grade::Fail(_)
        ));
        // Expected value but trapped = fail.
        assert!(matches!(
            grade_trial(
                &GExpect::Output("(: 42 Int64)".into()),
                &Outcome::Trap("overflow".into())
            ),
            Grade::Fail(_)
        ));
    }

    #[test]
    fn grade_trial_trap_matches_by_kind_and_flags_value_as_miscompile() {
        assert_eq!(
            grade_trial(
                &GExpect::Trap("division by zero".into()),
                &Outcome::Trap("integer divide by zero".into())
            ),
            Grade::Pass
        );
        // Trapped but reason maps to no kind → Todo (never a false pass).
        assert!(matches!(
            grade_trial(
                &GExpect::Trap("weird".into()),
                &Outcome::Trap("odd reason".into())
            ),
            Grade::Todo(_)
        ));
        // Expected trap, got a value = miscompile.
        assert!(matches!(
            grade_trial(
                &GExpect::Trap("overflow".into()),
                &Outcome::Value("7".into())
            ),
            Grade::Fail(_)
        ));
    }

    #[test]
    fn decode_reads_the_shred_test_run_shape() {
        // Build a (test-run …) with the same builder the shred uses, via cadenza-syntax.
        use cadenza_syntax::ast::{Builder, Leaf};
        use std::sync::Arc;
        let mut b = Builder::new();
        let s = |b: &mut Builder, t: &str| b.atom_leaf(Leaf::Str(Arc::from(t)));
        let head = b.name("test-run");
        let dh = b.name("description");
        let dv = s(&mut b, "a case");
        let desc = b.list(vec![dh, dv]);
        // one trial: (trial (call "main") (arg "41") (expect-output "(: 42 Int64)"))
        let th = b.name("trial");
        let ch = b.name("call");
        let cv = s(&mut b, "main");
        let call = b.list(vec![ch, cv]);
        let ah = b.name("arg");
        let av = s(&mut b, "41");
        let arg = b.list(vec![ah, av]);
        let eh = b.name("expect-output");
        let ev = s(&mut b, "(: 42 Int64)");
        let expect = b.list(vec![eh, ev]);
        let trial = b.list(vec![th, call, arg, expect]);
        let trials_head = b.name("trials");
        let trials = b.list(vec![trials_head, trial]);
        let root = b.list(vec![head, desc, trials]);
        let bytes = codec::encode(&b.finish(root));

        let tr = decode_test_run(&bytes).expect("decodes");
        assert_eq!(tr.description, "a case");
        assert_eq!(tr.trials.len(), 1);
        let t = &tr.trials[0];
        let c = t.call.as_ref().expect("call");
        assert_eq!(c.export, "main");
        assert_eq!(c.args, vec!["41".to_string()]);
        assert!(matches!(&t.expect, GExpect::Output(v) if v == "(: 42 Int64)"));
    }
}
