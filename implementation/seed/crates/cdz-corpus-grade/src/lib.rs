//! Grade a run/compile OUTCOME against a shredded `test-run.ast` — the corpus grade compare, shared by
//! the wasm (`cdz-run`) and rust (`cdz-rust-run`) exec backends. The shred writes one `test-run.ast` per
//! case (`cdz corpus records --out-dir`): description, trials (each an optional `(call export)` +
//! `(arg …)`s + an expected outcome), a host-response tape, the expected host-call sequence, and pinned
//! `(warns …)`. This crate decodes it and grades an outcome EXACTLY as `cargo xtask gate` does — value
//! string-match (bare or full `(: v T)` form), canonical `trap_kind`, exact error-code + message
//! substring, warns presence, ordered host-call sequence.
//!
//! The grade is IDENTICAL across backends; only HOW a runnable trial produces an [`Outcome`] differs (the
//! wasm run vs the rust compile+run). So [`grade_run`] is the whole orchestration, parameterized on a
//! per-backend trial-runner closure — no wasmtime, no compiler, in this crate.

use std::process::ExitCode;

use anyhow::Result;
use cadenza_syntax::ast::{Arenas, Struct, StructId};
use cadenza_syntax::codec;

/// What running a trial produced — the backend-independent outcome the grade compares against.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The export returned; its value rendered to canonical text, plus the observed host-call op sequence
    /// (`host-call\t<op>` lines, in call order; empty for a non-host program).
    Value(String, Vec<String>),
    /// The export TRAPPED at run time (the trap reason).
    Trap(String),
    /// The emitted artifact did not build / would not run (the reason) — a value/trap case with this is a
    /// miscompile (Fail).
    BadArtifact(String),
}

/// A per-case grade verdict — the three-way the corpus gate uses. `Todo` is a run that happened but could
/// not be classified pass/fail (a real trap whose reason maps to no canonical kind, or a not-yet-compilable
/// value case) — NOT a failure; only `Fail` fails the exec derivation.
#[derive(Debug, PartialEq)]
pub enum Grade {
    Pass,
    Todo(String),
    Fail(String),
}

