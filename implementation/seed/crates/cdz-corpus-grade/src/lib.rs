//! Grade a run/compile OUTCOME against a shredded `test-run.ast` — the corpus grade compare, shared by
//! the wasm (`cdz-run`) and rust (`cdz-rust-run`) exec backends. The shred writes one `test-run.ast` per
//! case (`cdz corpus records --out-dir`): description, trials (each an optional `(call export)` +
//! `(arg …)`s + an expected outcome), a host-response tape, the expected host-call sequence, and pinned
//! `(warns …)`. This crate decodes it and grades an outcome EXACTLY as `cargo xtask gate` does — value
//! string-match (bare or full `(: v T)` form), canonical `TrapCode`, exact error-code + message
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

    /// The payload-free verdict KIND, for the baseline compare (`<verdict>\t<description>`).
    pub fn verdict(&self) -> Verdict {
        match self {
            Grade::Pass => Verdict::Pass,
            Grade::Todo(_) => Verdict::Todo,
            Grade::Fail(_) => Verdict::Fail,
        }
    }
}

/// The three-way verdict KIND (no payload), the vocabulary of the committed `.gate-baseline*` files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Todo,
    Fail,
}

impl Verdict {
    /// Parse a baseline verdict tag (`pass`/`todo`/`fail`), matching the xtask gate's `Verdict::parse`.
    pub fn parse(s: &str) -> Option<Verdict> {
        match s.trim() {
            "pass" => Some(Verdict::Pass),
            "todo" => Some(Verdict::Todo),
            "fail" => Some(Verdict::Fail),
            _ => None,
        }
    }
}

/// The baseline verdict recorded for `description` in a `.gate-baseline*` file — its lines are
/// `<verdict>\t<description>` (`#`/blank lines skipped). `None` if the description is absent. On a
/// duplicate description the LAST line wins, matching the xtask gate's map-load (`base.insert`); a
/// CONFLICTING duplicate is a baseline-integrity issue handled by `cargo xtask gate --save`/canonicalize,
/// not by this per-case lookup.
pub fn baseline_verdict(baseline: &str, description: &str) -> Option<Verdict> {
    let mut found = None;
    for line in baseline.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((v, d)) = line.split_once('\t')
            && d == description
            && let Some(verdict) = Verdict::parse(v)
        {
            found = Some(verdict); // last-wins
        }
    }
    found
}

/// A REGRESSION vs the committed baseline for this case: the baseline recorded `pass` but the current run
/// did NOT pass — the exact `pass -> not-pass` rule the xtask gate's `check_baseline` fails on (a gained
/// case `not-pass -> pass`, a still-todo case, or a case absent from the baseline is NOT a regression).
/// Returns the failure message when regressed, else `None`. NOTE: "vanished" detection (a baseline case
/// with no corresponding run) needs a global view over all cases and is a separate aggregate, not this
/// per-case check.
pub fn check_regression(actual: Verdict, description: &str, baseline: &str) -> Option<String> {
    match baseline_verdict(baseline, description) {
        Some(Verdict::Pass) if actual != Verdict::Pass => Some(format!(
            "REGRESSION vs baseline: case {description:?} was pass, now {actual:?}"
        )),
        _ => None,
    }
}