impl Grade {
    /// Combine verdicts, keeping the WORST (Fail > Todo > Pass) — the case verdict is the worst of its
    /// trials + checks, matching the gate's `grade_ran`.
    pub fn worse(self, other: Grade) -> Grade {
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
pub struct GTrial {
    pub call: Option<GCall>,
    pub expect: GExpect,
}

pub struct GCall {
    pub export: String,
    pub args: Vec<String>,
}

/// The expected outcome of a trial. `Output`/`Trap` are RUN outcomes (graded against the run); `Error`/
/// `Declines` are COMPILE outcomes (graded against the captured compiler diagnostic).
pub enum GExpect {
    Output(String),
    Trap(String),
    /// `(expect-error CODE msg?)` — the compiler must REFUSE with exactly `CODE` (+ optional message substring).
    Error(String, Option<String>),
    /// `(expect-declines msg?)` — the compiler must refuse (any code, or codeless); optional message substring.
    Declines(Option<String>),
}

/// A decoded `test-run.ast`: the case's run/grade metadata.
pub struct TestRun {
    pub description: String,
    pub trials: Vec<GTrial>,
    /// The recorded host-call response tape, `(op, value)` in call order.
    pub host_responses: Vec<(String, String)>,
    /// The expected observed host-call op sequence.
    pub host_calls: Vec<String>,
    /// Pinned warnings `(code, optional message-substring)` — a PRESENCE check against the compile diag.
    pub warns: Vec<(String, Option<String>)>,
}

/// The combined grade of a case + whether any runnable trial actually ran (a pure error/declines case runs
/// none — its verdict is graded entirely from the compile outcome).
pub struct GradeResult {
    pub grade: Grade,
    pub ran_a_trial: bool,
}

/// Grade a whole case: decode is the caller's (it has the bytes); this orchestrates the trials + checks,
/// calling `run_trial` for each RUNNABLE (output/trap, compiled) trial to obtain its [`Outcome`]. Compile
/// outcomes (error/declines) + warns are graded from `compile_status`/`compile_diag` (no run). Reproduces
/// the gate's `grade_ran`: the worst of every trial + the host-call-sequence + the warns checks.
pub fn grade_run<F>(
    test_run: &TestRun,
    compile_status: i32,
    compile_diag: &str,
    mut run_trial: F,
) -> Result<GradeResult>
where
    F: FnMut(&GTrial) -> Result<Outcome>,
{
    let compiled = compile_status == 0;
    let mut worst = Grade::Pass;
    let mut ran_a_trial = false;
    // The observed host-call sequence of the first value-producing trial — the gate checks host_calls
    // against exactly this (a compiled program's host effects are the same on every trial).
    let mut first_observed: Option<Vec<String>> = None;

    for trial in &test_run.trials {
        // COMPILE-OUTCOME expectations (error/declines) are graded against the captured diagnostic, no run.
        match &trial.expect {
            GExpect::Error(code, msg) => {
                worst = worst.worse(grade_compile_error(
                    compiled,
                    compile_diag,
                    code,
                    msg.as_deref(),
                ));
                if matches!(worst, Grade::Fail(_)) {
                    break;
                }
                continue;
            }
            GExpect::Declines(msg) => {
                worst = worst.worse(grade_compile_declines(
                    compiled,
                    compile_diag,
                    msg.as_deref(),
                ));
                if matches!(worst, Grade::Fail(_)) {
                    break;
                }
                continue;
            }
            GExpect::Output(_) | GExpect::Trap(_) => {}
        }

        // RUN outcome. If the compiler DECLINED a value/trap case, the gate grades it Todo (a
        // not-yet-implemented feature), never Fail — and there is nothing to run.
        if !compiled {
            worst = worst.worse(Grade::Todo(
                "output/trap case the compiler declined (not-yet-implemented; todo like the gate)"
                    .into(),
            ));
            continue;
        }
        ran_a_trial = true;
        let outcome = run_trial(trial)?;
        if first_observed.is_none()
            && let Outcome::Value(_, observed) = &outcome
        {
            first_observed = Some(observed.clone());
        }
        worst = worst.worse(grade_trial(&trial.expect, &outcome));
        if matches!(worst, Grade::Fail(_)) {
            break;
        }
    }

    // WARNS (orthogonal presence check) — only checkable on a clean compile.
    if !matches!(worst, Grade::Fail(_)) && !test_run.warns.is_empty() && compiled {
        let emitted = collect_warnings(compile_diag);
        for (code, msg) in &test_run.warns {
            let hit = emitted
                .iter()
                .any(|(c, m)| c == code && msg.as_deref().is_none_or(|p| m.contains(p)));
            if !hit {
                worst = Grade::Fail(format!(
                    "expected warning {code}{} not emitted; got {emitted:?}",
                    msg.as_deref()
                        .map(|p| format!(" (message ~ {p:?})"))
                        .unwrap_or_default()
                ));
                break;
            }
        }
    }

    // Host-call SEQUENCE check — the observed calls of the first value-producing trial must equal the
    // recorded ops exactly (ordered). `run_trial` returns each observed entry as `<op>` OR `<op>\t<message>`
    // (a call carrying a string arg), so compare on the op alone (split on the first tab), as the gate does.
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

    Ok(GradeResult {
        grade: worst,
        ran_a_trial,
    })
}

/// Print the case verdict to stdout (a `Fail` reason also to stderr) and return the process exit code —
/// `0` for Pass/Todo, `1` for Fail. Shared by both exec bins.
pub fn print_verdict(result: &GradeResult, description: &str) -> ExitCode {
    match &result.grade {
        Grade::Pass if result.ran_a_trial => {
            println!("PASS\t{description}");
            ExitCode::SUCCESS
        }
        Grade::Pass => {
            println!("PASS (build-graded, no run-time trial)\t{description}");
            ExitCode::SUCCESS
        }
        Grade::Todo(why) => {
            println!("TODO\t{description}\t{why}");
            ExitCode::SUCCESS
        }
        Grade::Fail(why) => {
            println!("FAIL\t{description}\t{why}");
            eprintln!("grade: FAIL: {description}: {why}");
            ExitCode::FAILURE
        }
    }
}

/// Grade one trial's RUN outcome against its expectation — string-exact value compare against the bare
/// value OR the full `(: v T)` form; canonical-`trap_kind` trap match (an unclassifiable trap is `Todo`,
/// never a false pass); a value where a trap was expected (or vice-versa) is a `Fail` (miscompile); a
/// `BadArtifact` on a value/trap case is a `Fail` (the emit did not build/run).
pub fn grade_trial(expect: &GExpect, outcome: &Outcome) -> Grade {
    match expect {
        GExpect::Output(payload) => {
            let expected_val = expected_value(payload);
            let expected_full = payload.trim().to_string();
            match outcome {
                Outcome::Value(v, _) if *v == expected_val || *v == expected_full => Grade::Pass,
                Outcome::Value(v, _) => {
                    Grade::Fail(format!("expected output {payload}, got value {v}"))
                }
                Outcome::Trap(t) => {
                    Grade::Fail(format!("expected output {payload}, but trapped: {t}"))
                }
                Outcome::BadArtifact(e) => Grade::Fail(format!(
                    "expected output {payload}, but the artifact did not build: {e}"
                )),
            }
        }
        GExpect::Trap(reason) => match outcome {
            Outcome::Trap(actual) => match (trap_kind(reason), trap_kind(actual)) {
                (Some(want), Some(got)) if want == got => Grade::Pass,
                _ => Grade::Todo(format!(
                    "trapped ({actual}) but reason kind ≠ expected ({reason})"
                )),
            },
            Outcome::Value(v, _) => Grade::Fail(format!(
                "expected trap {reason}, got value {v} (miscompile)"
            )),
            Outcome::BadArtifact(e) => Grade::Fail(format!(
                "expected trap {reason}, but the artifact did not build: {e}"
            )),
        },
        // Not reached (compile-outcome expectations are graded before the run), but total for safety.
        GExpect::Error(..) | GExpect::Declines(..) => Grade::Todo(
            "compile-outcome expectation is graded from the diagnostic, not the run".into(),
        ),
    }
}

/// Grade an `(expect-error CODE msg?)` against the compile outcome — running an ill-formed program is a
/// Fail (miscompile); the right CODE (+ optional message substring) is a Pass; a DIFFERENT code is Todo.
pub fn grade_compile_error(compiled: bool, diag: &str, want: &str, msg: Option<&str>) -> Grade {
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

/// Grade an `(expect-declines msg?)` — ANY refusal passes (coded or codeless); a compiled program is a
/// Fail; the optional message substring must appear.
pub fn grade_compile_declines(compiled: bool, diag: &str, msg: Option<&str>) -> Grade {
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
/// (coded) or `error: msg` (codeless). Ported verbatim from the `xtask gate`.
pub fn first_error_diag(diag: &str) -> (Option<String>, String) {
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

/// EVERY `warning [CODE] (node N): message` in a compiler stderr — a clean compile can emit a SET.
pub fn collect_warnings(diag: &str) -> Vec<(String, String)> {
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

/// Decode a `(test-run …)` binary AST into a [`TestRun`]. Every text field is a string LEAF (read opaquely).
pub fn decode_test_run(bytes: &[u8]) -> Result<TestRun> {
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
                if let Some(t) = a.as_form(child, "expect-error") {
                    let code = t.first().copied().and_then(|id| str_leaf(a, id));
                    let msg = t.get(1).copied().and_then(|id| str_leaf(a, id));
                    if let Some(code) = code {
                        expect = Some(GExpect::Error(code, msg));
                    }
                }
            }
            Some("expect-declines") => {
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
/// from the corpus gate: balanced-paren / quoted-string aware, so a compound value/type
/// (`(: (tuple 0 7) (Tuple Int64 Int64))`) or a string value (`(: "parse error" String)`) is not miscut.
pub fn expected_value(payload: &str) -> String {
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

/// Map a trap reason to its canonical KIND, reconciling corpus vocabulary vs the backend reason. Ported
/// verbatim from the `xtask gate` so a `(trap …)` case grades identically on wasm and rust.
pub fn trap_kind(reason: &str) -> Option<&'static str> {
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
        assert_eq!(
            grade_trial(
                &GExpect::Output("(: 42 Int64)".into()),
                &Outcome::Value("42".into(), vec![])
            ),
            Grade::Pass
        );
        assert!(matches!(
            grade_trial(
                &GExpect::Output("(: 42 Int64)".into()),
                &Outcome::Value("43".into(), vec![])
            ),
            Grade::Fail(_)
        ));
        assert!(matches!(
            grade_trial(
                &GExpect::Output("(: 42 Int64)".into()),
                &Outcome::BadArtifact("boom".into())
            ),
            Grade::Fail(_)
        ));
    }

    #[test]
    fn grade_trial_trap_by_kind_and_miscompile() {
        assert_eq!(
            grade_trial(
                &GExpect::Trap("division by zero".into()),
                &Outcome::Trap("integer divide by zero".into())
            ),
            Grade::Pass
        );
        assert!(matches!(
            grade_trial(
                &GExpect::Trap("overflow".into()),
                &Outcome::Value("7".into(), vec![])
            ),
            Grade::Fail(_)
        ));
        assert!(matches!(
            grade_trial(&GExpect::Trap("weird".into()), &Outcome::Trap("odd".into())),
            Grade::Todo(_)
        ));
    }

    #[test]
    fn compile_error_grade() {
        assert_eq!(
            grade_compile_error(false, "cdz: error [CDZ0201] (node 4): sep", "CDZ0201", None),
            Grade::Pass
        );
        // Different code → Todo (still refused). Compiled → Fail.
        assert!(matches!(
            grade_compile_error(false, "error [CDZ0300]: x", "CDZ0201", None),
            Grade::Todo(_)
        ));
        assert!(matches!(
            grade_compile_error(true, "", "CDZ0201", None),
            Grade::Fail(_)
        ));
    }

    #[test]
    fn grade_run_orchestrates_an_output_trial() {
        // Build a (test-run …) with one output trial, decode it, grade with a stub runner.
        use cadenza_syntax::ast::{Builder, Leaf};
        use std::sync::Arc;
        let mut b = Builder::new();
        let s = |b: &mut Builder, t: &str| b.atom_leaf(Leaf::Str(Arc::from(t)));
        let head = b.name("test-run");
        let dh = b.name("description");
        let dv = s(&mut b, "case");
        let desc = b.list(vec![dh, dv]);
        let th = b.name("trial");
        let eh = b.name("expect-output");
        let ev = s(&mut b, "(: 42 Int64)");
        let expect = b.list(vec![eh, ev]);
        let trial = b.list(vec![th, expect]);
        let trials_head = b.name("trials");
        let trials = b.list(vec![trials_head, trial]);
        let root = b.list(vec![head, desc, trials]);
        let bytes = codec::encode(&b.finish(root));

        let tr = decode_test_run(&bytes).expect("decodes");
        let res = grade_run(&tr, 0, "", |_| Ok(Outcome::Value("42".into(), vec![]))).unwrap();
        assert_eq!(res.grade, Grade::Pass);
        assert!(res.ran_a_trial);
        // A wrong value → Fail.
        let res = grade_run(&tr, 0, "", |_| Ok(Outcome::Value("41".into(), vec![]))).unwrap();
        assert!(matches!(res.grade, Grade::Fail(_)));
    }
}