/// The per-case exec EXIT CODE — prints the verdict, then decides pass/fail. WITHOUT a baseline, the exec
/// fails on any outright `Fail` (the miscompile check). WITH a baseline, it reproduces `xtask gate --check`
/// EXACTLY (main.rs `check_baseline`), failing on THREE conditions so the nix corpus check can be the
/// authoritative landing bar (so `--check` may delegate to it + drop the in-process grade):
///   (1) REGRESSION — a case the baseline recorded as `pass` that no longer passes (`pass → not-pass`);
///   (2) GATE-HOLE #3984 — a `todo`/absent case that now `Fail`s (baseline verdict NOT `pass` and NOT
///       `fail`): a real miscompile that the pass→not-pass rule alone would let slip past `--check`,
///       making the fleet bar strictly weaker than plain `gate`. Reds it, matching `check_baseline`'s
///       `failing` set.
/// A `fail`-baseline + `fail` verdict is the EXCEPTION — a TRACKED, git-committed known-fail (#4547): the
/// EXPECTED state, NOT a gate failure (noted so it stays visible; a later PASS surfaces as a regression to
/// re-baseline). The whole-run "vanished" case (a baseline title with no run) is a separate global aggregate.
pub fn exec_exit(result: &GradeResult, description: &str, baseline: Option<&str>) -> ExitCode {
    let printed = print_verdict(result, description);
    match baseline {
        None => printed,
        Some(bl) => {
            // (1) pass → not-pass regression.
            if let Some(msg) = check_regression(result.grade.verdict(), description, bl) {
                eprintln!("grade: {msg}");
                return ExitCode::FAILURE;
            }
            let bv = baseline_verdict(bl, description);
            if matches!(result.grade, Grade::Fail(_)) {
                // (2) #3984: a `todo`/absent (NOT pass, NOT fail) case that now FAILs reds — a miscompile
                //     the pass→not-pass rule misses. Matches `check_baseline`'s `failing` set.
                if !matches!(bv, Some(Verdict::Pass) | Some(Verdict::Fail)) {
                    eprintln!(
                        "grade: {description}: FAIL and baseline verdict is {bv:?} (todo/absent) — gate \
                         hole (xtask #3984): a non-pass, non-fail baseline that now fails reds `--check`"
                    );
                    return ExitCode::FAILURE;
                }
                // (3) #4547: a `fail` baseline + `fail` verdict is the EXPECTED tracked known-fail — exempt,
                //     but noted so the log stays honest + the pin stays visible.
                eprintln!(
                    "grade: {description}: FAIL, TRACKED known-fail (explicit `fail` baseline) — not a \
                     gate failure (a later pass surfaces as a regression to re-baseline)"
                );
            }
            ExitCode::SUCCESS
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
    /// A `(then …)` two-call continuation (the corpus `(then …)` clause): the SECOND call's args, or
    /// `None` for the ordinary one-call form. `Some` (possibly empty) drives the same borrowed closure
    /// handle twice, rendering the pair as a tuple — the grade-path analog of `--call-twice`.
    pub second_call: Option<Vec<String>>,
    /// A `(drop)` clause: resource-drop the minted handle after the call(s) before reading the heap
    /// balance (so a `(live-objects 0)` case pins release) — the grade-path analog of `--drop-handle`.
    pub drop_handle: bool,
    /// A `(call-method <member>)` clause: the NAMED value-resource member to reach (grade-path analog of
    /// `--call-member`). `None` = an ordinary call/escape; `Some` = no export, reach this member after make.
    pub method: Option<String>,
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
    /// The live-heap-cell count a case asserts after the run (the heap-balance invariant). Under the
    /// OPT-OUT model the absent-clause default depends on whether the component imports the value-heap
    /// runtime: a HEAP-importing case with `None` here enforces == 0 (the new default — no leak), a
    /// NO-HEAP case with `None` is skipped (no heap to balance, never a false fail). `Some(N)` asserts
    /// == N exactly (an explicit `(live-objects N)`, or a `(live-objects known-leak N)` marker — see
    /// `live_objects_known_leak`). The wasm exec runs a heap case on the debug-counters runtime (the
    /// shipped one reports 0 vacuously) and fails on a count mismatch.
    pub live_objects: Option<u32>,
    /// `true` iff the count above came from a `(live-objects known-leak N)` OPT-OUT MARKER — N is the
    /// TOLERATED current leak of a case not yet reclaim-clean (grandfathered when the opt-out default
    /// landed). Graded identically to a plain `(live-objects N)` (assert == N, which doubles as a
    /// regression guard: if the count drifts the case fails, forcing a deliberate marker update as the
    /// runtime reclaims). This flag only records the INTENT (a shrinking exception set), so tooling can
    /// find + retire markers; it does not change the assertion. `false` for `None` or a plain count.
    pub live_objects_known_leak: bool,
}

/// The combined grade of a case + whether any runnable trial actually ran (a pure error/declines case runs
/// none — its verdict is graded entirely from the compile outcome).
pub struct GradeResult {
    pub grade: Grade,
    pub ran_a_trial: bool,
}

/// Grade the post-run HEAP-BALANCE across ALL of a case's trials (the opt-out live-objects invariant).
///
/// `per_trial[i]` is trial `i`'s observed live-cell count: `Some(n)` when trial `i` imported the value-heap
/// runtime and ended at `n` live cells (a HEAP trial, run on the debug-counters runtime); `None` when the
/// trial imported no heap (a scalar/const program — nothing to balance). `expected` is the case's asserted
/// count (`None` = no `(live-objects …)` clause → the opt-out default of 0 = no leak / no double-free), and
/// it applies UNIFORMLY to every call.
///
/// Returns a `Fail` message for the FIRST heap trial whose balance ≠ `expected`, else `None`. The key fix
/// over the historical code: it checks EVERY call, not just call[0] — a multi-call case that balances on
/// call 0 but leaks on call 2 (or whose leak scales with call depth) is a real leak the corpus MUST catch.
/// The first-call-only capture silently FALSE-GREENED those leaks fleet-wide. No-heap trials are skipped
/// (never a false fail), so a mixed case is graded on its heap trials alone.
pub fn check_live_objects(per_trial: &[Option<u32>], expected: Option<u32>) -> Option<String> {
    let want = expected.unwrap_or(0);
    for (i, live) in per_trial.iter().enumerate() {
        if let Some(n) = live
            && *n != want
        {
            // A single-trial case keeps the historical (call-index-free) message so its verdict text stays
            // stable; a multi-call case names the offending call so a depth-scaling leak is legible.
            let has_multiple_heap_trials = per_trial.iter().filter(|l| l.is_some()).count() > 1;
            return Some(if has_multiple_heap_trials {
                format!("live-objects mismatch on call {i}: expected {want}, got {n}")
            } else {
                format!("live-objects mismatch: expected {want}, got {n}")
            });
        }
    }
    None
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

        // RUN outcome. A value/trap case whose compiler did NOT emit is graded from the DIAGNOSTIC, never run.
        // Most non-emit outcomes are HONEST not-yet-implemented declines → Todo (a coded `CDZxxxx` decline, OR a
        // code-less "type has no machine representation"-family decline — the canonical type-without-boundary
        // decline). But a CODE-LESS compile error whose message matches a curated INTERNAL-COMPILER-ERROR
        // signature (`is_ice_signature`: "no local slot", a self-labeled "compiler bug", a panic) is a compiler
        // BUG, not a capability gap, and must FAIL — never hide in the todo pool the all-declines scoreboard
        // audits (operator ruling 2026-08-27; discriminator refined WITH breaker — code-less ALONE false-flags
        // ~60 honest declines, so ONLY the ICE-signature set FAILs). An unrecognized code-less message stays
        // Todo (zero false positives; a novel ICE signature is added to the set as it is found).
        if !compiled {
            let (code, message) = first_error_diag(compile_diag);
            worst = worst.worse(if code.is_none() && is_ice_signature(&message) {
                Grade::Fail(format!(
                    "output/trap case failed to compile with an INTERNAL COMPILER ERROR (a bug, not a \
                     capability gap): {message}"
                ))
            } else {
                Grade::Todo(
                    "output/trap case the compiler declined (not-yet-implemented; todo like the gate)"
                        .into(),
                )
            });
            if matches!(worst, Grade::Fail(_)) {
                break;
            }
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
/// value OR the full `(: v T)` form; canonical-`TrapCode` trap match (an unclassifiable trap is `Todo`,
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
            // Resolve the EXPECTED side to a `TrapCode`: an explicit code id (`from_id`, the preferred stable
            // form) or a legacy English reason (`classify`, back-compat). Compare by CODE to `classify(actual)`
            // — the runtime reason (which the backend emits) is the only English matched here.
            // FIRST: an ARTIFACT-ICE actual (`invalid component` / `failed to parse WebAssembly` — the compiler
            // said YES and emitted a component that won't even load) is a compiler BUG, never a runtime trap.
            // It classifies to no `TrapCode`, so a trap-expectation case would otherwise swallow it as Todo
            // (breaker's B1 catch: value-expectation cases already FAIL such an actual, but the trap channel
            // hid it). FAIL unconditionally, before the kind comparison — an unloadable artifact is never the
            // expected trap.
            Outcome::Trap(actual) if is_artifact_ice(actual) => Grade::Fail(format!(
                "expected trap {reason}, but the compiled artifact failed to LOAD (an ICE — the compiler \
                 emitted an invalid component): {actual}"
            )),
            Outcome::Trap(actual) => {
                let want = TrapCode::from_id(reason).or_else(|| classify(reason));
                match (want, classify(actual)) {
                    (Some(w), Some(g)) if w == g => Grade::Pass,
                    // BOTH sides classify to KNOWN trap codes but they DIFFER → a hard disagreement (a
                    // miscompile, or a wrong-kind expectation), graded FAIL exactly like a wrong output value.
                    // Now that trap codes are semantic (CDZ07xx), a mismatched KIND between two traps is a real
                    // disagreement, NOT an unconfirmed Todo that hides it + exits 0 (breaker's grading-gap catch).
                    (Some(w), Some(g)) => Grade::Fail(format!(
                        "expected trap {} ({reason}) but trapped {} ({actual}) — wrong trap kind",
                        w.code(),
                        g.code()
                    )),
                    // The actual reason (or the expectation) classifies to NO known code → genuinely
                    // unconfirmed; stays Todo (never a false Pass, and never a false Fail on a novel trap).
                    _ => Grade::Todo(format!(
                        "trapped ({actual}) but reason kind ≠ expected ({reason})"
                    )),
                }
            }
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

/// Whether a CODE-LESS compile-failure message is an INTERNAL COMPILER ERROR (a bug) rather than an honest
/// not-yet-implemented decline. A curated signature set (operator ruling 2026-08-27, refined WITH breaker):
/// code-less-ness ALONE does NOT mark an ICE — the ~60 honest capability declines ("… has no machine
/// representation / valtype / unbox op / native Rust representation") are also code-less — so only these
/// internal-invariant shapes FAIL, everything else code-less stays Todo (zero false positives). Ported into
/// `xtask gate` — keep in sync. New ICE signatures are ADDED here as they surface (until then they stay Todo).
///  - "no local slot": `parameter/let-binding reference has no local slot` — an emit-stage internal invariant
///    (a resolved binder lost its slot; `rcdzc::opt` confirms it is a compiler bug from wrong resolution/timing).
///  - "is a compiler bug": a self-labeled internal error (`a wildcard literal test is a compiler bug`).
///  - "no bound rust identifier" / "sum match sub-value has no declaration": a resolved reference / match
///    sub-value that lost its binding — a broken internal invariant (breaker-adjudicated true-ICE).
///  - "panicked" / "internal error": a compiler panic/assertion surfaced in stderr.
pub fn is_ice_signature(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("no local slot")
        || m.contains("is a compiler bug")
        || m.contains("no bound rust identifier")
        || m.contains("sum match sub-value has no declaration")
        // An UNREACHABLE-by-construction defensive fallthrough: `lower_quantity_combine`'s caller filters `op`
        // to exactly {Add,Sub,Lt,Gt,Le,Ge,Eq} (all handled by the combine fold), so this `_ =>` arm is a
        // "can't happen" guard — if it ever fires, the caller-filter and the fold-match diverged (a bug), not
        // a capability gap (breaker-flagged, adjudicated internal-invariant). Zero current-corpus reach.
        || m.contains("unexpected op in mixed-unit")
        || m.contains("panicked")
        || m.contains("internal error")
}

/// Whether a RUNTIME failure reason is an ARTIFACT-ICE: the compiler reported success and emitted a component
/// that then FAILS TO LOAD (wasmtime `Component::new` / instantiate rejects it) — the "compiler said yes and
/// produced garbage" face of an internal compiler error (breaker's B1). It is NEVER a legitimate runtime trap,
/// so it must FAIL regardless of the case's expectation kind (a value-expectation already FAILs a trap outcome;
/// this closes the TRAP-expectation channel, where the unloadable-artifact reason classifies to no `TrapCode`
/// and would otherwise be swallowed as an unconfirmed Todo).
pub fn is_artifact_ice(reason: &str) -> bool {
    let r = reason.to_ascii_lowercase();
    r.contains("invalid component")
        || r.contains("failed to parse webassembly")
        || r.contains("failed to instantiate")
        || r.contains("instantiate component")
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
    let mut live_objects: Option<u32> = None;
    let mut live_objects_known_leak = false;

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
            // `(live-objects <N>)` — the post-run heap-balance the case asserts (N as a string leaf). A
            // `(live-objects known-leak <N>)` marker prefixes the count with the literal `known-leak`
            // (the opt-out grandfather form); both assert == N, the marker also records the intent.
            Some("live-objects") => {
                let items = a.as_form(clause, "live-objects").unwrap_or(&[]);
                let first = items.first().copied().and_then(|id| str_leaf(&a, id));
                match first.as_deref() {
                    Some("known-leak") => {
                        live_objects_known_leak = true;
                        live_objects = items
                            .get(1)
                            .copied()
                            .and_then(|id| str_leaf(&a, id))
                            .and_then(|s| s.trim().parse::<u32>().ok());
                    }
                    _ => {
                        live_objects = first.and_then(|s| s.trim().parse::<u32>().ok());
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
        live_objects,
        live_objects_known_leak,
    })
}

/// Decode one `(trial (call export)? (arg v)* <expect>)` form.
fn decode_trial(a: &Arenas, id: StructId) -> Option<GTrial> {
    let items = a.as_form(id, "trial")?;
    let mut export: Option<String> = None;
    let mut method: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    let mut second_call: Option<Vec<String>> = None;
    let mut drop_handle = false;
    let mut expect: Option<GExpect> = None;
    for &child in items {
        match a.head_name(child) {
            Some("call") => {
                export = a
                    .as_form(child, "call")
                    .and_then(|t| t.first().copied())
                    .and_then(|cid| str_leaf(a, cid));
            }
            // `(call-method <member>)` — a value-resource member drive (no export).
            Some("call-method") => {
                method = a
                    .as_form(child, "call-method")
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
            // `(then-call)` opens a two-call continuation (a bare marker; args arrive as `(then-arg …)`),
            // so `Some(vec![])` (nullary second call) is distinct from `None` (no second call).
            Some("then-call") => second_call = Some(Vec::new()),
            Some("then-arg") => {
                if let Some(sc) = second_call.as_mut()
                    && let Some(v) = a
                        .as_form(child, "then-arg")
                        .and_then(|t| t.first().copied())
                        .and_then(|aid| str_leaf(a, aid))
                {
                    sc.push(v);
                }
            }
            // `(drop-handle)` — resource-drop the minted handle after the call(s).
            Some("drop-handle") => drop_handle = true,
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
    // A trial has a call if it named an export OR a `(call-method)` member (the latter has no export — the
    // program's producer makes the value-resource, the member is reached after).
    let call = if export.is_some() || method.is_some() {
        Some(GCall {
            export: export.unwrap_or_default(),
            args,
            second_call,
            drop_handle,
            method,
        })
    } else {
        None
    };
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

/// A STANDARD, CLOSED set of runtime trap KINDS — the "trap code" analogue of a diagnostic `Code` (operator
/// 2026-08-27: "similar to error/warning codes … we're not string matching on english but an actual unique
/// id"). A `(trap …)` corpus expectation and a runtime trap OUTCOME are BOTH resolved to one of these codes
/// and compared by CODE EQUALITY, so grading no longer substring-matches free-form English on the AUTHORED
/// side — the corpus writes a stable code id, and English is matched ONLY on the backend's own runtime reason
/// (which we cannot choose). Like a diagnostic code, a code's id [`TrapCode::code`] MUST NOT change when a
/// backend's human-readable trap wording changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapCode {
    /// Integer divide/remainder by zero (wasm "integer divide by zero"; rust "divide by zero").
    DivByZero,
    /// A heap/list/string index or memory access outside its bounds (wasm "out of bounds memory access").
    OutOfBounds,
    /// A defined arithmetic overflow — `numeric-model.md` §Overflow Is Defined (wasm "integer overflow").
    Overflow,
    /// A bare `unreachable` halt with no more specific kind: an explicit `(trap …)`, an uninhabited match,
    /// a masked shift-count guard, or any trap that classifies to nothing else (wasm "unreachable executed").
    Unreachable,
    /// A cross-component PEER compose-time signature/arity/type reject (`run_with_peers` refuses a peer
    /// whose exported op does not match the consumer's binding). Not a runtime arithmetic trap — a compose
    /// refusal surfaced as a trap by the gate.
    PeerSignatureMismatch,
    /// A peer that does not EXPORT the interface the consumer imports (a missing-op / wrong-interface reject).
    PeerMissingInterface,
}

impl TrapCode {
    /// The STABLE `CDZ07xx` id for this code — the canonical spelling a `(trap "CDZ07xx")` corpus expectation
    /// uses, and the token [`TrapCode::from_id`] round-trips (operator ruling 2026-08-27: "I'm wanting CDZxxxx
    /// codes for trap reasons" — matching the `diag::Code` error/warning scheme). Fixed forever: a backend
    /// reason's wording may drift, but this id must not (the diagnostics.md #every-diagnostic-has-a-stable-code
    /// discipline, for traps). `CDZ07xx` is the RUNTIME-TRAP block, distinct from the compile-diagnostic ranges
    /// in `rcdzc::diag::Code` (through `CDZ06xx` + `CDZ0999`) — keep new trap codes here to avoid collision.
    pub fn code(self) -> &'static str {
        match self {
            TrapCode::DivByZero => "CDZ0701",
            TrapCode::OutOfBounds => "CDZ0702",
            TrapCode::Overflow => "CDZ0703",
            TrapCode::Unreachable => "CDZ0704",
            TrapCode::PeerSignatureMismatch => "CDZ0705",
            TrapCode::PeerMissingInterface => "CDZ0706",
        }
    }

    /// Parse a corpus `(trap "…")` token as an explicit trap CODE id (the [`TrapCode::code`] inverse). `None`
    /// if the token is not a code id — the grader then falls back to [`classify`] for a legacy English reason,
    /// so old `(trap "divide by zero")` cases keep grading while new cases use the stable `(trap "CDZ0701")`.
    pub fn from_id(token: &str) -> Option<TrapCode> {
        match token.trim() {
            "CDZ0701" => Some(TrapCode::DivByZero),
            "CDZ0702" => Some(TrapCode::OutOfBounds),
            "CDZ0703" => Some(TrapCode::Overflow),
            "CDZ0704" => Some(TrapCode::Unreachable),
            "CDZ0705" => Some(TrapCode::PeerSignatureMismatch),
            "CDZ0706" => Some(TrapCode::PeerMissingInterface),
            _ => None,
        }
    }
}

/// Classify a runtime trap REASON (a backend's human-readable string) to its [`TrapCode`], reconciling the
/// corpus + wasmtime + rust-panic vocabularies to one code. `None` for a reason that maps to no known code
/// (an unclassifiable trap → graded Todo, never a false pass). This is the ONE place English is matched — on
/// the RUNTIME reason the backend emits (which we cannot choose); the AUTHORED side uses [`TrapCode::from_id`].
/// Ported verbatim into `xtask gate` so a `(trap …)` case grades identically on wasm and rust — keep in sync.
pub fn classify(reason: &str) -> Option<TrapCode> {
    let r = reason.to_ascii_lowercase();
    if r.contains("divide by zero")
        || r.contains("division by zero")
        || r.contains("remainder by zero")
    {
        Some(TrapCode::DivByZero)
    } else if r.contains("out of bounds") || r.contains("out-of-bounds") {
        Some(TrapCode::OutOfBounds)
    } else if r.contains("overflow") {
        Some(TrapCode::Overflow)
    } else if r.contains("unreachable") || r.contains("shift count out of range") {
        Some(TrapCode::Unreachable)
    } else if r.contains("signature mismatch") || r.contains("type mismatch") {
        // A cross-component PEER compose-time reject in the SIGNATURE family: cdz-run's peer signature check
        // refuses to compose a peer whose exported op does not match the consumer's binding — either an
        // ARITY mismatch ("peer `<iface>` op `<f>` signature mismatch: …") or a per-argument/result TYPE
        // mismatch ("peer `<iface>` op `<f>` type mismatch at argument <n>: …"). Both cdz-run phrases, and
        // the corpus `(trap "signature mismatch")` / `(trap "CDZ0705")` expectation, classify here — so a
        // compose-time reject grades PASS, not an unconfirmed todo. (A compose reject is neither a compile
        // `(declines)`/`(error)` — both components compile — nor a runtime arithmetic trap; it is its own kind.)
        Some(TrapCode::PeerSignatureMismatch)
    } else if r.contains("does not export op") || r.contains("does not export the interface") {
        // A peer that does not export a bound OP ("peer `<iface>` does not export op `<f>`, … offers …") or
        // does not export the bound INTERFACE at all ("peer does not export the interface `<iface>`") — the
        // missing-op / wrong-interface compose reject. Matched on the two SPECIFIC peer phrases (not a bare
        // "does not export", which would also catch unrelated runtime/reducer/NFC infrastructure errors).
        Some(TrapCode::PeerMissingInterface)
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
    fn classify_canonicalizes_backend_vocabulary_to_a_trap_code() {
        assert_eq!(
            classify("integer divide by zero"),
            Some(TrapCode::DivByZero)
        );
        assert_eq!(classify("remainder by zero"), Some(TrapCode::DivByZero));
        assert_eq!(
            classify("out of bounds memory access"),
            Some(TrapCode::OutOfBounds)
        );
        assert_eq!(classify("integer overflow"), Some(TrapCode::Overflow));
        assert_eq!(
            classify("wasm 'unreachable' instruction executed"),
            Some(TrapCode::Unreachable)
        );
        assert_eq!(
            classify("shift count out of range"),
            Some(TrapCode::Unreachable)
        );
        assert_eq!(classify("something else"), None);
        // Cross-component peer compose-time rejects classify to their own codes (both the legacy corpus
        // reason and cdz-run's full message), so a peer reject case grades on the nix path, not just direct.
        assert_eq!(
            classify(
                "peer `cadenza:math/api` op `neg` signature mismatch: expected 2 args, found 1"
            ),
            Some(TrapCode::PeerSignatureMismatch)
        );
        assert_eq!(
            classify("signature mismatch"),
            Some(TrapCode::PeerSignatureMismatch)
        );
        assert_eq!(
            classify("peer does not export the interface `cadenza:math/api`"),
            Some(TrapCode::PeerMissingInterface)
        );
        // A per-argument/result TYPE mismatch is the same signature-family reject as an arity mismatch —
        // cdz-run's actual "type mismatch at argument …" phrase must classify (it lacks "signature mismatch").
        assert_eq!(
            classify(
                "peer `cadenza:math/api` op `neg` type mismatch at argument 0: consumer S64 vs peer Float64"
            ),
            Some(TrapCode::PeerSignatureMismatch)
        );
        // A MISSING bound op — cdz-run's actual "does not export op …, offers …" phrase (distinct from the
        // whole-interface "does not export the interface" phrase) must classify as the missing-interface kind.
        assert_eq!(
            classify("peer `cadenza:math/api` does not export op `neg`, offers [add]"),
            Some(TrapCode::PeerMissingInterface)
        );
    }

    #[test]
    fn trap_code_ids_round_trip_and_grade_by_code() {
        // Every code's stable id round-trips through `from_id` (the corpus-facing token).
        for tc in [
            TrapCode::DivByZero,
            TrapCode::OutOfBounds,
            TrapCode::Overflow,
            TrapCode::Unreachable,
            TrapCode::PeerSignatureMismatch,
            TrapCode::PeerMissingInterface,
        ] {
            assert_eq!(
                TrapCode::from_id(tc.code()),
                Some(tc),
                "id round-trips: {tc:?}"
            );
        }
        assert_eq!(TrapCode::from_id("not-a-code"), None);
        // A `(trap "CDZ07xx")` code-id expectation grades against a runtime English reason by CODE equality —
        // the authored side is the stable id, not English.
        assert_eq!(
            grade_trial(
                &GExpect::Trap("CDZ0701".into()),
                &Outcome::Trap("cdz-run: trap: wasm trap: integer divide by zero".into())
            ),
            Grade::Pass
        );
        // Back-compat: a legacy English expectation still grades (falls back to `classify`).
        assert_eq!(
            grade_trial(
                &GExpect::Trap("divide by zero".into()),
                &Outcome::Trap("integer divide by zero".into())
            ),
            Grade::Pass
        );
        // A code id that classifies but MISMATCHES the actual (both known, differ) is a hard FAIL — a wrong
        // trap kind is a real disagreement, not a hidden Todo (breaker's grading-gap fix).
        assert!(matches!(
            grade_trial(
                &GExpect::Trap("CDZ0703".into()),
                &Outcome::Trap("integer divide by zero".into())
            ),
            Grade::Fail(_)
        ));
        // But when the ACTUAL reason classifies to no known code, it stays Todo (a novel/unclassifiable trap
        // is genuinely unconfirmed — never a false Fail).
        assert!(matches!(
            grade_trial(
                &GExpect::Trap("CDZ0701".into()),
                &Outcome::Trap("some novel host failure".into())
            ),
            Grade::Todo(_)
        ));
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
    fn an_artifact_ice_actual_fails_any_expectation_not_todo() {
        // breaker's B1: a compile-success-but-unloadable component ("invalid component: failed to parse
        // WebAssembly module") is an ICE, never a runtime trap. On a TRAP expectation it classifies to no
        // TrapCode, so WITHOUT the guard it would swallow as Todo — assert it FAILs instead.
        let ice =
            Outcome::Trap("cdz-run: invalid component: failed to parse WebAssembly module".into());
        assert!(matches!(
            grade_trial(&GExpect::Trap("does not export the interface".into()), &ice),
            Grade::Fail(_)
        ));
        assert!(is_artifact_ice(
            "invalid component: failed to parse WebAssembly module"
        ));
        assert!(is_artifact_ice("failed to instantiate component"));
        assert!(!is_artifact_ice("integer divide by zero"));
        // is_ice_signature covers the compile-decline ICE shapes, incl. the unreachable-by-construction
        // "unexpected op in mixed-unit …" defensive fallthrough (breaker-adjudicated internal-invariant);
        // an honest capability decline ("has no machine representation") is NOT an ICE.
        assert!(is_ice_signature("parameter reference has no local slot"));
        assert!(is_ice_signature("unexpected op in mixed-unit int combine"));
        assert!(is_ice_signature(
            "unexpected op in mixed-unit float combine"
        ));
        assert!(!is_ice_signature(
            "a function parameter's type has no machine representation"
        ));
        // A VALUE expectation already FAILs any trap actual (regression guard, unchanged).
        assert!(matches!(
            grade_trial(&GExpect::Output("(: 60 Int64)".into()), &ice),
            Grade::Fail(_)
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

        // COMPILE-FAILURE grading of an output case (compile_status != 0, nothing runs):
        let never = |_: &GTrial| -> Result<Outcome> { panic!("must not run a declined case") };
        // An ICE-signature code-less error (a compiler BUG) → FAIL, never a hidden todo.
        let res = grade_run(
            &tr,
            1,
            "cdz: error: parameter reference has no local slot",
            never,
        )
        .unwrap();
        assert!(
            matches!(res.grade, Grade::Fail(_)),
            "ICE signature → fail: {:?}",
            res.grade
        );
        assert!(!res.ran_a_trial);
        // A code-less HONEST decline (no ICE signature) stays Todo — the false-positive guard: the ~60
        // capability-gap declines ("type has no machine representation") must NOT flip to fail.
        let res = grade_run(
            &tr,
            1,
            "cdz: error: a function parameter's type has no machine representation",
            never,
        )
        .unwrap();
        assert!(
            matches!(res.grade, Grade::Todo(_)),
            "honest code-less decline → todo: {:?}",
            res.grade
        );
        // A CODED decline (CDZxxxx) is a genuine capability gap → Todo.
        let res = grade_run(
            &tr,
            1,
            "error [CDZ0210] (node 3): match is non-exhaustive",
            never,
        )
        .unwrap();
        assert!(
            matches!(res.grade, Grade::Todo(_)),
            "coded decline → todo: {:?}",
            res.grade
        );
        // A silent decline (no error line at all) stays Todo.
        let res = grade_run(&tr, 1, "", never).unwrap();
        assert!(
            matches!(res.grade, Grade::Todo(_)),
            "silent decline → todo: {:?}",
            res.grade
        );
    }

    #[test]
    fn baseline_lookup_and_regression() {
        let baseline = "# gate baseline\n\
                        pass\ta passing case\n\
                        todo\tan incomplete case\n\
                        pass\ta dup case\n\
                        todo\ta dup case\n"; // last-wins → todo
        assert_eq!(
            baseline_verdict(baseline, "a passing case"),
            Some(Verdict::Pass)
        );
        assert_eq!(
            baseline_verdict(baseline, "an incomplete case"),
            Some(Verdict::Todo)
        );
        assert_eq!(
            baseline_verdict(baseline, "a dup case"),
            Some(Verdict::Todo)
        );
        assert_eq!(baseline_verdict(baseline, "absent"), None);

        // pass -> not-pass is the ONLY regression.
        assert!(check_regression(Verdict::Todo, "a passing case", baseline).is_some());
        assert!(check_regression(Verdict::Fail, "a passing case", baseline).is_some());
        assert!(check_regression(Verdict::Pass, "a passing case", baseline).is_none());
        // todo -> fail is NOT flagged (only pass -> not-pass), matching the gate.
        assert!(check_regression(Verdict::Fail, "an incomplete case", baseline).is_none());
        // gained (todo -> pass) is not a regression; an absent case is not either.
        assert!(check_regression(Verdict::Pass, "an incomplete case", baseline).is_none());
        assert!(check_regression(Verdict::Fail, "absent", baseline).is_none());
    }

    #[test]
    fn exec_exit_matches_xtask_check_semantics() {
        let fmt = |c: ExitCode| format!("{c:?}"); // ExitCode has no PartialEq; compare Debug
        let success = fmt(ExitCode::SUCCESS);
        let failure = fmt(ExitCode::FAILURE);
        let baseline = "pass\ta passing case\ntodo\tan incomplete case\nfail\ta known-fail case\n";
        let res = |g: Grade| GradeResult {
            grade: g,
            ran_a_trial: true,
        };
        // WITH baseline: a baseline-PASS case that now FAILS is a REGRESSION → FAILURE.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Fail("x".into())),
                "a passing case",
                Some(baseline)
            )),
            failure
        );
        // WITH baseline: a baseline-TODO case that now FAILS reds (#3984 gate-hole) → FAILURE.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Fail("x".into())),
                "an incomplete case",
                Some(baseline)
            )),
            failure
        );
        // WITH baseline: an ABSENT case that FAILS reds (#3984: non-pass, non-fail) → FAILURE.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Fail("x".into())),
                "absent",
                Some(baseline)
            )),
            failure
        );
        // WITH baseline: a baseline-TODO case that stays TODO is NOT a regression → SUCCESS.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Todo("x".into())),
                "an incomplete case",
                Some(baseline)
            )),
            success
        );
        // WITH baseline: a `fail` baseline + `fail` verdict is a TRACKED known-fail (#4547) → SUCCESS.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Fail("x".into())),
                "a known-fail case",
                Some(baseline)
            )),
            success
        );
        // WITHOUT baseline: any outright Fail → FAILURE (the miscompile check).
        assert_eq!(
            fmt(exec_exit(&res(Grade::Fail("x".into())), "x", None)),
            failure
        );
        // A Pass always succeeds.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Pass),
                "a passing case",
                Some(baseline)
            )),
            success
        );
    }

    /// `check_live_objects` balances EVERY trial, not just call[0] — the fix for the systemic false-green
    /// where a multi-call case that balanced on the first call hid a leak on later calls.
    #[test]
    fn check_live_objects_balances_every_call() {
        // No-heap trials are skipped entirely (never a false fail), regardless of `expected`.
        assert_eq!(check_live_objects(&[None, None], Some(0)), None);
        assert_eq!(check_live_objects(&[], None), None);
        // A single heap trial at the expected count passes; the message on a miss is index-free (stable text).
        assert_eq!(check_live_objects(&[Some(0)], Some(0)), None);
        assert_eq!(
            check_live_objects(&[Some(1)], Some(0)).as_deref(),
            Some("live-objects mismatch: expected 0, got 1")
        );
        // Absent clause ⇒ opt-out default of 0.
        assert_eq!(
            check_live_objects(&[Some(2)], None).as_deref(),
            Some("live-objects mismatch: expected 0, got 2")
        );
        // THE FIX: a multi-call case that balances on call 0 but LEAKS on call 2 is now caught — the
        // historical first-call-only capture returned None here (false green).
        assert_eq!(
            check_live_objects(&[Some(0), Some(0), Some(0)], Some(0)),
            None
        );
        assert_eq!(
            check_live_objects(&[Some(0), Some(0), Some(3)], Some(0)).as_deref(),
            Some("live-objects mismatch on call 2: expected 0, got 3")
        );
        // A depth-scaling leak on the FIRST call still reports call 0 (multi-call message form).
        assert_eq!(
            check_live_objects(&[Some(1), Some(2)], Some(0)).as_deref(),
            Some("live-objects mismatch on call 0: expected 0, got 1")
        );
        // A uniform expected N > 0 (an explicit `(live-objects N)` / known-leak N) holds across every call.
        assert_eq!(check_live_objects(&[Some(2), Some(2)], Some(2)), None);
        // Interleaved no-heap trials don't shift the reported call index (index is the trial position).
        assert_eq!(
            check_live_objects(&[None, Some(0), Some(5)], Some(0)).as_deref(),
            Some("live-objects mismatch on call 2: expected 0, got 5")
        );
    }

    /// `decode_test_run` reads a `(live-objects <N>)` form into `TestRun.live_objects` (and the
    /// `(live-objects known-leak <N>)` marker form into `live_objects_known_leak` + the count); a
    /// test-run with no clause leaves the count `None` and the flag `false`.
    #[test]
    fn decode_reads_live_objects() {
        use cadenza_syntax::ast::{Builder, Leaf};
        use std::sync::Arc;
        // `clause`: None = no live-objects form; Some(leaves) = a `(live-objects <leaves…>)` form.
        let build = |clause: Option<&[&str]>| -> Vec<u8> {
            let mut b = Builder::new();
            let s = |b: &mut Builder, t: &str| b.atom_leaf(Leaf::Str(Arc::from(t)));
            let head = b.name("test-run");
            let dh = b.name("description");
            let dv = s(&mut b, "case");
            let desc = b.list(vec![dh, dv]);
            let trials_head = b.name("trials");
            let trials = b.list(vec![trials_head]);
            let mut kids = vec![head, desc, trials];
            if let Some(leaves) = clause {
                let loh = b.name("live-objects");
                let mut lo = vec![loh];
                for &l in leaves {
                    let v = s(&mut b, l);
                    lo.push(v);
                }
                kids.push(b.list(lo));
            }
            let root = b.list(kids);
            codec::encode(&b.finish(root))
        };
        // Plain count.
        let tr = decode_test_run(&build(Some(&["0"]))).unwrap();
        assert_eq!(tr.live_objects, Some(0));
        assert!(!tr.live_objects_known_leak);
        // known-leak marker → count + flag.
        let tr = decode_test_run(&build(Some(&["known-leak", "3"]))).unwrap();
        assert_eq!(tr.live_objects, Some(3));
        assert!(tr.live_objects_known_leak);
        // No clause.
        let tr = decode_test_run(&build(None)).unwrap();
        assert_eq!(tr.live_objects, None);
        assert!(!tr.live_objects_known_leak);
    }
}
