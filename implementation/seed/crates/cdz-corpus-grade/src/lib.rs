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

use std::collections::BTreeMap;
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
///
/// `membership_only` = the baseline is a CURATED SUBSET of the corpus (the RUST backend: `.gate-baseline-rust`
/// covers ~8962 of ~10819 cases — rust stays INCREMENTAL, no-value-heap; the ABSENT cases are intentionally
/// not-covered-on-rust, not gate-holes). Under it, a case ABSENT from the baseline (verdict `None`) is
/// NOT ENFORCED — a FAIL there is out-of-scope (the case is not in the curated set), NOT a #3984 red. This is
/// exactly "grade IFF title ∈ baseline" (a baseline-absent case is unenforced, matching `check_regression`).
/// A baselined `todo` case that now fails STILL reds (it IS covered). `false` = the WASM bar: the wasm
/// `.gate-baseline` == the full-corpus harvest (no legitimately-absent cases), so #3984 stays strict there.
pub fn exec_exit(
    result: &GradeResult,
    description: &str,
    baseline: Option<&str>,
    membership_only: bool,
) -> ExitCode {
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
                    // MEMBERSHIP-ONLY (curated-subset baseline, e.g. rust): an ABSENT case (`bv` None) is
                    // NOT in the curated set → a FAIL there is out-of-scope, NOT enforced (the rust backend
                    // is incremental; absent = not-covered-on-rust, not a gate-hole). A baselined `todo`
                    // (bv Some(Todo)) that fails STILL reds — it IS covered. Only the ABSENT case is exempt.
                    if membership_only && bv.is_none() {
                        eprintln!(
                            "grade: {description}: FAIL but ABSENT from the curated baseline — NOT enforced \
                             (membership-only: grade IFF title ∈ baseline; the rust backend is incremental)"
                        );
                        return ExitCode::SUCCESS;
                    }
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
    /// Optional DIAGNOSTIC-QUALITY assertions for an `(error …)` / `(warning …)` compile outcome — a
    /// structural `(fix …)` / `(no-fix)` / exact `(count N)`/`(once)` beyond the code + message. `None`
    /// when the case asserts only code + message (the common form). Graded by [`grade_diag_quality`]
    /// against the structured diagnostics once the exec captures them (the `(error …)` diagnostic-quality
    /// capability — lets the corpus "express fixes", migrating the `rcdzc/tests.rs` fix-it tests).
    pub diag: Option<DiagExpect>,
    /// The `(exact-code)` opt-in on an `(expect-error …)` trial (C1 fence): demand the compiler emit EXACTLY
    /// this code — a DIFFERENT/uncoded refusal FAILs (not the default lenient Todo). Fences error-masking
    /// regressions. `false` (the norm) keeps the lenient wrong-code→Todo. See [`grade_compile_error`].
    pub exact_code: bool,
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
    /// `(output-byte-len N)` — a RUN outcome that pins ONLY the SIZE of the escaped value: its canonical
    /// binary-AST ENCODING must be exactly `N` bytes. Type-AGNOSTIC (one grade path for String / Bytes /
    /// list / closure-result) and impl-INDEPENDENT (the binary-AST encoding is THE canonical data-exchange
    /// format — every backend produces identical bytes for the same value), so a >64KiB value-escape (the
    /// #7793/#7800 OOB class) is corpus-fenceable at O(1) bytes: the case BUILDS the big value at runtime (a
    /// tiny doubling source) and lets it ESCAPE as the result; this clause asserts its encoded length without
    /// spelling the 64KiB literal (which would blow the 512KB source mandate). Deliberately WEAKER than a full
    /// `Output` value pin (a wrong-CONTENT, right-LENGTH payload passes) — a size-class fence; pair with a
    /// consumed scalar `(output …)` in-case for content coverage. Graded by [`value_encoding_byte_len`].
    OutputByteLen(u64),
    Trap(String),
    /// `(expect-error CODE msg* (not "phrase")*)` — the compiler must REFUSE with exactly `CODE` (+ required
    /// message substrings AND + required-ABSENCE substrings the message must NOT contain, seq-29).
    Error(String, Vec<String>, Vec<String>),
    /// `(expect-warning CODE msg* (not "phrase")*)` — the compiler must COMPILE (produce an artifact) AND emit
    /// a WARNING with exactly `CODE` (+ required message substrings AND + required-ABSENCE substrings, seq-29).
    /// Severity-distinct from `Error` (which DENIES the artifact): a warning ACCOMPANIES a produced component.
    /// Pairs with a `(count N)` for the exact-count warning cases a presence-only `(warns …)` can't express.
    Warning(String, Vec<String>, Vec<String>),
    // NOTE: the `Declines` expectation (`(expect-declines …)`) was REMOVED (operator directive; corpus
    // (declines)=0). A corpus rejection is coded `(error CDZxxxx)` and a should-work is a TODO `(output V)`.
    // The `cdz-corpus` parser rejects a `(declines)` clause as a hard error, so no `expect-declines` form is
    // ever produced to grade — this enum carries no `Declines` variant and `grade_compile_declines` is gone.
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
    /// PER-CALL positional expected counts — `Some` iff the case authored `(live-objects [known-leak] N1 N2
    /// …)` with 2+ counts, one per trial in call order. `None` for the uniform/absent forms (then
    /// `live_objects` applies to every call). This expresses an ARM-DEPENDENT balance a single count cannot:
    /// a leak that scales with input size (FLETCHER-16 r=1→3, r=4→13, r=0→0), or a benign per-call count
    /// difference (dedup 17→16) — surfaced once `#5008` graded every call, not just call[0]. `live_objects`
    /// still holds the FIRST count (so the uniform/direct-gate path keeps grading call[0]); the wasm nix
    /// grade path uses this list to balance EACH call against its own count.
    pub live_objects_per_call: Option<Vec<u32>>,
    /// `true` iff the case authored `(no-other-errors)` — a CASE-LEVEL no-cascade assertion: after the
    /// trials, the set of EMITTED error-severity fault codes must be a SUBSET of the case's asserted
    /// `(error CODE)` codes (FAIL on any unasserted error code). Errors only. Composes with per-code
    /// `(count …)`; only enforced when the diagnostics wire was captured (`diag_wire` `Some`).
    pub no_other_errors: bool,
    /// `(no-diagnostic "phrase")` clauses — CASE-LEVEL, PROGRAM-SCOPED, CROSS-KIND message-ABSENCE pins:
    /// each phrase must appear in NO diagnostic the compiler emits for the program (ANY kind — coded/uncoded
    /// error, decline, warning). Graded by scanning the FULL raw `compile_diag` text (not a single matched
    /// diagnostic), which is what distinguishes it from a trial's KIND-scoped `(not "phrase")` and from the
    /// coded-error-only `(no-other-errors)`. Repeatable (AND — every phrase must be absent). Empty = no clause.
    pub no_diagnostic: Vec<String>,
    /// `true` iff the case authored a bare `(diagnostic-quality)` marker — the C1 opt-in: assert EVERY
    /// emitted CODED diagnostic's message contains no globally-forbidden phrase (`DESIGN-diagnostic-quality-
    /// rubric.md` §1 — future-promise/deferral + internal-implementation leak). (The §2 per-code
    /// required-token check was withdrawn as unsound for umbrella codes — see [`grade_diagnostic_quality`];
    /// message SHAPE lives in per-case `(message …)` pins.) Graded by [`grade_diagnostic_quality`] against
    /// the captured structured faults — composes with the per-trial `(fix …)`/`(count …)` facets and the
    /// per-case `(no-other-errors)`/`(no-diagnostic …)`, on the SAME nix diagnostics bar. NOW A NO-OP: the
    /// C1 lint is DEFAULT-ON (the opt-in→default flip landed after the warm-lane full-corpus verify), so
    /// this legacy `(diagnostic-quality)` opt-IN marker no longer gates anything (grading runs regardless).
    /// Retained so the ~36 chapters' existing markers still parse harmlessly; sweepable later.
    pub diagnostic_quality: bool,
    /// `true` iff the case (or its file) authored `(no-diagnostic-quality)` — the C1 opt-OUT escape hatch:
    /// SUPPRESS the default-on §1 lint for this case (a case that legitimately must carry a §1 forbidden
    /// phrase in its diagnostic — none known today; reversible safety net for a post-flip surprise). The
    /// only gate on the default-on `grade_diagnostic_quality`; `false` (the norm) = §1-enforced.
    pub diagnostic_quality_opt_out: bool,
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
/// Does a trial's `expect` denote a HEAP-FREE SCALAR return (so its live-cell residual is unambiguously
/// 0)? Only an `output` trial returning a bare scalar — integer/float/bool/char/identifier — owns no live
/// heap after the run. A heap/compound return (`#…`/`(…)`/`"…"`/`{…}`/`[…]`) has a nonzero reachable-return
/// count a single case-level `(live-objects 0)` clause can't express per-trial, so it is NOT a per-trial
/// 0-check candidate. Ported from xtask's guarded-all `trial_expect_is_scalar_return` (#7527) so the SHARED
/// nix grade path applies the same discriminator (else the coarse harvest false-fails the 3 heap-return
/// non-leaks — the empty-or-nonempty list/map/set trio). Errs toward NOT-a-candidate: a missed scalar leak
/// is under-admit (leak-safe), a wrong 0-check on a heap return would RED a correct case.
pub fn expect_is_scalar_return(expect: &GExpect) -> bool {
    let GExpect::Output(v) = expect else {
        return false; // trap/error/warning return no value → not a 0-check candidate
    };
    // The value is bare (`42`) or in the resource-escape ascription form (`(: 42 Int64)`).
    let value = v
        .trim_start()
        .strip_prefix("(:")
        .map(str::trim_start)
        .unwrap_or(v.trim_start());
    match value.chars().next() {
        Some('#' | '(' | '"' | '{' | '[') => false, // heap/compound render
        Some(c)
            if c.is_ascii_digit() || matches!(c, '-' | '+' | '\'') || c.is_ascii_alphabetic() =>
        {
            true
        }
        _ => false,
    }
}

/// The heap-balance assertion (strict): checks EVERY heap trial's 0-residual with NO scalar-return
/// discriminator. The grade path calls [`check_live_objects_scalar`] instead (which applies the #7527
/// discriminator); this strict entry point is retained for the existing count-logic call sites/tests.
pub fn check_live_objects(
    per_trial: &[Option<u32>],
    expected: Option<u32>,
    per_call: Option<&[u32]>,
) -> Option<String> {
    check_live_objects_scalar(per_trial, expected, per_call, &[])
}

/// As [`check_live_objects`], plus the #7527 scalar-return discriminator: `per_trial_scalar[i]` says whether
/// trial `i` returns a heap-free scalar (see [`expect_is_scalar_return`]). Under a must-reclaim-to-0
/// expectation (UNIFORM, `want == 0`), a LATER trial (`i > 0`) whose return is NOT a scalar (a heap/compound
/// return) has a legitimate nonzero reachable-return count → its 0-check is SKIPPED (not a leak). Trial 0
/// stays the always-checked calibration; POSITIONAL `(live-objects N1 N2 …)` and `Expect(n>0)` are
/// unaffected. Under-admit-safe (a heap-return trial that ALSO leaks extra goes uncaught; the full fix is
/// per-trial expected counts — a future corpus-grammar increment). An empty `per_trial_scalar` applies no
/// skip (strict — every trial checked).
pub fn check_live_objects_scalar(
    per_trial: &[Option<u32>],
    expected: Option<u32>,
    per_call: Option<&[u32]>,
    per_trial_scalar: &[bool],
) -> Option<String> {
    // POSITIONAL: `(live-objects [known-leak] N1 N2 …)` — one expected count per trial, index-aligned to
    // the call order. This is the arm-dependent case (e.g. a leak that SCALES with input size): call 0 may
    // balance to N1 while call 1 balances to N2. A no-heap trial (`None`) is skipped (its entry is ignored).
    // The list length MUST equal the trial count so an author can't silently under-specify a call.
    if let Some(list) = per_call {
        if list.len() != per_trial.len() {
            return Some(format!(
                "live-objects per-call list has {} entr{} but the case ran {} trial(s)",
                list.len(),
                if list.len() == 1 { "y" } else { "ies" },
                per_trial.len()
            ));
        }
        for (i, live) in per_trial.iter().enumerate() {
            if let Some(n) = live
                && *n != list[i]
            {
                return Some(format!(
                    "live-objects mismatch on call {i}: expected {}, got {n}",
                    list[i]
                ));
            }
        }
        return None;
    }
    // UNIFORM: `(live-objects [known-leak] N)` (or no clause → 0) — every heap trial must end at the SAME
    // count. Checks EVERY call, not just call[0] (the systemic false-green the first-call-only capture hid).
    let want = expected.unwrap_or(0);
    for (i, live) in per_trial.iter().enumerate() {
        if let Some(n) = live
            && *n != want
        {
            // #7527 discriminator: under a must-reclaim-to-0 expectation, a LATER trial (i > 0) that
            // RETURNS a heap value owns a nonzero reachable-return count a single `(live-objects 0)` clause
            // can't express per-trial — skip its 0-check (not a leak). Trial 0 is the always-checked
            // calibration; strict when `per_trial_scalar` is empty/absent (defaults to scalar = checked).
            if want == 0 && i > 0 && !per_trial_scalar.get(i).copied().unwrap_or(true) {
                continue;
            }
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

/// seq-15 PURE-BINARY leak semantics: a KNOWN-LEAK case (`(live-objects known-leak)`) is NEVER count-checked
/// (the leak magnitude does not matter — [`check_live_objects`] is simply skipped for it by the grade
/// callers). This surfaces the FIX signal instead: return `true` iff the known-leak case now measures FULLY
/// CLEAN — at least one heap trial ran and EVERY heap trial ended at 0 live cells — so its reclaim fix has
/// landed and the `(live-objects known-leak)` marker is a TIGHTEN CANDIDATE ready to drop. Non-blocking (the
/// grade prints an advisory, never fails on it). A no-heap case (all `None`) is not a candidate (nothing
/// measured).
pub fn known_leak_now_clean(per_trial: &[Option<u32>]) -> bool {
    per_trial.iter().any(Option::is_some) && per_trial.iter().flatten().all(|&n| n == 0)
}

/// The BASELINE-SIDE known-leak LEDGER (operator directive 2026-09-05: known-leak is a COMPILER FACT, not
/// spec, so it lives baseline-side, not as a corpus annotation — see the `B + leak-ledger` design). This is
/// the ADDITIVE mechanism half (parse + compare), OFF until the `B` grade change (drop-top-level result →
/// true-leak count) + migration wire it in; building it dead-first mirrors the C1 fence rollout (#8401).
///
/// The ledger file (`spec/semantics/.gate-baseline-leaks`, WASM-only) mirrors `.gate-baseline`'s
/// `<key>\t<description>` shape: one line `<N>\t<description>` per case whose TRUE leak (top-level result
/// dropped, post-B) is `N > 0`. A case ABSENT from the ledger is expected to leak 0 (the default). Lines that
/// are blank or start with `#` (the header) are ignored.
pub fn parse_leak_ledger(text: &str) -> BTreeMap<String, u32> {
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((n, desc)) = line.split_once('\t')
            && let Ok(n) = n.trim().parse::<u32>()
            && !desc.is_empty()
        {
            m.insert(desc.to_string(), n);
        }
    }
    m
}

/// Grade one case's observed TRUE-leak count (top-level result dropped, post-`B`) against the leak ledger,
/// mirroring the verdict-baseline mismatch model. RED-ON-ANY-CHANGE (operator ruling 2026-09-05: "notified
/// ANY time that progress changes"): a case's `tracked` leak is `ledger[description]` (or 0 when ABSENT), and
/// ANY deviation — a leak that GREW (regression) OR SHRANK (progress to record via `save-leaks`) — returns a
/// mismatch, exactly like a `.gate-baseline` verdict deviation. `None` = matches (no notification). Pure +
/// OFF until `B`+migration call it from the grade path.
pub fn check_leak_ledger(
    description: &str,
    observed_true_leak: u32,
    ledger: &BTreeMap<String, u32>,
) -> Option<String> {
    let tracked = ledger.get(description).copied().unwrap_or(0);
    (observed_true_leak != tracked).then(|| {
        format!(
            "leak-ledger mismatch: tracked {tracked} true-leak(s), observed {observed_true_leak} \
             (grew=regression / shrank=progress — re-run `save-leaks` to record the new count)"
        )
    })
}

/// Clamp each observed live-cell count UP to its expected threshold — the LEAK-CEILING tolerance for a
/// KNOWN-LEAK case graded on a strictly-safer path (the corpus-cadenza cadenza-hop, opted in via the
/// `--tolerate-fewer-live-objects` flag). A known-leak `N` is a TOLERATED-leak CEILING, so a path that
/// reclaims MORE (ends at `count <= N`) is strictly safer and must PASS; only EXCEEDING the ceiling
/// (`count > N`) is a real worse-leak regression. Clamping `n -> max(n, threshold)` makes [`check_live_objects`]
/// see `== threshold` (pass) for `<=` and still `> threshold` (fail) for an over-count — WITHOUT relaxing the
/// direct path (which never clamps, so it stays an exact `== N` drift guard). `threshold` is the per-call
/// count when positional, else the uniform expected. A `None` (no-heap) trial stays `None`.
pub fn leak_ceiling_clamp(
    per_trial: &[Option<u32>],
    uniform: u32,
    per_call: Option<&[u32]>,
) -> Vec<Option<u32>> {
    per_trial
        .iter()
        .enumerate()
        .map(|(i, c)| {
            c.map(|n| {
                let threshold = per_call.and_then(|l| l.get(i).copied()).unwrap_or(uniform);
                n.max(threshold)
            })
        })
        .collect()
}

/// Grade a whole case: decode is the caller's (it has the bytes); this orchestrates the trials + checks,
/// calling `run_trial` for each RUNNABLE (output/trap, compiled) trial to obtain its [`Outcome`]. Compile
/// outcomes (error/declines) + warns are graded from `compile_status`/`compile_diag` (no run). Reproduces
/// the gate's `grade_ran`: the worst of every trial + the host-call-sequence + the warns checks.
pub fn grade_run<F>(
    test_run: &TestRun,
    compile_status: i32,
    compile_diag: &str,
    diag_wire: Option<&[u8]>,
    // The `cdz check --diagnostics-wire` capture for the SAME case, for the C1 check-vs-compile parity leg
    // (see [`grade_check_parity`]). `None` = the pipeline did not capture a check wire (parity OFF, today's
    // default); the upstream capture flips it on. Parity is graded only when BOTH this and `diag_wire` are
    // `Some` (nothing to compare otherwise).
    check_diag: Option<&[u8]>,
    mut run_trial: F,
) -> Result<GradeResult>
where
    F: FnMut(&GTrial) -> Result<Outcome>,
{
    let compiled = compile_status == 0;
    let mut worst = Grade::Pass;
    let mut ran_a_trial = false;
    // The STRUCTURED diagnostics (`KIND_DIAGNOSTICS` wire) a trial's `(fix …)`/`(count …)` facets grade
    // against, parsed once. `None` = the pipeline did not capture the wire (diagnostic-QUALITY grading is
    // OFF — the facets still parse/shred/decode, but are not asserted, today's behavior); `Some(faults)` =
    // captured, so `grade_diag_quality` fires per diag-bearing trial. `Some(&[])` (captured but empty) is
    // meaningful: a warning/fix case with no matching fault then FAILS ("expected a fault, found none").
    let faults: Option<Vec<DiagFault>> = diag_wire.map(parse_diagnostics);
    // The observed host-call sequence of the first value-producing trial — the gate checks host_calls
    // against exactly this (a compiled program's host effects are the same on every trial).
    let mut first_observed: Option<Vec<String>> = None;

    for trial in &test_run.trials {
        // COMPILE-OUTCOME expectations (error/declines) are graded against the captured diagnostic, no run.
        match &trial.expect {
            GExpect::Error(code, msg, not_msg) => {
                worst = worst.worse(grade_compile_error(
                    compiled,
                    compile_diag,
                    code,
                    msg,
                    not_msg,
                    trial.exact_code,
                ));
                // DIAGNOSTIC-QUALITY facets (`(fix …)`/`(no-fix)`/`(count …)`) — graded against the captured
                // structured faults for THIS error's `(Error, code)`. Only when the wire was captured.
                if let (Some(faults), Some(diag)) = (&faults, &trial.diag) {
                    worst = worst.worse(grade_diag_quality(faults, Severity::Error, code, diag));
                }
                if matches!(worst, Grade::Fail(_)) {
                    break;
                }
                continue;
            }
            GExpect::Warning(code, msg, not_msg) => {
                worst = worst.worse(grade_compile_warning(
                    compiled,
                    compile_diag,
                    code,
                    msg,
                    not_msg,
                ));
                // Same diagnostic-QUALITY facets, graded for THIS warning's `(Warning, code)`.
                if let (Some(faults), Some(diag)) = (&faults, &trial.diag) {
                    worst = worst.worse(grade_diag_quality(faults, Severity::Warning, code, diag));
                }
                if matches!(worst, Grade::Fail(_)) {
                    break;
                }
                continue;
            }
            GExpect::Output(_) | GExpect::Trap(_) | GExpect::OutputByteLen(_) => {}
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
    // GATED on `compiled` (the sibling `warns` check above is too): a case that did NOT compile RAN NOTHING,
    // so it has no observed host-calls to compare — a SOUND coded decline of a should-run case (its idealistic
    // `(host-calls …)` is the goal, not yet met) must stay its decline-TODO, NOT be spuriously FAILed by
    // `observed [] != [expected]` (operator corpus policy: a sound decline is a `todo`, never a hard fail —
    // e.g. fpr3's deferred cross-function-resume CDZ0900 decline). A COMPILED run that made the wrong calls
    // (observed Some, even empty) still fails correctly.
    if !matches!(worst, Grade::Fail(_)) && !test_run.host_calls.is_empty() && compiled {
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

    // CASE-LEVEL `(no-other-errors)` — the no-cascade assertion: every EMITTED coded error-fault must be one
    // the case asserted via an `(error CODE)` trial. Any coded error outside that set is an unexpected cascade
    // (FAIL). Only when the diagnostics wire was captured (`Some(faults)`); errors only (warnings orthogonal).
    // A codeless error fault carries no code to match, so it is out of this facet's scope (the primary decline
    // is graded by its own trial's message).
    if test_run.no_other_errors
        && let Some(faults) = &faults
    {
        let asserted: std::collections::HashSet<&str> = test_run
            .trials
            .iter()
            .filter_map(|t| match &t.expect {
                GExpect::Error(code, ..) => Some(code.as_str()),
                _ => None,
            })
            .collect();
        for f in faults.iter().filter(|f| f.severity == Severity::Error) {
            if let Some(code) = f.code.as_deref()
                && !asserted.contains(code)
            {
                worst = worst.worse(Grade::Fail(format!(
                    "(no-other-errors): unexpected error {code} beyond the asserted {asserted:?}"
                )));
                break;
            }
        }
    }

    // CASE-LEVEL `(no-diagnostic "phrase")` — PROGRAM-SCOPED, CROSS-KIND message-absence: each pinned phrase
    // must appear in NO diagnostic emitted for the program. Scans the FULL raw `compile_diag` (all lines, any
    // kind: coded/uncoded error, decline, warning) — the capability a trial's `(not "phrase")` (kind-scoped to
    // its own matched diagnostic) and `(no-other-errors)` (coded-error-only) cannot express. Always checkable
    // (the diag text is captured on every case); a present phrase is a FAIL (the forbidden diagnostic leaked).
    for phrase in &test_run.no_diagnostic {
        if compile_diag.contains(phrase.as_str()) {
            worst = worst.worse(Grade::Fail(format!(
                "(no-diagnostic {phrase:?}): the forbidden phrase appears in a diagnostic, but the case \
                 asserts it must appear in NONE"
            )));
            break;
        }
    }

    // C1 §1 diagnostic-quality lint — DEFAULT-ON (the opt-in→default flip; concierge-greenlit, warm-lane
    // full-corpus §1 verify GREEN = 0 forbidden-phrase / 0 dq-fails across all 36 files, v-gha-green). Every
    // emitted CODED diagnostic's message must contain no forbidden phrase (`DESIGN-diagnostic-quality-
    // rubric.md` §1; §2 withdrawn) — the corpus-wide golden-standard guarantee. Runs on EVERY case UNLESS it
    // opts OUT via `(no-diagnostic-quality)` (the reversible per-case/file escape hatch). It is a CODE
    // default (not an env) so it enforces in BOTH the in-process gate AND the hermetic nix corpus-exec (a
    // shell env does not propagate into the nix drv — v-gha-green's caveat). Only when the diagnostics wire
    // was captured (`Some(faults)`), like `(no-other-errors)` + the `(fix …)`/`(count …)` facets — the
    // sidecar-blind in-process path skips it. The legacy `(diagnostic-quality)` opt-IN markers are now
    // no-ops (harmless; sweepable later). A post-flip §1 surprise is one `(no-diagnostic-quality)` + a rewrite.
    if !test_run.diagnostic_quality_opt_out
        && let Some(faults) = &faults
        && let Some(msg) = grade_diagnostic_quality(faults)
    {
        worst = worst.worse(Grade::Fail(msg));
    }

    // C1 check-vs-compile diagnostic PARITY (#7143): when BOTH the compile-phase diagnostics wire and a
    // `cdz check` wire were captured for this case, `cdz check` must surface every CODED fault `cdz compile`
    // rejects. INERT unless the pipeline threads `check_diag` (today's callers pass `None`); the upstream
    // per-case `cdz check --diagnostics-wire` capture flips it on. A miss downgrades the grade to Fail.
    if let (Some(compile_wire), Some(check_wire)) = (diag_wire, check_diag)
        && let Some(msg) = grade_check_parity(compile_wire, check_wire)
    {
        worst = worst.worse(Grade::Fail(msg));
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
        GExpect::Output(payload) => match outcome {
            // STRUCTURAL value compare via the canonical reader/printer (operator directive: NO
            // hand-rolled value scan). Both the expected `(: v T)` payload and the run value are read
            // by the canonical s-expr reader and re-printed from their VALUE subtree, so bare-vs-annotated
            // and any rendering variance normalize away; a parse failure is surfaced LOUDLY as a Fail
            // (never a silent pass) — a corpus authoring error on the expected side, a compiler emit bug
            // on the actual side (the decode-validity invariant).
            Outcome::Value(v, _) => {
                match (canonical_output_value(payload), canonical_output_value(v)) {
                    (Ok(want), Ok(got)) if want == got => Grade::Pass,
                    (Ok(want), Ok(got)) => Grade::Fail(format!(
                        "expected output {payload} (canonical value {want}), got value {v} \
                         (canonical value {got})"
                    )),
                    (Err(e), _) => Grade::Fail(format!(
                        "corpus expected-output {payload} did not parse as a canonical value: {e}"
                    )),
                    (_, Err(e)) => Grade::Fail(format!(
                        "run value {v} did not parse as a canonical value (compiler emit bug): {e}"
                    )),
                }
            }
            Outcome::Trap(t) => Grade::Fail(format!("expected output {payload}, but trapped: {t}")),
            Outcome::BadArtifact(e) => Grade::Fail(format!(
                "expected output {payload}, but the artifact did not build: {e}"
            )),
        },
        // SIZE-ONLY pin: measure the escaped value's canonical binary-AST ENCODING length and compare to N.
        // A parse failure on the run value is a LOUD Fail (a compiler emit bug — the decode-validity
        // invariant), never a silent pass; a trap / bad artifact where a value was expected is a Fail.
        // The measured length is ALWAYS printed (pass or fail) so a `--case` run surfaces the exact N to
        // author the pin against (the "grader prints the measured byte-len" affordance).
        GExpect::OutputByteLen(want) => match outcome {
            Outcome::Value(v, _) => match value_encoding_byte_len(v) {
                Ok(got) => {
                    eprintln!("grade: output-byte-len: measured {got} bytes (run value {v})");
                    if got as u64 == *want {
                        Grade::Pass
                    } else {
                        Grade::Fail(format!(
                            "expected output-byte-len {want}, got {got} (canonical binary-AST \
                             encoding of run value {v})"
                        ))
                    }
                }
                Err(e) => Grade::Fail(format!(
                    "run value {v} did not parse as a canonical value (compiler emit bug): {e}"
                )),
            },
            Outcome::Trap(t) => {
                Grade::Fail(format!("expected output-byte-len {want}, but trapped: {t}"))
            }
            Outcome::BadArtifact(e) => Grade::Fail(format!(
                "expected output-byte-len {want}, but the artifact did not build: {e}"
            )),
        },
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
        GExpect::Error(..) | GExpect::Warning(..) => Grade::Todo(
            "compile-outcome expectation is graded from the diagnostic, not the run".into(),
        ),
    }
}

/// Grade an `(expect-error CODE msg*)` against the compile outcome — running an ill-formed program is a
/// Fail (miscompile); the right CODE with the message containing EVERY pinned substring is a Pass; a
/// DIFFERENT code is Todo. `msgs` is the list of `(message …)` substrings (AND — all required; empty =
/// code-only).
///
/// `exact` = the case's `(exact-code)` opt-in (C1 diagnostic-quality fence): when SET, a DIFFERENT/uncoded
/// code FAILs instead of the default lenient Todo — so a case can FENCE that the compiler emits EXACTLY this
/// code (an error-masking regression, where the subject's own CDZ code is masked by a downstream uncoded
/// decline, becomes a red). Default `false` preserves the "never false-fail a novel decline" leniency for
/// the ~60 uncoded/capability declines; only opted-in cases demand the exact code.
pub fn grade_compile_error(
    compiled: bool,
    diag: &str,
    want: &str,
    msgs: &[String],
    not_msgs: &[String],
    exact: bool,
) -> Grade {
    if compiled {
        return Grade::Fail(format!(
            "expected compile error {want} but the program COMPILED (miscompile)"
        ));
    }
    let (got, _first_message) = first_error_diag(diag);
    match got {
        Some(code) if code == want => {
            // Search each `(message …)` phrase across ALL diagnostics carrying `want` (not just the first),
            // so a case whose asserted phrases are SPLIT across multiple same-code diagnostics — e.g. two
            // CDZ0101s for a `(Qty widget meter)` bad-inner + bad-unit, each anchoring its own position —
            // grades correctly. A phrase passes if it appears in SOME same-code diagnostic; a `(not …)`
            // absence pin fails if it appears in ANY. For a single same-code diagnostic (the common case)
            // this is identical to the old first-diagnostic check.
            let messages = same_code_messages(diag, want);
            if let Some(p) = msgs
                .iter()
                .find(|p| !messages.iter().any(|m| m.contains(p.as_str())))
            {
                return Grade::Fail(format!(
                    "error {want} but no {want} diagnostic message contains {p:?} (searched {} same-code diag(s))",
                    messages.len()
                ));
            }
            // seq-29 message-ABSENCE: a `(not "phrase")` pin fails if ANY same-code diagnostic CONTAINS it.
            if let Some(p) = not_msgs
                .iter()
                .find(|p| messages.iter().any(|m| m.contains(p.as_str())))
            {
                return Grade::Fail(format!(
                    "error {want} but a {want} diagnostic message unexpectedly contains {p:?}"
                ));
            }
            Grade::Pass
        }
        // A DIFFERENT/uncoded code: default = lenient Todo (refused-to-confirm; never false-fail a novel
        // decline). With the `(exact-code)` opt-in = FAIL (fence the exact code — catch an error-masking
        // regression where `want`'s own diagnostic is masked by a downstream uncoded/wrong-code decline).
        other if exact => Grade::Fail(format!(
            "expected EXACTLY {want} (exact-code fence), but refused with {other:?} — an error-masking or wrong-code regression"
        )),
        _ => Grade::Todo(format!("refused, but not with {want} (got {got:?})")),
    }
}

/// Grade an `(expect-warning CODE msg?)` against the compile outcome — the SEVERITY-warning companion of
/// `grade_compile_error`. The program must COMPILE (a warning accompanies a produced artifact; a refusal is
/// a Fail — the expected warning never got the chance to fire), AND the diagnostic must carry a `warning
/// [CODE]` with that exact code (+ optional message substring). Distinct from the presence-only `(warns …)`
/// clause: this is a first-class outcome kind, so it composes with a `(count N)` for the exact-count warning
/// cases. A DIFFERENT/absent warning code is `Todo` (refused-to-confirm), never a false pass.
pub fn grade_compile_warning(
    compiled: bool,
    diag: &str,
    want: &str,
    msgs: &[String],
    not_msgs: &[String],
) -> Grade {
    if !compiled {
        return Grade::Fail(format!(
            "expected the program to COMPILE with warning {want} but the compiler REFUSED it"
        ));
    }
    let emitted = collect_warnings(diag);
    // A matching warning = the pinned code with EVERY positive substring present.
    match emitted
        .iter()
        .find(|(c, m)| c == want && msgs.iter().all(|p| m.contains(p.as_str())))
    {
        // seq-29 message-ABSENCE: the matched warning must NOT contain any `(not "phrase")` substring.
        Some((_, m)) => match not_msgs.iter().find(|p| m.contains(p.as_str())) {
            Some(p) => Grade::Fail(format!(
                "warning {want} but message {m:?} unexpectedly contains {p:?}"
            )),
            None => Grade::Pass,
        },
        None => Grade::Todo(format!(
            "compiled, but warning {want}{} not among {emitted:?}",
            if msgs.is_empty() {
                String::new()
            } else {
                format!(" (~ {msgs:?})")
            }
        )),
    }
}

/// One STRUCTURED diagnostic fault, parsed from rcdzc's machine-readable diagnostics wire (the
/// `rcdzc::sidecar` `KIND_DIAGNOSTICS` artifact, also what `cdz check --json` projects). This is the
/// grade-path counterpart of `rcdzc::abi::Diagnostic`: it lets a corpus `(error …)` / `(warning …)` case
/// assert DIAGNOSTIC QUALITY — a structural fix, its verified flag, an exact fault count, the severity —
/// not just the code + a message substring (the capability the operator greenlit so the corpus can
/// "express fixes", unblocking the diagnostic-quality tests' migration off `rcdzc/tests.rs`).
///
/// The wire is one fault per line, EIGHT TAB-separated columns (see `rcdzc/src/sidecar.rs` `Query::
/// Diagnostics`): `severity⇥code⇥node⇥fix-kind⇥fix-node⇥fix-repl⇥fix-verified⇥message`, with `-` for any
/// absent field (uncoded/unanchored/no-fix) and `message` LAST (a free-text remainder after seven tabs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagFault {
    /// `Error` denies the component; `Warning` accompanies a produced one. From the wire's `error`/`warning`.
    pub severity: Severity,
    /// The stable `CDZ####` code, or `None` for an uncoded decline (wire `-`).
    pub code: Option<String>,
    /// The anchored AST node index the diagnostic is about, or `None` if unanchored (wire `-`).
    pub node: Option<u32>,
    /// The structural repair the diagnostic carries, or `None` when no fix is proposed (wire fix-kind `-`).
    pub fix: Option<DiagFaultFix>,
    /// The human message (the free-text last column).
    pub message: String,
}

/// The structural repair a [`DiagFault`] carries — the grade-path projection of `rcdzc::abi::DiagnosticFix`.
/// The `kind` is STRUCTURAL (matching the ABI's `FixKind`), spelled exactly as the wire emits it; the
/// semantic flavor of a fix (a coercion, a rename, …) lives in `replacement`/the message, not in `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagFaultFix {
    /// The structural edit kind, verbatim from the wire: `replace` | `insert` | `wrap` | `delete`.
    pub kind: String,
    /// The AST node index the edit targets, or `None` (wire `-`).
    pub node: Option<u32>,
    /// The edit's surface payload — the spelling to substitute (`replace`), the child form(s) to append
    /// (`insert`), or the wrap text with a `…` hole (`wrap`). Empty-ish for `delete`.
    pub replacement: String,
    /// `true` iff the compiler PROVED the fix correct (wire `verified`); `false` for a heuristic (wire
    /// `heuristic`) an agent should confirm before applying.
    pub verified: bool,
}

/// A diagnostic's severity — the grade-path mirror of `rcdzc::abi::Severity`. An error DENIES the produced
/// component; a warning ACCOMPANIES one. A corpus case reads failure-ness from this, not from the
/// diagnostic's kind (reject/decline/trap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Parse rcdzc's structured-diagnostics wire (the `KIND_DIAGNOSTICS` artifact) into [`DiagFault`]s. Each
/// non-empty line is split into its eight TAB columns (the message may itself contain no tab — it is the
/// remainder after seven splits); a line with fewer than eight columns, or an unrecognized severity token,
/// is SKIPPED (defensive — the grader treats a malformed line as no fault rather than erroring). A `-` in
/// any optional column decodes to `None`/no-fix; the fix columns decode to a [`DiagFaultFix`] only when the
/// fix-kind column is not `-`.
pub fn parse_diagnostics(wire: &[u8]) -> Vec<DiagFault> {
    // Decode the KIND_DIAGNOSTICS wire (binary-AST, seq-254/seq-284: binary-AST is THE data-exchange
    // format) via the ONE codec `cadenza_compile_abi::decode_diagnostics`, then project each
    // `abi::Diagnostic` onto the grade-path `DiagFault`. The binary wire is a SUPERSET of the old 8-tab
    // text: it preserves the human `label` (which the grader does NOT assert, so we drop it), keeps
    // newlines uncollapsed in message/replacement, and spells `InsertInto` as `insert-into` (the tab wire
    // used `insert`) — the corpus `(fix (kind …))` cases already author `insert-into`, so grading aligns.
    cadenza_compile_abi::decode_diagnostics(wire)
        .into_iter()
        .map(|d| DiagFault {
            severity: match d.severity {
                cadenza_compile_abi::Severity::Error => Severity::Error,
                cadenza_compile_abi::Severity::Warning => Severity::Warning,
            },
            code: d.code,
            node: d.node,
            fix: d.fix.map(|f| DiagFaultFix {
                kind: fix_kind_wire_name(f.kind).to_string(),
                node: Some(f.node),
                replacement: f.replacement,
                verified: f.verified,
            }),
            message: d.message,
        })
        .collect()
}

/// The wire spelling of a `FixKind` — mirrors `cadenza_compile_abi`'s wire encoder and the corpus
/// `(fix (kind …))` authoring (`insert-into`, not `insert`).
fn fix_kind_wire_name(k: cadenza_compile_abi::FixKind) -> &'static str {
    match k {
        cadenza_compile_abi::FixKind::Replace => "replace",
        cadenza_compile_abi::FixKind::InsertInto => "insert-into",
        cadenza_compile_abi::FixKind::Wrap => "wrap",
        cadenza_compile_abi::FixKind::Delete => "delete",
    }
}

impl DiagFault {
    /// Whether this fault has the given `severity` AND `code` — the primary selector a corpus `(error
    /// CDZ####)` / `(warning CDZ####)` quality assertion keys on.
    pub fn is(&self, severity: Severity, code: &str) -> bool {
        self.severity == severity && self.code.as_deref() == Some(code)
    }
}

/// Count the faults in `faults` with the given `severity` and `code` — the basis for a `(count N)` / `(once)`
/// assertion (e.g. "exactly one CDZ0305 dead-trap warning", which a presence-only check cannot express).
pub fn count_faults(faults: &[DiagFault], severity: Severity, code: &str) -> usize {
    faults.iter().filter(|f| f.is(severity, code)).count()
}

/// The set of CODED-ERROR codes (`error CDZ####`) in a `KIND_DIAGNOSTICS` wire — the parity key. Uncoded
/// declines (`code == None`) and warnings are excluded (see [`grade_check_parity`]).
fn coded_error_codes(wire: &[u8]) -> std::collections::BTreeSet<String> {
    parse_diagnostics(wire)
        .into_iter()
        .filter(|f| f.severity == Severity::Error)
        .filter_map(|f| f.code)
        .collect()
}

/// Check-vs-compile diagnostic PARITY (C1 diagnostic-quality): assert that `cdz check` surfaces every
/// CODED fault `cdz compile` does. Given the two `KIND_DIAGNOSTICS` wires (the compile-phase capture and a
/// `cdz check --emit-diagnostics` capture of the SAME case), returns `Some(msg)` iff some coded ERROR
/// present in `compile_diag` is ABSENT from `check_diag` — the `#7143` violation class (a coded rejection
/// silent under `cdz check` but caught under `cdz compile`, e.g. the CDZ0203 that was invisible to check in
/// a parameterized export until #7375). Returns `None` when parity holds.
///
/// SCOPE — coded ERRORS only. Two things are intentionally OUT of scope because including them would
/// false-red a check that legitimately does less work than a full compile:
/// * CODELESS not-yets (`code == None`) — `cdz check` MAY under-report these; a strict subset is fine.
/// * coded WARNINGS — `cdz check` is a rejection-surfacing pass; a warning-only analysis (e.g. the
///   CDZ0305 dead-trap sweep) that only a full compile runs need not be mirrored.
///
/// So the contract is precisely: check's coded-error set ⊇ compile's coded-error set (check must NEVER
/// MISS a coded rejection; it MAY add or under-report the excluded classes). Widening the scope to
/// warnings is a future option once the fleet's check-path is proven parity-clean.
///
/// This is the PURE set-superset mechanism (reuses [`parse_diagnostics`] to decode both wires); WHICH cases
/// opt into the parity leg (a default for coded-error cases vs an explicit `(check-parity)` facet) is a
/// wiring-layer decision for the 3-way with the `cdz check --emit-diagnostics` capture (v-nix / cdz-run).
/// On `Some`, the caller downgrades the grade to `Fail` (mirroring the `check_live_objects` → `Fail` path).
pub fn grade_check_parity(compile_diag: &[u8], check_diag: &[u8]) -> Option<String> {
    let compile_codes = coded_error_codes(compile_diag);
    let check_codes = coded_error_codes(check_diag);
    let missing: Vec<String> = compile_codes.difference(&check_codes).cloned().collect();
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "check-vs-compile parity: cdz check did not surface coded fault(s) that cdz compile rejects: {} \
         (check must never miss a coded fault — #7143 parity contract)",
        missing.join(", ")
    ))
}

/// How a corpus `(fix …)` clause matches a fault's `replacement` text: `Exact` demands equality (the
/// spelling/wrap-form the repair substitutes, order-sensitive); `Contains` demands a substring (the repair
/// merely mentions a name/arm). Both are common in the migrated tests (≈19 exact / 23 substring), so the
/// surface offers both: `(replacement "r")` = `Exact`, `(replacement-contains "s")` = `Contains`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplMatch {
    Exact(String),
    Contains(String),
}

impl ReplMatch {
    fn matches(&self, actual: &str) -> bool {
        match self {
            ReplMatch::Exact(s) => actual == s,
            ReplMatch::Contains(s) => actual.contains(s.as_str()),
        }
    }
}

/// The asserted STRUCTURAL FIX a corpus `(fix …)` clause pins on a diagnostic — each field optional, so a
/// case constrains only what it cares about (kind, replacement text, verified flag). Matches the structural
/// `DiagFaultFix` (kind ∈ {replace, insert, wrap, delete}; the semantic flavor lives in the replacement /
/// message, not the kind).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FixExpect {
    /// The structural edit kind the fix must have (wire spelling), or `None` to not constrain it.
    pub kind: Option<String>,
    /// How the fix's replacement text must match, or `None` to not constrain it.
    pub replacement: Option<ReplMatch>,
    /// The verified flag the fix must have, or `None` to not constrain it.
    pub verified: Option<bool>,
}

/// The DIAGNOSTIC-QUALITY assertions a corpus `(error …)` / `(warning …)` case pins beyond the code +
/// message: an exact fault `count`, and either a required `fix` (matched by [`FixExpect`]) or `no_fix` (the
/// fault must carry NO repair). All optional — a case asserts only what it checks. `fix` and `no_fix` are
/// mutually exclusive (a `no_fix` case that also named a `fix` is an authoring error; grading treats a
/// present fix under `no_fix` as a Fail).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiagExpect {
    pub fix: Option<FixExpect>,
    pub no_fix: bool,
    pub count: Option<u32>,
}

impl DiagExpect {
    /// Whether this asserts anything at all (else the quality grade is a no-op Pass).
    pub fn is_empty(&self) -> bool {
        self.fix.is_none() && !self.no_fix && self.count.is_none()
    }
}

/// Grade a corpus case's DIAGNOSTIC-QUALITY assertions against the parsed structured `faults` — the check
/// that lets the corpus "express fixes" (the operator-greenlit capability). `severity`/`code` select the
/// fault the assertions are about (the case's `(error CDZ####)` / `(warning CDZ####)`). Returns `Pass` when
/// every asserted facet holds, else `Fail` naming the first mismatch. An empty `DiagExpect` is a no-op Pass.
///
/// The FIX facets (`fix`/`no_fix`) apply to the FIRST fault matching `(severity, code)` — a case pins one
/// fault's repair; a multi-fault case uses `count` to bound the set. When `count` asserts `0` and none are
/// present, that is a Pass (the fault is absent as required) with no fix to check.
pub fn grade_diag_quality(
    faults: &[DiagFault],
    severity: Severity,
    code: &str,
    expect: &DiagExpect,
) -> Grade {
    if expect.is_empty() {
        return Grade::Pass;
    }
    let matching: Vec<&DiagFault> = faults.iter().filter(|f| f.is(severity, code)).collect();
    if let Some(n) = expect.count
        && matching.len() != n as usize
    {
        return Grade::Fail(format!(
            "expected {n} {severity:?} {code} fault(s), found {}",
            matching.len()
        ));
    }
    let Some(fault) = matching.first() else {
        // No matching fault. If the case asserted exactly zero, that already passed the count check above;
        // otherwise a fix/no_fix assertion has nothing to grade against — a real mismatch.
        if expect.count == Some(0) {
            return Grade::Pass;
        }
        return Grade::Fail(format!("expected a {severity:?} {code} fault, found none"));
    };
    if expect.no_fix
        && let Some(f) = &fault.fix
    {
        return Grade::Fail(format!(
            "expected NO fix on {code}, but a {:?} fix was proposed",
            f.kind
        ));
    }
    if let Some(want) = &expect.fix {
        let Some(got) = &fault.fix else {
            return Grade::Fail(format!("expected a fix on {code}, but none was proposed"));
        };
        if let Some(k) = &want.kind
            && &got.kind != k
        {
            return Grade::Fail(format!(
                "expected fix kind {k:?} on {code}, got {:?}",
                got.kind
            ));
        }
        if let Some(rm) = &want.replacement
            && !rm.matches(&got.replacement)
        {
            return Grade::Fail(format!(
                "fix replacement {:?} on {code} does not match {rm:?}",
                got.replacement
            ));
        }
        if let Some(v) = want.verified
            && got.verified != v
        {
            return Grade::Fail(format!(
                "expected fix verified={v} on {code}, got verified={}",
                got.verified
            ));
        }
    }
    Grade::Pass
}

/// §1 of `DESIGN-diagnostic-quality-rubric.md` — the GLOBALLY-forbidden message phrases (future-promise /
/// deferral framing + internal-implementation leak). A coded diagnostic message containing ANY of these
/// (matched case-insensitively at a WORD BOUNDARY — see [`contains_word_ci`]) fails the C1 lint. The set is
/// grounded false-positive-free on the current corpus (the doc's Grounding section): the deferral phrases
/// live only in Rust source comments, and `unsupported`/`not supported`/`trap` are DELIBERATELY carved out
/// (honest CDZ0900 / CDZ0309 semantics). `None`/`Some` were the calibration pair and are now CARVED OUT too
/// (v-diagnostics full-corpus validation, #7852): they are Cadenza's OWN `Option` constructors, named in
/// golden messages ("wrap the value in `Some`", did-you-mean candidates), not Rust leaks — a genuine leak
/// would be `Option::None` syntax, not the bare words, so they are not linted at all.
const C1_FORBIDDEN_PHRASES: &[&str] = &[
    // §1a future-promise / deferral (a decline states a PERMANENT fact, never promises a future version)
    "not yet",
    "unimplemented",
    "WIP",
    "TODO",
    "for now",
    "coming soon",
    "will be supported",
    "will support",
    "later increment",
    // §1b internal-implementation leak (a user must never see Rust-internal vocabulary)
    "internal error",
    "ICE",
    "panicked",
    "panic!",
    "compiler bug",
    "unreachable!",
    // NOTE: `unwrap` is NOT here — it is a Cadenza SURFACE operation (`(unwrap …)`, 81× in the corpus), so
    // the bare word (e.g. CDZ0202's golden "unwrap the nominal to compare") is exempt. Only the Rust CALL
    // form is a leak — see `C1_FORBIDDEN_CALL_SYNTAX` (#7921).
];

/// §1b CALL-SYNTAX leaks — matched as a PLAIN case-insensitive substring (NOT word-boundaried), because
/// these are call fragments that legitimately abut a receiver/`::` (a real leak `x.unwrap()` /
/// `Option::unwrap()` must match). Currently just the Rust `unwrap` call: `unwrap(` or `.unwrap` — the
/// panic-y `Option::unwrap()` an internal-error message might leak. The BARE word `unwrap` is a Cadenza
/// operation (see `C1_FORBIDDEN_PHRASES`'s note), so `(unwrap x)` / "unwrap the nominal" prose do NOT match
/// (no `(`/`.` immediately after `unwrap`). Scoping resolved in `DESIGN-diagnostic-quality-rubric.md` §1b (#7921).
const C1_FORBIDDEN_CALL_SYNTAX: &[&str] = &["unwrap(", ".unwrap"];

/// Case-insensitive, WORD-BOUNDARIED substring test — the `\b…\b` match §1 requires so a forbidden phrase
/// does not false-trip inside a larger word (the doc's `None` ⊂ `Nonesuch` case: `contains_word_ci("Nonesuch",
/// "None")` is `false`). A match is a word match iff the char immediately before its start and immediately
/// after its end are both non-alphanumeric (or the string edge). Phrases with internal spaces (`not yet`)
/// or a trailing `!` (`panic!`) are matched verbatim; the boundary check only guards the alphanumeric flanks.
fn contains_word_ci(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let h = hay.to_lowercase();
    let n = needle.to_lowercase();
    let mut from = 0;
    while let Some(rel) = h[from..].find(&n) {
        let i = from + rel;
        let j = i + n.len();
        let before_ok = i == 0 || !h[..i].chars().next_back().unwrap().is_alphanumeric();
        let after_ok = j == h.len() || !h[j..].chars().next().unwrap().is_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        from = i + 1;
    }
    false
}

/// The C1 GENERAL diagnostic-quality lint (`(diagnostic-quality)` opt-in) — assert every emitted CODED
/// diagnostic's message contains NO globally-forbidden phrase (`DESIGN-diagnostic-quality-rubric.md` §1:
/// future-promise/deferral + internal-implementation leak).
///
/// The §2 per-code required-token check was WITHDRAWN (#7856) as unsound: the `CDZ####` codes are umbrella
/// BANDS, not uniform message templates — e.g. CDZ0203 spans arity ("Box takes 1 type argument"),
/// value-vs-type, "not fully determined", guard-must-be-Bool — none of which say "expected"/"found", so a
/// per-code token requirement mass-false-reds golden messages. Message SHAPE (expected/found, did-you-mean)
/// belongs in per-case `(message …)`/`(fix …)` pins, scoped to the situation, which the corpus already
/// supports. C1's general, universally-sound assertion is the forbidden-phrase check ALONE.
///
/// Applies to CODED faults only (a codeless decline's ICE-flavored leak is caught separately by
/// `is_ice_signature`); both severities. Returns the FIRST violation as a Fail reason, else `None`.
pub fn grade_diagnostic_quality(faults: &[DiagFault]) -> Option<String> {
    for f in faults {
        let Some(code) = f.code.as_deref() else {
            continue; // codeless — out of scope (message-quality of a coded diagnostic only)
        };
        // Scan the PROSE only: strip any `(trap …)` suggested-fix STUB span first (§1-note, #7896) — a
        // `(trap "TODO: collect")` / `(trap TODO-colon-collect)` is runnable user-code-to-fill (the Cadenza
        // analogue of Rust `todo!()`), golden practice, NOT deferral prose; a forbidden token inside it
        // (CDZ0405's inline `(resume (trap TODO…) s)`) must not trip. §1 governs the compiler's prose,
        // never the user-code it suggests. (A `(fix …)` facet is already out of scope — we scan `message`.)
        let prose = strip_trap_stubs(&f.message);
        for phrase in C1_FORBIDDEN_PHRASES {
            if contains_word_ci(&prose, phrase) {
                return Some(format!(
                    "(diagnostic-quality): {code} message contains the forbidden phrase {phrase:?} \
                     (rubric §1) — {:?}",
                    f.message
                ));
            }
        }
        // CALL-SYNTAX leaks — PLAIN case-insensitive substring (not word-boundaried): the Rust `unwrap`
        // call (`unwrap(` / `.unwrap`), never the bare Cadenza `unwrap` operation.
        let prose_lc = prose.to_lowercase();
        for pat in C1_FORBIDDEN_CALL_SYNTAX {
            if prose_lc.contains(pat) {
                return Some(format!(
                    "(diagnostic-quality): {code} message contains the forbidden Rust call-syntax {pat:?} \
                     (rubric §1b — the bare word is a Cadenza op, only the call leaks) — {:?}",
                    f.message
                ));
            }
        }
    }
    None
}

/// Remove every `(trap …)` s-expression span from a diagnostic message — the §1 fix-stub carve-out
/// (#7896). A `(trap …)` in a message is a suggested-fix STUB (runnable user-code-to-fill, the Cadenza
/// `todo!()`), so a §1 forbidden token INSIDE it (e.g. `TODO` in CDZ0405's inline `(resume (trap
/// TODO-colon-collect) s)`) is legitimate and must not be scanned. Balanced-paren removal, string-aware
/// (a `)` inside a `"…"` trap payload does not close the span), covering both bare and quoted trap forms;
/// an unbalanced/trailing `(trap` strips to end (a malformed stub, still not scanned). `(trapfoo …)` (a
/// non-`trap` head with a `trap` prefix) is NOT stripped — the char after `trap` must be a non-word char.
fn strip_trap_stubs(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut rest = msg;
    while let Some(pos) = rest.find("(trap") {
        // Word-boundary: the char after `(trap` must be a non-word char (so `(traperror …)` is not a stub).
        if rest[pos + 5..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            out.push_str(&rest[..pos + 5]);
            rest = &rest[pos + 5..];
            continue;
        }
        out.push_str(&rest[..pos]); // prose before the stub is kept
        let span = &rest[pos..];
        let (mut depth, mut in_str, mut prev, mut end) = (0usize, false, '\0', span.len());
        for (j, ch) in span.char_indices() {
            if in_str {
                if ch == '"' && prev != '\\' {
                    in_str = false;
                }
            } else {
                match ch {
                    '"' => in_str = true,
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = j + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            prev = ch;
        }
        rest = &span[end..]; // the `(trap …)` span [0..end] is dropped
    }
    out.push_str(rest);
    out
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

/// The messages of EVERY `error [CODE]` diagnostic whose code == `code`, in emission order. A single
/// program can raise the same coded fault at multiple positions as SEPARATE diagnostics (e.g. `(Qty widget
/// meter)` emits two CDZ0101s — one anchoring `widget` "not a type variable", one anchoring `meter` "not a
/// unit"), and a case's `(message …)` asserts phrases that are split across them. `first_error_diag` only
/// sees the first, so a per-first-message check false-fails such a split case; this collects all same-code
/// messages so the phrase search can span them (see `grade_compile_error`).
pub fn same_code_messages(diag: &str, code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in diag.lines() {
        if let Some((_, after)) = line.split_once("error [")
            && let Some((c, rest)) = after.split_once(']')
            && c.trim() == code
        {
            out.push(
                rest.split_once(": ")
                    .map(|(_, m)| m)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            );
        }
    }
    out
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
    let mut live_objects_per_call: Option<Vec<u32>> = None;
    let mut no_other_errors = false;
    let mut no_diagnostic: Vec<String> = Vec::new();
    let mut diagnostic_quality = false;
    let mut diagnostic_quality_opt_out = false;

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
            // `(live-objects <N>…)` — the post-run heap-balance the case asserts (each N a string leaf). A
            // `(live-objects known-leak <N>…)` marker prefixes the counts with the literal `known-leak`.
            // ONE count = uniform (every call == N); 2+ counts = PER-CALL positional (call i == Ni). Both
            // forms may carry the known-leak marker. `live_objects` gets the FIRST count (uniform/direct-gate
            // path); `live_objects_per_call` gets the whole list when 2+ (the wasm nix path's per-call check).
            Some("live-objects") => {
                let items = a.as_form(clause, "live-objects").unwrap_or(&[]);
                let mut leaves: Vec<String> =
                    items.iter().filter_map(|&id| str_leaf(&a, id)).collect();
                if leaves.first().map(String::as_str) == Some("known-leak") {
                    live_objects_known_leak = true;
                    leaves.remove(0);
                }
                let counts: Vec<u32> = leaves
                    .iter()
                    .filter_map(|s| s.trim().parse::<u32>().ok())
                    .collect();
                live_objects = counts.first().copied();
                if counts.len() >= 2 {
                    live_objects_per_call = Some(counts);
                }
            }
            // `(no-other-errors)` — the bare case-level no-cascade flag (shredded from the case clause).
            Some("no-other-errors") => no_other_errors = true,
            // `(diagnostic-quality)` — the bare C1 opt-in marker (assert every coded diagnostic meets §1+§2).
            Some("diagnostic-quality") => diagnostic_quality = true,
            // `(no-diagnostic-quality)` — the C1 opt-OUT escape hatch (suppress the default-on §1 lint).
            Some("no-diagnostic-quality") => diagnostic_quality_opt_out = true,
            // `(no-diagnostic "phrase")` — a case-level program-scoped cross-kind absence pin (one per form,
            // repeatable). Read the phrase as the first string leaf; ignore a malformed/empty clause.
            Some("no-diagnostic") => {
                if let Some(phrase) = a
                    .as_form(clause, "no-diagnostic")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| str_leaf(&a, id))
                {
                    no_diagnostic.push(phrase);
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
        live_objects_per_call,
        no_other_errors,
        no_diagnostic,
        diagnostic_quality,
        diagnostic_quality_opt_out,
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
    let mut exact_code = false;
    let mut expect: Option<GExpect> = None;
    // The diagnostic-QUALITY facets (`(fix …)` / `(no-fix)` / `(count N)` / `(once)`) that pin more than the
    // code + message on an `(expect-error …)` / `(expect-warning …)` case — accumulated into a `DiagExpect`
    // below. Absent clauses leave the facet unconstrained (an all-absent `DiagExpect` grades as a no-op Pass).
    let mut diag_fix: Option<FixExpect> = None;
    let mut diag_no_fix = false;
    let mut diag_count: Option<u32> = None;
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
            // `(exact-code)` — C1 fence: demand the compiler emit EXACTLY this `(expect-error …)` code (a
            // different/uncoded refusal FAILs, not the lenient Todo). See `grade_compile_error`.
            Some("exact-code") => exact_code = true,
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
            // `(expect-output-byte-len N)` — the size-only pin (leaf = the decimal N as text).
            Some("expect-output-byte-len") => {
                expect = a
                    .as_form(child, "expect-output-byte-len")
                    .and_then(|t| t.first().copied())
                    .and_then(|vid| str_leaf(a, vid))
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .map(GExpect::OutputByteLen);
            }
            Some("expect-error") => {
                if let Some(t) = a.as_form(child, "expect-error") {
                    let code = t.first().copied().and_then(|id| str_leaf(a, id));
                    // code is leaf[0]; each remaining bare STRING leaf is a required message substring (AND),
                    // each `(not "phrase")` sub-form is a required-ABSENCE substring (seq-29).
                    let msgs: Vec<String> =
                        t.iter().skip(1).filter_map(|&id| str_leaf(a, id)).collect();
                    let not_msgs: Vec<String> =
                        t.iter().skip(1).filter_map(|&id| not_leaf(a, id)).collect();
                    if let Some(code) = code {
                        expect = Some(GExpect::Error(code, msgs, not_msgs));
                    }
                }
            }
            Some("expect-warning") => {
                if let Some(t) = a.as_form(child, "expect-warning") {
                    let code = t.first().copied().and_then(|id| str_leaf(a, id));
                    let msgs: Vec<String> =
                        t.iter().skip(1).filter_map(|&id| str_leaf(a, id)).collect();
                    let not_msgs: Vec<String> =
                        t.iter().skip(1).filter_map(|&id| not_leaf(a, id)).collect();
                    if let Some(code) = code {
                        expect = Some(GExpect::Warning(code, msgs, not_msgs));
                    }
                }
            }
            // `(count N)` — the fault-count the `(severity, code)` set must match exactly; `(once)` is the
            // common `(count 1)` shorthand.
            Some("count") => {
                diag_count = a
                    .as_form(child, "count")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| str_leaf(a, id))
                    .and_then(|s| s.trim().parse::<u32>().ok());
            }
            Some("once") => diag_count = Some(1),
            // `(no-fix)` — the matched fault must carry NO repair (mutually exclusive with `(fix …)`).
            Some("no-fix") => diag_no_fix = true,
            // `(fix (kind K)? (replacement R)? / (replacement-contains S)? (verified|unverified)?)`.
            Some("fix") => diag_fix = Some(decode_fix_expect(a, child)),
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
    // Fold the diagnostic-quality facets into a `DiagExpect`, or `None` when the case pinned none (the
    // common code+message-only form) — `None` and an empty `DiagExpect` both grade as a no-op quality Pass,
    // but `None` keeps the shredded `test-run.ast` clause-free.
    let diag = {
        let d = DiagExpect {
            fix: diag_fix,
            no_fix: diag_no_fix,
            count: diag_count,
        };
        (!d.is_empty()).then_some(d)
    };
    Some(GTrial {
        call,
        expect: expect?,
        diag,
        exact_code,
    })
}

/// Decode a `(fix (kind K)? (replacement R)? / (replacement-contains S)? (verified|unverified)?)` clause
/// into a [`FixExpect`]. Each sub-clause is optional (an absent facet is unconstrained); `(replacement R)`
/// pins an EXACT match and `(replacement-contains S)` a SUBSTRING (mutually exclusive — the last one wins if
/// both appear, an authoring slip). A bare `(fix)` constrains nothing but still requires SOME fix be present.
fn decode_fix_expect(a: &Arenas, id: StructId) -> FixExpect {
    let mut fx = FixExpect::default();
    for &child in a.as_form(id, "fix").unwrap_or(&[]) {
        match a.head_name(child) {
            Some("kind") => {
                fx.kind = a
                    .as_form(child, "kind")
                    .and_then(|t| t.first().copied())
                    .and_then(|cid| str_leaf(a, cid));
            }
            Some("replacement") => {
                fx.replacement = a
                    .as_form(child, "replacement")
                    .and_then(|t| t.first().copied())
                    .and_then(|cid| str_leaf(a, cid))
                    .map(ReplMatch::Exact);
            }
            Some("replacement-contains") => {
                fx.replacement = a
                    .as_form(child, "replacement-contains")
                    .and_then(|t| t.first().copied())
                    .and_then(|cid| str_leaf(a, cid))
                    .map(ReplMatch::Contains);
            }
            Some("verified") => fx.verified = Some(true),
            Some("unverified") => fx.verified = Some(false),
            _ => {}
        }
    }
    fx
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

/// Read the phrase from a `(not "phrase")` sub-form (seq-29 message-ABSENCE pin), or `None` for any other
/// node — lets the `expect-*` decoders partition their leaves into bare-string message substrings vs the
/// `(not …)` absence substrings.
fn not_leaf(a: &Arenas, id: StructId) -> Option<String> {
    a.as_form(id, "not")
        .and_then(|t| t.first().copied())
        .and_then(|vid| str_leaf(a, vid))
}

/// Canonicalize an `output` value TEXT to the canonical s-expr render of its VALUE, via the canonical
/// reader/printer (`cadenza_syntax::sexpr`) — NOT a hand-rolled paren/`#`/quote scan (operator directive
/// 2026-09-01: "use our canonical parser … or exchange a binary AST"). A top-level `(: <value> <Type>)`
/// ascription is stripped to `<value>` (type-blind, matching the historical `expected_value`); a bare
/// value is returned as its own canonical print. Both the corpus expected payload and the run value pass
/// through this, so bare-vs-annotated and any rendering variance normalize away structurally.
///
/// `Err` if the text does not parse as a single canonical s-expr — surfaced LOUDLY by the caller (a corpus
/// authoring error on the expected side, a compiler emit bug on the run-value side), never a silent pass.
///
/// PUBLIC as the SINGLE SOURCE for output-value canonicalization: the in-process `xtask gate --check`
/// (the authoritative merge gate, until the gateCheckNix swap) calls THIS from its own `grade_trial`
/// Output arm instead of a divergent local copy — the divergence that produced the #7273 fleet red. Keep
/// it the one canonical value-canon both graders share.
pub fn canonical_output_value(text: &str) -> Result<String, String> {
    // Normalize classic name-head compounds ((tuple …)/(list …)/(record …)/(map …)/(set …)) to native
    // `#ctor` form BEFORE comparing, so value identity is SPELLING-INVARIANT: `(tuple 1 2)` ≡ `#tuple(1 2)`
    // — the SAME binary AST, just two surface renderings (concierge ruling 2026-09-02; binary-AST is THE
    // value, the surface spelling is a rendering choice). This makes the value compare robust to a backend
    // rendering a tuple as `#tuple(…)` (rust, post value-doc flip) vs `(tuple …)` (the wasm corpus-grade
    // render + legacy corpus outputs) — the grade compares the VALUE, not its spelling. Applied to BOTH the
    // expected and the run value (below), so the compare stays symmetric; it can only make two SPELLINGS of
    // one value equal (a genuine content diff — different elements/arity — still differs), never mask a real
    // mismatch. FAIL-SAFE: if the nativize can't parse the text, fall back to it verbatim so the `read`
    // below surfaces the real parse error exactly as before (no NEW error path introduced).
    let text = text.trim();
    let normalized =
        cadenza_syntax::sexpr::nativize_compound_source(text).unwrap_or_else(|_| text.to_string());
    let a = cadenza_syntax::sexpr::read(&normalized).map_err(|e| e.0)?;
    // Strip ALL leading top-level `(: value type)` ascriptions to the bare value. Annotation is type
    // METADATA, not value identity, so a value annotated once, twice (a redundant `(: (: v T) T)`), or not
    // at all all denote the SAME value — looping makes the value compare ANNOTATION-INVARIANT. (Fixes the
    // guarded-all double-annotation drift: a `(: (: (Some 5) (Option Int64)) (Option Int64))` expected vs a
    // single-annotated `(: (Some 5) (Option Int64))` ran now both reduce to `(Some 5)`.) A genuine value
    // never heads with the reserved `:` ascription operator, so this only unwraps real ascriptions.
    let mut value_id = a.root;
    while a.head_name(value_id) == Some(":") {
        match children(&a, value_id).first().copied() {
            Some(child) => value_id = child,
            None => break, // a malformed bare `(:)` with no value child — stop, don't spin
        }
    }
    Ok(cadenza_syntax::sexpr::print_from(&a, value_id))
}

/// The byte-length of a value's CANONICAL BINARY-AST ENCODING — the number a `(output-byte-len N)` pin
/// asserts. `text` is a value render (bare or `(: v T)`); it is first canonicalized by
/// [`canonical_output_value`] (ascription-stripped, `#ctor`-nativized, reprinted) so the measured length
/// is ANNOTATION- and SPELLING-invariant, then re-read and encoded via the shared `codec` (the binary-AST
/// data-exchange format). Two spellings of the same value therefore measure identically; a genuine content
/// or arity difference measures differently. `Err` (propagated from `canonical_output_value` or a re-read
/// failure) is surfaced LOUDLY by the caller — never a silent pass. The SINGLE SOURCE both graders (the
/// in-process `xtask gate` and the exec bins) call, keeping the two-mechanism paths verdict-identical.
pub fn value_encoding_byte_len(text: &str) -> Result<usize, String> {
    let canon = canonical_output_value(text)?;
    let a = cadenza_syntax::sexpr::read(&canon).map_err(|e| e.0)?;
    Ok(codec::encode(&a).len())
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

    /// Build a binary-AST `KIND_DIAGNOSTICS` wire (seq-254) from compact fault specs — the round-trip input
    /// for the `parse_diagnostics` tests, via the ONE codec `cadenza_compile_abi::encode_diagnostics`. Each
    /// spec is `(severity, code?, node?, fix?, message)` where `fix` is `(kind, node, replacement, verified)`.
    #[allow(clippy::type_complexity)]
    fn bin_wire(
        faults: &[(
            cadenza_compile_abi::Severity,
            Option<&str>,
            Option<u32>,
            Option<(cadenza_compile_abi::FixKind, u32, &str, bool)>,
            &str,
        )],
    ) -> Vec<u8> {
        let diags: Vec<cadenza_compile_abi::Diagnostic> = faults
            .iter()
            .map(
                |(sev, code, node, fix, msg)| cadenza_compile_abi::Diagnostic {
                    severity: *sev,
                    code: code.map(|c| c.to_string()),
                    node: *node,
                    message: msg.to_string(),
                    fix: fix.map(|(k, n, r, v)| cadenza_compile_abi::DiagnosticFix {
                        label: String::new(),
                        kind: k,
                        node: n,
                        replacement: r.to_string(),
                        verified: v,
                    }),
                },
            )
            .collect();
        cadenza_compile_abi::encode_diagnostics(&diags)
    }

    /// seq-29 message-ABSENCE `(not "phrase")`: an error whose diagnostic does NOT contain a forbidden
    /// phrase PASSES; one that DOES contain it FAILS (even with the right code + all positive substrings).
    #[test]
    fn grade_message_not_contains_absence_assertion() {
        // expect-error: absence holds → Pass; present → Fail.
        let e = "cdz: error [CDZ0201] (node 4): malformed record separator";
        assert!(matches!(
            grade_compile_error(
                false,
                e,
                "CDZ0201",
                &["malformed".into()],
                &["internal".into()],
                false
            ),
            Grade::Pass
        ));
        assert!(matches!(
            grade_compile_error(false, e, "CDZ0201", &[], &["separator".into()], false),
            Grade::Fail(_)
        ));
        // expect-warning: matched code+positive but forbidden phrase present → Fail.
        assert!(matches!(
            grade_compile_warning(
                true,
                "warning [CDZ0305] (node 3): this trap is unreachable",
                "CDZ0305",
                &["unreachable".into()],
                &["trap".into()]
            ),
            Grade::Fail(_)
        ));
        assert!(matches!(
            grade_compile_warning(
                true,
                "warning [CDZ0305] (node 3): this trap is unreachable",
                "CDZ0305",
                &["unreachable".into()],
                &["internal".into()]
            ),
            Grade::Pass
        ));
    }

    #[test]
    fn canonical_output_value_strips_ascription_to_scalar_compound_and_string() {
        // The canonical reader/printer replaces the old hand-rolled `expected_value` scan: a `(: v T)`
        // ascription is stripped type-blind to `v`'s canonical print; a bare value prints itself.
        assert_eq!(canonical_output_value("(: 42 Int64)").unwrap(), "42");
        // A classic name-head `(tuple …)` is NORMALIZED to native `#tuple(…)` (spelling-invariant value
        // identity — same binary AST; see the ctor-head normalization in canonical_output_value).
        assert_eq!(
            canonical_output_value("(: (tuple 0 7) (Tuple Int64 Int64))").unwrap(),
            "#tuple(0 7)"
        );
        assert_eq!(
            canonical_output_value("(: \"parse error\" String)").unwrap(),
            "\"parse error\""
        );
        assert_eq!(canonical_output_value("bare").unwrap(), "bare");
    }

    #[test]
    fn canonical_output_value_is_annotation_invariant() {
        // Annotation is type metadata, not value identity: a value annotated ZERO, ONE, or MORE times all
        // reduce to the same bare value. The guarded-all double-annotation drift (#7329 fold): a
        // `(: (: v T) T)` expected and a single-annotated `(: v T)` ran must BOTH canonicalize to `v`.
        let bare = canonical_output_value("(Some 5)").unwrap();
        let single = canonical_output_value("(: (Some 5) (Option Int64))").unwrap();
        let double =
            canonical_output_value("(: (: (Some 5) (Option Int64)) (Option Int64))").unwrap();
        assert_eq!(bare, "(Some 5)");
        assert_eq!(single, "(Some 5)");
        assert_eq!(double, "(Some 5)");
        assert_eq!(single, double); // the guarded-all case: expected(double) == ran(single) after canon
    }

    #[test]
    fn canonical_output_value_handles_hashtag_compound_values() {
        // `#`-prefixed native compounds no longer miscut at the first inner space (the corpus-rust-05-1321
        // nested-record bug: the old scan truncated `#record((= …) …)` to `#record((=`). The canonical
        // reader parses the whole compound and re-prints it.
        assert_eq!(
            canonical_output_value(
                "(: #record((= first (Ok 7)) (= second (Err b\"no\"))) (record (first (Result Int64 String)) (second (Result Int64 Bytes))))"
            ).unwrap(),
            "#record((= first (Ok 7)) (= second (Err b\"no\")))"
        );
        assert_eq!(
            canonical_output_value("(: #list(1 2 3) (List Int64))").unwrap(),
            "#list(1 2 3)"
        );
        assert_eq!(
            canonical_output_value("(: #tuple(127 -128) (Tuple Int64 Int64))").unwrap(),
            "#tuple(127 -128)"
        );
    }

    /// CTOR-HEAD SPELLING-INVARIANCE (concierge 2026-09-02): a classic name-head compound and its native
    /// `#ctor` twin are the SAME value → they canonicalize IDENTICALLY, so the grade passes regardless of
    /// whether a backend renders `(tuple …)` (wasm corpus-grade) or `#tuple(…)` (rust post value-doc flip).
    /// A genuine CONTENT difference still differs (not masked).
    #[test]
    fn canonical_output_value_normalizes_classic_compound_head_to_native() {
        for (classic, native) in [
            ("(tuple -3 -4 -3 -4)", "#tuple(-3 -4 -3 -4)"),
            ("(list 1 2 3)", "#list(1 2 3)"),
            (
                "(record (= first 7) (= second 9))",
                "#record((= first 7) (= second 9))",
            ),
        ] {
            let c = canonical_output_value(classic).unwrap();
            let n = canonical_output_value(native).unwrap();
            assert_eq!(
                c, n,
                "classic {classic:?} must canonicalize to native {native:?} form"
            );
        }
        // A real content diff is NOT masked by the normalization.
        assert_ne!(
            canonical_output_value("(tuple 1 2)").unwrap(),
            canonical_output_value("#tuple(1 3)").unwrap()
        );
    }

    #[test]
    fn canonical_output_value_errs_on_unparsable_text() {
        // A run value that is not a canonical s-expr is a compiler emit bug → the caller Fails loudly
        // (never a silent pass). An unterminated compound must not parse.
        assert!(canonical_output_value("#record((= a 1").is_err());
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
    fn value_encoding_byte_len_is_annotation_and_spelling_invariant() {
        // Ascription is metadata, not value identity → stripped before measuring.
        assert_eq!(
            value_encoding_byte_len("(: #list(1 2 3) (List Int64))").unwrap(),
            value_encoding_byte_len("#list(1 2 3)").unwrap()
        );
        // Two spellings of the SAME value (classic name-head vs native `#ctor`) encode to the same bytes.
        assert_eq!(
            value_encoding_byte_len("(tuple 1 2)").unwrap(),
            value_encoding_byte_len("#tuple(1 2)").unwrap()
        );
        // A genuine content/arity difference measures differently (the fence is a real size check).
        assert_ne!(
            value_encoding_byte_len("#list(1 2 3)").unwrap(),
            value_encoding_byte_len("#list(1 2 3 4 5 6)").unwrap()
        );
        // A larger list encodes to strictly more bytes than a smaller one (monotone in content — the
        // property the >64KiB escape fence relies on).
        assert!(
            value_encoding_byte_len("#list(1 2 3 4)").unwrap()
                > value_encoding_byte_len("#list(1 2)").unwrap()
        );
        // An unparsable run value surfaces LOUDLY as an Err (never a silent 0-length pass).
        assert!(value_encoding_byte_len("#list(1 2").is_err());
    }

    #[test]
    fn grade_trial_output_byte_len_pass_fail_and_wrong_outcome() {
        // Measure a value's encoding, then assert the exact N passes and N±1 fails — no hand-derived
        // constant (the codec owns the length; the grade just compares to whatever the grader measures).
        let v = "#list(1 2 3 4 5)";
        let n = value_encoding_byte_len(v).unwrap() as u64;
        assert_eq!(
            grade_trial(
                &GExpect::OutputByteLen(n),
                &Outcome::Value(v.into(), vec![])
            ),
            Grade::Pass
        );
        assert!(matches!(
            grade_trial(
                &GExpect::OutputByteLen(n + 1),
                &Outcome::Value(v.into(), vec![])
            ),
            Grade::Fail(_)
        ));
        // Spelling-invariant: the classic-head render of the SAME value passes the same pin.
        assert_eq!(
            grade_trial(
                &GExpect::OutputByteLen(n),
                &Outcome::Value("(list 1 2 3 4 5)".into(), vec![])
            ),
            Grade::Pass
        );
        // A trap / bad artifact where a value was expected is a Fail (never a hidden pass).
        assert!(matches!(
            grade_trial(&GExpect::OutputByteLen(n), &Outcome::Trap("boom".into())),
            Grade::Fail(_)
        ));
        assert!(matches!(
            grade_trial(
                &GExpect::OutputByteLen(n),
                &Outcome::BadArtifact("nope".into())
            ),
            Grade::Fail(_)
        ));
    }

    fn coded_fault(code: &str, message: &str) -> DiagFault {
        DiagFault {
            severity: Severity::Error,
            code: Some(code.to_string()),
            node: None,
            fix: None,
            message: message.to_string(),
        }
    }

    #[test]
    fn contains_word_ci_is_word_boundaried_and_case_insensitive() {
        // The doc's calibration case: `None` must NOT match inside `Nonesuch`.
        assert!(!contains_word_ci("case Nonesuch escapes", "None"));
        assert!(contains_word_ci("the value is None here", "None"));
        // Case-insensitive; multi-word phrase; trailing-`!` phrase.
        assert!(contains_word_ci("This is NOT YET reducible", "not yet"));
        assert!(contains_word_ci("hit unreachable! macro", "unreachable!"));
        // Not a false-trip inside a larger word.
        assert!(!contains_word_ci("unimplementedness", "unimplemented"));
        assert!(contains_word_ci("marked unimplemented.", "unimplemented"));
    }

    #[test]
    fn c1_lint_unwrap_is_scoped_to_rust_call_syntax_not_the_cadenza_op() {
        // CDZ0202's golden message guides the user to the Cadenza `unwrap` operation — the BARE word must
        // NOT flag (unwrap is a surface op, 81× in the corpus), only the Rust call form leaks (#7921).
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0202",
                "Age and Int64 are not comparable across the nominal boundary (unwrap the nominal to \
                 compare the underlying value)"
            )])
            .is_none(),
            "bare `unwrap` guidance is golden, not a Rust leak"
        );
        // The Cadenza `(unwrap …)` op form is exempt (space after unwrap, not `(`/`.`).
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0202",
                "unwrap the nominal with (unwrap x)"
            )])
            .is_none()
        );
        // The Rust CALL form DOES flag: `.unwrap()` (method) and `Option::unwrap(` (path).
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0201",
                "the value was x.unwrap() at runtime"
            )])
            .is_some()
        );
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0201",
                "called Option::unwrap() on a None"
            )])
            .is_some()
        );
    }

    #[test]
    fn c1_lint_trap_stub_carve_out_does_not_flag_todo_inside_a_trap() {
        // CDZ0405's real message embeds a suggested-fix STUB inline: a `(trap …)` is user-code-to-fill, so
        // its `TODO` must NOT trip §1 (#7896). Bare and quoted forms; nested parens; string-with-paren.
        let cdz0405 = "this handler does not discharge every operation its effect declares: operation \
             collect not handled — a handle must discharge its effect's whole operation set; add \
             (collect () s (resume (trap TODO-colon-collect) s))";
        assert!(
            grade_diagnostic_quality(&[coded_fault("CDZ0405", cdz0405)]).is_none(),
            "TODO inside a (trap …) fix-stub is golden, not deferral prose"
        );
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0405",
                "add (resume (trap \"TODO: collect\") s)"
            )])
            .is_none()
        );
        // A forbidden token in the PROSE (outside any trap) STILL flags — the carve-out is scoped to traps.
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0405",
                "this handler is not yet supported"
            )])
            .is_some()
        );
        // A prose TODO alongside a trap stub still flags (only the trap span is exempt).
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0405",
                "TODO: rework this; add (resume (trap \"fill\") s)"
            )])
            .is_some()
        );
        // `strip_trap_stubs` keeps surrounding prose + does not strip a non-trap `(trapfoo …)` head.
        assert_eq!(
            strip_trap_stubs("before (trap TODO) after"),
            "before  after"
        );
        assert_eq!(strip_trap_stubs("a (trapfoo x) b"), "a (trapfoo x) b");
    }

    #[test]
    fn c1_lint_flags_forbidden_phrase_only_with_carve_outs() {
        // §1 forbidden phrase on a coded diagnostic → flagged.
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0900",
                "this construct is not yet supported"
            )])
            .is_some()
        );
        // §1 internal-leak.
        assert!(
            grade_diagnostic_quality(&[coded_fault("CDZ0201", "internal error: unwrap on None")])
                .is_some()
        );
        // §2 WITHDRAWN (#7856): a coded message with NO forbidden phrase passes regardless of shape — the
        // per-code required-token check is gone (CDZ codes are umbrella bands, not templates). A CDZ0203
        // message that names arity/value-vs-type/etc. WITHOUT "expected"/"found" must NOT be flagged now.
        assert!(
            grade_diagnostic_quality(&[coded_fault("CDZ0203", "`Box` takes 1 type argument")])
                .is_none()
        );
        assert!(
            grade_diagnostic_quality(&[coded_fault("CDZ0203", "the types are incompatible")])
                .is_none()
        );
        assert!(grade_diagnostic_quality(&[coded_fault("CDZ0101", "unbound name `x`")]).is_none());
        assert!(
            grade_diagnostic_quality(&[coded_fault("CDZ9999", "a clean message with no tokens")])
                .is_none()
        );
        // The CDZ0900 carve-out: `unsupported`/`not supported` is NOT forbidden (honest semantics).
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0900",
                "the cadenza backend does not support this construct"
            )])
            .is_none()
        );
        // The None/Some carve-out (#7852): they are Cadenza's OWN Option constructors, named in golden
        // messages — must NOT be flagged (a real Rust leak would be `Option::None` syntax, not the bare word).
        assert!(
            grade_diagnostic_quality(&[coded_fault("CDZ0201", "wrap the value in `Some`")])
                .is_none()
        );
        assert!(
            grade_diagnostic_quality(&[coded_fault(
                "CDZ0101",
                "unbound name `x`; closest matches: None, Some"
            )])
            .is_none()
        );
        // A CODELESS fault is out of scope (no code → skipped), even with a leak phrase.
        let codeless = DiagFault {
            severity: Severity::Error,
            code: None,
            node: None,
            fix: None,
            message: "internal error: panicked".to_string(),
        };
        assert!(grade_diagnostic_quality(&[codeless]).is_none());
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
            grade_compile_error(
                false,
                "cdz: error [CDZ0201] (node 4): sep",
                "CDZ0201",
                &[],
                &[],
                false
            ),
            Grade::Pass
        );
        // Different code → Todo (still refused). Compiled → Fail.
        assert!(matches!(
            grade_compile_error(false, "error [CDZ0300]: x", "CDZ0201", &[], &[], false),
            Grade::Todo(_)
        ));
        assert!(matches!(
            grade_compile_error(true, "", "CDZ0201", &[], &[], false),
            Grade::Fail(_)
        ));
        // MULTI-SUBSTRING (AND): the message must contain EVERY pinned substring — a diagnostic that names
        // the rule AND both operand types passes only when all are present; a missing one fails.
        let diag = "cdz: error [CDZ0301] (node 2): no implicit conversion from Float64 to Int64";
        let all = [
            "no implicit conversion".to_string(),
            "Float64".to_string(),
            "Int64".to_string(),
        ];
        assert_eq!(
            grade_compile_error(false, diag, "CDZ0301", &all, &[], false),
            Grade::Pass
        );
        // One substring absent (UInt64 not in the message) → Fail naming the missing phrase.
        let miss = ["no implicit conversion".to_string(), "UInt64".to_string()];
        assert!(matches!(
            grade_compile_error(false, diag, "CDZ0301", &miss, &[], false),
            Grade::Fail(_)
        ));
    }

    /// A case whose `(message …)` phrases are SPLIT across MULTIPLE same-code diagnostics (a `(Qty widget
    /// meter)` bad-inner + bad-unit emits two CDZ0101s, one per position) grades correctly — each phrase is
    /// searched across ALL same-code diagnostics, not just the first. (18-units:0005 fix.)
    #[test]
    fn compile_error_grade_spans_multiple_same_code_diagnostics() {
        let two = "cdz: error [CDZ0101] (col 23): `widget` is not a type variable here\n\
                   cdz: error [CDZ0101] (col 30): `meter` is not a unit — Qty's second argument is a unit";
        let both = ["not a type variable".to_string(), "not a unit".to_string()];
        // Both phrases present across the two CDZ0101s → Pass (the first-diagnostic-only check false-failed this).
        assert_eq!(
            grade_compile_error(false, two, "CDZ0101", &both, &[], false),
            Grade::Pass
        );
        // A phrase in NEITHER same-code diagnostic → Fail.
        let absent = ["not a lifetime".to_string()];
        assert!(matches!(
            grade_compile_error(false, two, "CDZ0101", &absent, &[], false),
            Grade::Fail(_)
        ));
        // `same_code_messages` collects only the matching code (both CDZ0101 here; a CDZ0999 is ignored).
        assert_eq!(same_code_messages(two, "CDZ0101").len(), 2);
        assert_eq!(same_code_messages(two, "CDZ0999").len(), 0);
        // seq-29 absence across diags: a `(not …)` phrase present in ANY same-code diagnostic → Fail.
        assert!(matches!(
            grade_compile_error(
                false,
                two,
                "CDZ0101",
                &[],
                &["not a unit".to_string()],
                false
            ),
            Grade::Fail(_)
        ));
    }

    /// C1 exact-code fence (`(exact-code)` opt-in): a WRONG/uncoded refusal FAILs when `exact`, Todos when not.
    /// Fences an error-masking regression (the subject's own code masked by a downstream uncoded decline)
    /// that the default lenient wrong-code→Todo cannot catch.
    #[test]
    fn grade_compile_error_exact_code_fence() {
        let want = "CDZ0101";
        // The RIGHT code passes regardless of `exact`.
        let right = "cdz: error [CDZ0101] (node 2): unbound name `x`";
        assert_eq!(
            grade_compile_error(false, right, want, &[], &[], false),
            Grade::Pass
        );
        assert_eq!(
            grade_compile_error(false, right, want, &[], &[], true),
            Grade::Pass
        );
        // A DIFFERENT code: lenient (exact=false) → Todo; fenced (exact=true) → Fail.
        let wrong = "cdz: error [CDZ0201] (node 2): malformed";
        assert!(matches!(
            grade_compile_error(false, wrong, want, &[], &[], false),
            Grade::Todo(_)
        ));
        assert!(matches!(
            grade_compile_error(false, wrong, want, &[], &[], true),
            Grade::Fail(_)
        ));
        // An UNCODED masking decline (the fpr-class error-masking): lenient → Todo; fenced → Fail.
        let masked = "cdz: error: not a scalar literal";
        assert!(matches!(
            grade_compile_error(false, masked, want, &[], &[], false),
            Grade::Todo(_)
        ));
        assert!(matches!(
            grade_compile_error(false, masked, want, &[], &[], true),
            Grade::Fail(_)
        ));
        // exact does NOT override the COMPILED-miscompile Fail (a program that compiles when an error was
        // expected is still a Fail, exact or not).
        assert!(matches!(
            grade_compile_error(true, "", want, &[], &[], true),
            Grade::Fail(_)
        ));
    }

    /// `grade_compile_warning` = the severity-warning companion: COMPILE + warning-code-present is Pass; a
    /// REFUSAL (didn't compile) is Fail; a different/absent warning code is Todo (never a false pass).
    #[test]
    fn compile_warning_grade() {
        // Compiled + the warning present (+ message substring) → Pass.
        assert_eq!(
            grade_compile_warning(
                true,
                "cdz: warning [CDZ0305] (node 3): this trap is unreachable (dead code)",
                "CDZ0305",
                &["unreachable".to_string()],
                &[]
            ),
            Grade::Pass
        );
        assert_eq!(
            grade_compile_warning(
                true,
                "warning [CDZ0306] (node 1): unused binding",
                "CDZ0306",
                &[],
                &[]
            ),
            Grade::Pass
        );
        // Refused (didn't compile) → Fail (the warning never fired).
        assert!(matches!(
            grade_compile_warning(false, "", "CDZ0305", &[], &[]),
            Grade::Fail(_)
        ));
        // Compiled but a DIFFERENT/absent warning code → Todo (refused-to-confirm, not a false pass).
        assert!(matches!(
            grade_compile_warning(
                true,
                "warning [CDZ0999] (node 2): other",
                "CDZ0305",
                &[],
                &[]
            ),
            Grade::Todo(_)
        ));
        // Right code, message substring MISSING → Todo (the code matched but the phrase check failed).
        assert!(matches!(
            grade_compile_warning(
                true,
                "warning [CDZ0305] (node 3): dead trap",
                "CDZ0305",
                &["nope".to_string()],
                &[]
            ),
            Grade::Todo(_)
        ));
    }

    #[test]
    fn grade_run_decodes_and_orchestrates_an_output_byte_len_trial() {
        // Build a (test-run …) with one (expect-output-byte-len N) trial, decode it (the binary-manifest
        // path the nix exec bins use), and grade with a stub runner — proving the decode → GExpect →
        // grade_run wiring round-trips. N is measured off the value so no constant is hand-derived.
        use cadenza_syntax::ast::{Builder, Leaf};
        use std::sync::Arc;
        let v = "#list(1 2 3 4 5)";
        let n = value_encoding_byte_len(v).unwrap();
        let mut b = Builder::new();
        let s = |b: &mut Builder, t: &str| b.atom_leaf(Leaf::Str(Arc::from(t)));
        let head = b.name("test-run");
        let dh = b.name("description");
        let dv = s(&mut b, "byte-len case");
        let desc = b.list(vec![dh, dv]);
        let th = b.name("trial");
        let eh = b.name("expect-output-byte-len");
        let ev = s(&mut b, &n.to_string());
        let expect = b.list(vec![eh, ev]);
        let trial = b.list(vec![th, expect]);
        let trials_head = b.name("trials");
        let trials = b.list(vec![trials_head, trial]);
        let root = b.list(vec![head, desc, trials]);
        let bytes = codec::encode(&b.finish(root));

        let tr = decode_test_run(&bytes).expect("decodes");
        assert!(matches!(
            tr.trials.first().map(|t| &t.expect),
            Some(GExpect::OutputByteLen(m)) if *m == n as u64
        ));
        // The exact length passes.
        let res = grade_run(&tr, 0, "", None, None, |_| {
            Ok(Outcome::Value(v.into(), vec![]))
        })
        .unwrap();
        assert_eq!(res.grade, Grade::Pass);
        assert!(res.ran_a_trial);
        // A different-length value → Fail (the size-class miscompile signal).
        let res = grade_run(&tr, 0, "", None, None, |_| {
            Ok(Outcome::Value("#list(1 2 3)".into(), vec![]))
        })
        .unwrap();
        assert!(matches!(res.grade, Grade::Fail(_)));
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
        let res = grade_run(&tr, 0, "", None, None, |_| {
            Ok(Outcome::Value("42".into(), vec![]))
        })
        .unwrap();
        assert_eq!(res.grade, Grade::Pass);
        assert!(res.ran_a_trial);
        // A wrong value → Fail.
        let res = grade_run(&tr, 0, "", None, None, |_| {
            Ok(Outcome::Value("41".into(), vec![]))
        })
        .unwrap();
        assert!(matches!(res.grade, Grade::Fail(_)));

        // COMPILE-FAILURE grading of an output case (compile_status != 0, nothing runs):
        let never = |_: &GTrial| -> Result<Outcome> { panic!("must not run a declined case") };
        // An ICE-signature code-less error (a compiler BUG) → FAIL, never a hidden todo.
        let res = grade_run(
            &tr,
            1,
            "cdz: error: parameter reference has no local slot",
            None,
            None,
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
            None,
            None,
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
            None,
            None,
            never,
        )
        .unwrap();
        assert!(
            matches!(res.grade, Grade::Todo(_)),
            "coded decline → todo: {:?}",
            res.grade
        );
        // A silent decline (no error line at all) stays Todo.
        let res = grade_run(&tr, 1, "", None, None, never).unwrap();
        assert!(
            matches!(res.grade, Grade::Todo(_)),
            "silent decline → todo: {:?}",
            res.grade
        );
    }

    #[test]
    fn grade_run_declined_case_with_host_calls_stays_todo_not_spurious_fail() {
        // fpr3-class regression guard: a should-RUN output case that carries a `(host-calls …)` clause but
        // SOUNDLY DECLINES (coded CDZ, no run) must grade TODO — NOT be spuriously FAILed by the host-call
        // check comparing `observed []` (nothing ran) against the idealistic expected sequence. The host-call
        // check is gated on `compiled` for exactly this. (Operator corpus policy: a sound decline is a todo.)
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
        let ev = s(&mut b, "(: 2 Int64)");
        let expect = b.list(vec![eh, ev]);
        let trial = b.list(vec![th, expect]);
        let trials_head = b.name("trials");
        let trials = b.list(vec![trials_head, trial]);
        // (host-calls (op "ask.ask")) — the idealistic host-call the case WOULD make if it ran.
        let hc_head = b.name("host-calls");
        let op_head = b.name("op");
        let op_val = s(&mut b, "ask.ask");
        let op = b.list(vec![op_head, op_val]);
        let host_calls = b.list(vec![hc_head, op]);
        let root = b.list(vec![head, desc, trials, host_calls]);
        let bytes = codec::encode(&b.finish(root));
        let tr = decode_test_run(&bytes).expect("decodes");
        assert_eq!(
            tr.host_calls,
            vec!["ask.ask".to_string()],
            "host_calls parsed"
        );

        let never = |_: &GTrial| -> Result<Outcome> { panic!("must not run a declined case") };
        // DECLINED (coded CDZ0900) + a host-calls clause → TODO, not a spurious host-call-mismatch FAIL.
        let res = grade_run(
            &tr,
            1,
            "error [CDZ0900] (node 82): this handler is not reducible by the tail-resumptive fold",
            None,
            None,
            never,
        )
        .unwrap();
        assert!(
            matches!(res.grade, Grade::Todo(_)),
            "declined case with a host-calls clause → todo (not spurious host-call FAIL): {:?}",
            res.grade
        );
        // The check STILL fires for a case that actually RAN but made the WRONG calls (observed empty here) →
        // FAIL — the fix only skips the check when nothing compiled/ran, never masks a real run mismatch.
        let res = grade_run(&tr, 0, "", None, None, |_| {
            Ok(Outcome::Value("2".into(), vec![]))
        })
        .unwrap();
        assert!(
            matches!(res.grade, Grade::Fail(_)),
            "ran but made no calls vs expected [ask.ask] → fail: {:?}",
            res.grade
        );
        // A run that makes the RIGHT call → Pass.
        let res = grade_run(&tr, 0, "", None, None, |_| {
            Ok(Outcome::Value("2".into(), vec!["ask.ask".into()]))
        })
        .unwrap();
        assert_eq!(res.grade, Grade::Pass);
    }

    /// C1-PLUMBING: `grade_run` fires `grade_diag_quality` on a diag-bearing trial WHEN the structured
    /// diagnostics wire is supplied (`Some`), and SKIPS it when the wire is absent (`None`) — the switch
    /// that turns diagnostic-quality grading on without changing any case that lacks the wire.
    #[test]
    fn grade_run_fires_diag_quality_only_when_the_wire_is_present() {
        let never =
            |_: &GTrial| -> Result<Outcome> { panic!("compile-outcome case runs no trial") };
        // An (error CDZ0201) trial pinning a fix `(replacement "foo")` + `(count 1)`.
        let tr = TestRun {
            description: "diag-quality".into(),
            trials: vec![GTrial {
                call: None,
                expect: GExpect::Error("CDZ0201".into(), vec![], vec![]),
                diag: Some(DiagExpect {
                    fix: Some(FixExpect {
                        kind: None,
                        replacement: Some(ReplMatch::Exact("foo".into())),
                        verified: None,
                    }),
                    no_fix: false,
                    count: Some(1),
                }),
                exact_code: false,
            }],
            host_responses: vec![],
            host_calls: vec![],
            warns: vec![],
            live_objects: None,
            live_objects_known_leak: false,
            live_objects_per_call: None,
            no_other_errors: false,
            no_diagnostic: vec![],
            diagnostic_quality: false,
            diagnostic_quality_opt_out: false,
        };
        // The compile refused with the right code (so grade_compile_error passes) + the stderr line the
        // code-check reads.
        let diag = "cdz: error [CDZ0201] (node 1): bad separator";

        // (a) Wire ABSENT → quality NOT graded; the case grades Pass on code alone (today's behavior).
        let res = grade_run(&tr, 1, diag, None, None, never).unwrap();
        assert_eq!(res.grade, Grade::Pass, "no wire → quality ungraded");

        // (b) Wire present, ONE error CDZ0201 fault carrying a `replace` fix whose replacement is "foo",
        // verified — matches the pinned fix + count 1 → Pass.
        use cadenza_compile_abi::{FixKind, Severity as S};
        let wire_ok = bin_wire(&[(
            S::Error,
            Some("CDZ0201"),
            Some(1),
            Some((FixKind::Replace, 1, "foo", true)),
            "bad separator",
        )]);
        let res = grade_run(&tr, 1, diag, Some(&wire_ok), None, never).unwrap();
        assert_eq!(res.grade, Grade::Pass, "matching fix+count → pass");

        // (c) Wire present but the fix's replacement is "bar" (≠ pinned "foo") → the quality grade FAILS.
        let wire_bad = bin_wire(&[(
            S::Error,
            Some("CDZ0201"),
            Some(1),
            Some((FixKind::Replace, 1, "bar", true)),
            "bad separator",
        )]);
        let res = grade_run(&tr, 1, diag, Some(&wire_bad), None, never).unwrap();
        assert!(
            matches!(res.grade, Grade::Fail(_)),
            "mismatched fix replacement → fail: {:?}",
            res.grade
        );

        // (d) Wire present but carries TWO CDZ0201 faults (count ≠ 1) → the count facet FAILS.
        let wire_two = bin_wire(&[
            (
                S::Error,
                Some("CDZ0201"),
                Some(1),
                Some((FixKind::Replace, 1, "foo", true)),
                "one",
            ),
            (
                S::Error,
                Some("CDZ0201"),
                Some(2),
                Some((FixKind::Replace, 3, "foo", true)),
                "two",
            ),
        ]);
        let res = grade_run(&tr, 1, diag, Some(&wire_two), None, never).unwrap();
        assert!(
            matches!(res.grade, Grade::Fail(_)),
            "count mismatch → fail: {:?}",
            res.grade
        );
    }

    #[test]
    fn c1_is_default_on_and_the_no_diagnostic_quality_marker_opts_out() {
        // The opt-in→default flip: grade_diagnostic_quality now runs on EVERY case (no marker), UNLESS
        // `(no-diagnostic-quality)` opts out. A §1-violating fault reds by default; opt-out suppresses it.
        use cadenza_compile_abi::Severity as S;
        let never =
            |_: &GTrial| -> Result<Outcome> { panic!("compile-outcome case runs no trial") };
        let mk = |opt_out: bool| TestRun {
            description: "c1-default".into(),
            trials: vec![GTrial {
                call: None,
                expect: GExpect::Error("CDZ0900".into(), vec![], vec![]),
                diag: None,
                exact_code: false,
            }],
            host_responses: vec![],
            host_calls: vec![],
            warns: vec![],
            live_objects: None,
            live_objects_known_leak: false,
            live_objects_per_call: None,
            no_other_errors: false,
            no_diagnostic: vec![],
            diagnostic_quality: false, // NO opt-in marker — default-on still grades it
            diagnostic_quality_opt_out: opt_out,
        };
        let diag = "cdz: error [CDZ0900] (node 1): this construct is not yet supported";
        // A §1-forbidden phrase ("not yet") in the emitted diagnostic's message.
        let wire = bin_wire(&[(
            S::Error,
            Some("CDZ0900"),
            Some(1),
            None,
            "this construct is not yet supported",
        )]);
        // DEFAULT-ON: no marker, not opted out → §1 flags the "not yet" → Fail.
        assert!(
            matches!(
                grade_run(&mk(false), 1, diag, Some(&wire), None, never)
                    .unwrap()
                    .grade,
                Grade::Fail(_)
            ),
            "default-on §1 must flag a forbidden phrase with no marker"
        );
        // OPT-OUT: `(no-diagnostic-quality)` suppresses the §1 lint → the forbidden phrase is not flagged.
        // (grade_compile_error still passes: CDZ0900 declined matches the (error CDZ0900) trial.)
        assert!(
            !matches!(
                grade_run(&mk(true), 1, diag, Some(&wire), None, never)
                    .unwrap()
                    .grade,
                Grade::Fail(_)
            ),
            "(no-diagnostic-quality) must suppress the §1 lint"
        );
    }

    #[test]
    fn no_other_errors_flags_an_unasserted_cascade_code() {
        let never =
            |_: &GTrial| -> Result<Outcome> { panic!("compile-outcome case runs no trial") };
        // A case asserting exactly ONE error (CDZ0201), optionally with the `(no-other-errors)` clause.
        let mk = |no_other: bool| TestRun {
            description: "no-other-errors".into(),
            trials: vec![GTrial {
                call: None,
                expect: GExpect::Error("CDZ0201".into(), vec![], vec![]),
                diag: None,
                exact_code: false,
            }],
            host_responses: vec![],
            host_calls: vec![],
            warns: vec![],
            live_objects: None,
            live_objects_known_leak: false,
            live_objects_per_call: None,
            no_other_errors: no_other,
            no_diagnostic: vec![],
            diagnostic_quality: false,
            diagnostic_quality_opt_out: false,
        };
        let diag = "cdz: error [CDZ0201] (node 1): bad thing";
        use cadenza_compile_abi::Severity as S;
        // (a) EXACTLY the asserted code emitted → Pass.
        let wire_one = bin_wire(&[(S::Error, Some("CDZ0201"), Some(1), None, "bad thing")]);
        assert_eq!(
            grade_run(&mk(true), 1, diag, Some(&wire_one), None, never)
                .unwrap()
                .grade,
            Grade::Pass
        );
        // (b) an EXTRA unasserted CDZ0999 error (a cascade) → `(no-other-errors)` FAILS.
        let wire_cascade = bin_wire(&[
            (S::Error, Some("CDZ0201"), Some(1), None, "bad thing"),
            (S::Error, Some("CDZ0999"), Some(2), None, "cascade"),
        ]);
        assert!(
            matches!(
                grade_run(&mk(true), 1, diag, Some(&wire_cascade), None, never)
                    .unwrap()
                    .grade,
                Grade::Fail(_)
            ),
            "an unasserted error code must fail no-other-errors"
        );
        // (c) WITHOUT the clause the same cascade is NOT flagged by this facet → Pass.
        assert_eq!(
            grade_run(&mk(false), 1, diag, Some(&wire_cascade), None, never)
                .unwrap()
                .grade,
            Grade::Pass,
            "no clause → cascade not graded"
        );
        // (d) an extra WARNING is ignored (errors only) → Pass even with the clause.
        let wire_warn = bin_wire(&[
            (S::Error, Some("CDZ0201"), Some(1), None, "bad thing"),
            (S::Warning, Some("CDZ0305"), Some(2), None, "dead arm"),
        ]);
        assert_eq!(
            grade_run(&mk(true), 1, diag, Some(&wire_warn), None, never)
                .unwrap()
                .grade,
            Grade::Pass,
            "a warning is orthogonal to no-other-errors"
        );
    }

    #[test]
    fn no_diagnostic_is_program_scoped_and_cross_kind() {
        let never =
            |_: &GTrial| -> Result<Outcome> { panic!("compile-outcome case runs no trial") };
        // A case that refuses with CDZ0201 (graded from the diag, no run) AND pins that the phrase "needs a
        // heap walk" must appear in NO diagnostic — the cross-kind program-scoped absence `(not …)` can't do.
        let mk = |phrases: Vec<String>| TestRun {
            description: "no-diagnostic".into(),
            trials: vec![GTrial {
                call: None,
                expect: GExpect::Error("CDZ0201".into(), vec![], vec![]),
                diag: None,
                exact_code: false,
            }],
            host_responses: vec![],
            host_calls: vec![],
            warns: vec![],
            live_objects: None,
            live_objects_known_leak: false,
            live_objects_per_call: None,
            no_other_errors: false,
            no_diagnostic: phrases,
            diagnostic_quality: false,
            diagnostic_quality_opt_out: false,
        };
        let pin = || mk(vec!["needs a heap walk".into()]);

        // (a) the pinned CDZ0201 error alone, phrase ABSENT anywhere → Pass.
        let diag_clean = "cdz: error [CDZ0201] (node 1): bad separator";
        assert_eq!(
            grade_run(&pin(), 1, diag_clean, None, None, never)
                .unwrap()
                .grade,
            Grade::Pass
        );
        // (b) the phrase leaked as a SEPARATE uncoded decline line (a sibling of the matched error — the exact
        // case `(not …)` misses because it's kind-scoped to the first-error message) → Fail.
        let diag_uncoded = "cdz: error [CDZ0201] (node 1): bad separator\ncdz: error: needs a heap walk (not yet built)";
        assert!(
            matches!(
                grade_run(&pin(), 1, diag_uncoded, None, None, never)
                    .unwrap()
                    .grade,
                Grade::Fail(_)
            ),
            "a sibling uncoded-decline carrying the forbidden phrase must fail (cross-kind, program-scoped)"
        );
        // (c) the phrase leaked in a WARNING line (another kind an error's `(not …)` never scans) → Fail.
        let diag_warn = "cdz: error [CDZ0201] (node 1): bad separator\ncdz: warning [CDZ0306] (node 2): needs a heap walk";
        assert!(
            matches!(
                grade_run(&pin(), 1, diag_warn, None, None, never)
                    .unwrap()
                    .grade,
                Grade::Fail(_)
            ),
            "a warning carrying the forbidden phrase must fail (cross-kind)"
        );
        // (d) WITHOUT the pin the same leaking diag is NOT flagged by this facet → Pass (additive: no clause,
        // no new failure — nothing regresses for cases that don't author it).
        assert_eq!(
            grade_run(&mk(vec![]), 1, diag_warn, None, None, never)
                .unwrap()
                .grade,
            Grade::Pass,
            "no clause → the phrase is not graded"
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
                Some(baseline),
                false
            )),
            failure
        );
        // WITH baseline: a baseline-TODO case that now FAILS reds (#3984 gate-hole) → FAILURE.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Fail("x".into())),
                "an incomplete case",
                Some(baseline),
                false
            )),
            failure
        );
        // WITH baseline: an ABSENT case that FAILS reds (#3984: non-pass, non-fail) → FAILURE.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Fail("x".into())),
                "absent",
                Some(baseline),
                false
            )),
            failure
        );
        // WITH baseline: a baseline-TODO case that stays TODO is NOT a regression → SUCCESS.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Todo("x".into())),
                "an incomplete case",
                Some(baseline),
                false
            )),
            success
        );
        // WITH baseline: a `fail` baseline + `fail` verdict is a TRACKED known-fail (#4547) → SUCCESS.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Fail("x".into())),
                "a known-fail case",
                Some(baseline),
                false
            )),
            success
        );
        // WITHOUT baseline: any outright Fail → FAILURE (the miscompile check).
        assert_eq!(
            fmt(exec_exit(&res(Grade::Fail("x".into())), "x", None, false)),
            failure
        );
        // A Pass always succeeds.
        assert_eq!(
            fmt(exec_exit(
                &res(Grade::Pass),
                "a passing case",
                Some(baseline),
                false
            )),
            success
        );
    }

    /// MEMBERSHIP-ONLY (the rust curated-subset gate): an ABSENT case that FAILs is NOT enforced — exempt
    /// from the #3984 red (grade IFF title ∈ baseline; rust is incremental). It does NOT weaken the enforced
    /// set: a baselined `todo` that now fails STILL reds (covered), and a baselined `pass` regression STILL
    /// reds. Contrast the strict (wasm) mode (`false`) where an absent-fail reds.
    #[test]
    fn exec_exit_membership_only_exempts_absent_case_fail() {
        let fmt = |c: ExitCode| format!("{c:?}");
        let success = fmt(ExitCode::SUCCESS);
        let failure = fmt(ExitCode::FAILURE);
        let baseline = "pass\ta passing case\ntodo\tan incomplete case\n";
        let res = |g: Grade| GradeResult {
            grade: g,
            ran_a_trial: true,
        };
        let fail = || res(Grade::Fail("x".into()));
        // ABSENT + FAIL: strict (wasm) reds (#3984); membership-only (rust) EXEMPTS it.
        assert_eq!(
            fmt(exec_exit(&fail(), "absent case", Some(baseline), false)),
            failure
        );
        assert_eq!(
            fmt(exec_exit(&fail(), "absent case", Some(baseline), true)),
            success
        );
        // membership-only does NOT weaken the enforced set:
        //  - a baselined-TODO now-FAIL still reds (it IS covered) ...
        assert_eq!(
            fmt(exec_exit(
                &fail(),
                "an incomplete case",
                Some(baseline),
                true
            )),
            failure
        );
        //  - and a baselined-PASS regression still reds.
        assert_eq!(
            fmt(exec_exit(&fail(), "a passing case", Some(baseline), true)),
            failure
        );
    }

    /// `parse_diagnostics` decodes rcdzc's 8-column structured-diagnostics wire into typed faults — the
    /// foundation for corpus `(error …)`/`(warning …)` diagnostic-QUALITY assertions (fix/verified/count).
    #[test]
    fn parse_diagnostics_decodes_the_binary_wire() {
        use cadenza_compile_abi::{FixKind, Severity as S};
        // A coded ERROR with a verified REPLACE fix; a WARNING with no fix + no code; a heuristic WRAP.
        let wire = bin_wire(&[
            (
                S::Error,
                Some("CDZ0203"),
                Some(7),
                Some((FixKind::Replace, 7, "foo", true)),
                "undefined name `fooo`; did you mean `foo`?",
            ),
            (S::Warning, None, None, None, "an unanchored note"),
            (
                S::Error,
                Some("CDZ0210"),
                Some(3),
                Some((FixKind::Wrap, 3, "(Some …)", false)),
                "match is non-exhaustive",
            ),
        ]);
        let faults = parse_diagnostics(&wire);
        assert_eq!(faults.len(), 3);

        assert_eq!(faults[0].severity, Severity::Error);
        assert_eq!(faults[0].code.as_deref(), Some("CDZ0203"));
        assert_eq!(faults[0].node, Some(7));
        let fix0 = faults[0].fix.as_ref().expect("has fix");
        assert_eq!(fix0.kind, "replace");
        assert_eq!(fix0.replacement, "foo");
        assert!(fix0.verified);
        assert!(faults[0].message.contains("did you mean"));

        // A `-` code/node/fix decodes to None/no-fix; severity still parses.
        assert_eq!(faults[1].severity, Severity::Warning);
        assert_eq!(faults[1].code, None);
        assert_eq!(faults[1].node, None);
        assert_eq!(faults[1].fix, None);

        // A heuristic fix decodes verified=false; wrap payload (with a tab-free `…`) round-trips.
        let fix2 = faults[2].fix.as_ref().expect("has fix");
        assert_eq!(fix2.kind, "wrap");
        assert_eq!(fix2.replacement, "(Some …)");
        assert!(!fix2.verified);
    }

    /// Non-binary-AST bytes decode to NO faults (the codec is tolerant), never a panic — the grader treats
    /// a bad/absent wire as "no faults" rather than failing the whole grade.
    #[test]
    fn parse_diagnostics_ignores_non_binary_bytes() {
        assert!(parse_diagnostics(b"not a binary-ast tree").is_empty());
        assert!(parse_diagnostics(&[]).is_empty());
    }

    /// `count_faults` / `DiagFault::is` select by (severity, code) — the basis for a `(count N)` assertion
    /// that a presence-only check cannot express (e.g. "exactly one CDZ0305 dead-trap warning").
    #[test]
    fn count_faults_selects_by_severity_and_code() {
        use cadenza_compile_abi::Severity as S;
        let wire = bin_wire(&[
            (S::Warning, Some("CDZ0305"), Some(1), None, "dead trap A"),
            (S::Warning, Some("CDZ0305"), Some(2), None, "dead trap B"),
            (
                S::Error,
                Some("CDZ0305"),
                Some(3),
                None,
                "same code, different severity",
            ),
            (S::Warning, Some("CDZ0306"), Some(4), None, "unused binding"),
        ]);
        let faults = parse_diagnostics(&wire);
        assert_eq!(count_faults(&faults, Severity::Warning, "CDZ0305"), 2);
        assert_eq!(count_faults(&faults, Severity::Error, "CDZ0305"), 1);
        assert_eq!(count_faults(&faults, Severity::Warning, "CDZ0306"), 1);
        assert_eq!(count_faults(&faults, Severity::Warning, "CDZ9999"), 0);
        assert!(faults[0].is(Severity::Warning, "CDZ0305"));
        assert!(!faults[0].is(Severity::Error, "CDZ0305"));
    }

    /// `grade_check_parity`: parity HOLDS (`None`) when `cdz check` surfaces every coded ERROR `cdz compile`
    /// does — equal sets, and a check that OVER-reports (an extra coded error) is still fine (superset).
    #[test]
    fn check_parity_holds_when_check_covers_every_coded_compile_error() {
        use cadenza_compile_abi::Severity as S;
        let compile = bin_wire(&[
            (
                S::Error,
                Some("CDZ0203"),
                Some(4),
                None,
                "compound out of order",
            ),
            (
                S::Error,
                Some("CDZ0101"),
                Some(7),
                None,
                "not a type variable",
            ),
        ]);
        // Exact match → parity holds.
        assert_eq!(grade_check_parity(&compile, &compile), None);
        // Check reports a SUPERSET (adds CDZ0201) → still parity (check never MISSED a compile fault).
        let check_more = bin_wire(&[
            (
                S::Error,
                Some("CDZ0203"),
                Some(4),
                None,
                "compound out of order",
            ),
            (
                S::Error,
                Some("CDZ0101"),
                Some(7),
                None,
                "not a type variable",
            ),
            (
                S::Error,
                Some("CDZ0201"),
                Some(9),
                None,
                "extra check-only fault",
            ),
        ]);
        assert_eq!(grade_check_parity(&compile, &check_more), None);
    }

    /// `grade_check_parity`: the #7143 VIOLATION — a coded ERROR `cdz compile` rejects is SILENT under `cdz
    /// check` → `Some(msg)` naming the missing code (the caller downgrades to `Fail`).
    #[test]
    fn check_parity_flags_a_coded_fault_silent_under_check() {
        use cadenza_compile_abi::Severity as S;
        let compile = bin_wire(&[(
            S::Error,
            Some("CDZ0203"),
            Some(4),
            None,
            "coded fault in a parameterized export",
        )]);
        // check emits NOTHING for the same case → the CDZ0203 vanished from check.
        let check_empty: &[u8] = &[];
        let msg =
            grade_check_parity(&compile, check_empty).expect("a missing coded fault must red");
        assert!(
            msg.contains("CDZ0203"),
            "message must name the missing code, got {msg:?}"
        );
    }

    /// `grade_check_parity` SCOPE: a check that under-reports only CODELESS not-yets or coded WARNINGS is
    /// NOT a violation — both are out of scope (a check legitimately does less than a full compile).
    #[test]
    fn check_parity_ignores_codeless_declines_and_coded_warnings() {
        use cadenza_compile_abi::Severity as S;
        let compile = bin_wire(&[
            (
                S::Error,
                Some("CDZ0203"),
                Some(4),
                None,
                "the one coded rejection",
            ),
            (S::Error, None, Some(5), None, "an UNCODED not-yet decline"),
            (
                S::Warning,
                Some("CDZ0305"),
                Some(6),
                None,
                "a dead-trap WARNING",
            ),
        ]);
        // check surfaces ONLY the coded error, dropping the uncoded decline + the coded warning.
        let check = bin_wire(&[(
            S::Error,
            Some("CDZ0203"),
            Some(4),
            None,
            "the one coded rejection",
        )]);
        assert_eq!(
            grade_check_parity(&compile, &check),
            None,
            "under-reporting a codeless decline or a coded warning is in-scope-allowed, not a parity miss"
        );
    }

    /// `grade_diag_quality` checks the corpus fix/count assertions against parsed structured faults — the
    /// grading brain for the operator-greenlit "corpus expresses fixes" capability.
    #[test]
    fn grade_diag_quality_checks_fix_count_and_no_fix() {
        use cadenza_compile_abi::{FixKind, Severity as S};
        let wire = bin_wire(&[
            (
                S::Error,
                Some("CDZ0203"),
                Some(7),
                Some((FixKind::Replace, 7, "foo", true)),
                "undefined name; did you mean `foo`?",
            ),
            (S::Warning, Some("CDZ0305"), Some(1), None, "dead trap A"),
            (S::Warning, Some("CDZ0305"), Some(2), None, "dead trap B"),
        ]);
        let faults = parse_diagnostics(&wire);
        let pass = |g: &Grade| matches!(g, Grade::Pass);

        // Empty assertion = no-op Pass.
        assert!(pass(&grade_diag_quality(
            &faults,
            Severity::Error,
            "CDZ0203",
            &DiagExpect::default()
        )));

        // KIND + exact REPLACEMENT + VERIFIED all match → Pass.
        let want = DiagExpect {
            fix: Some(FixExpect {
                kind: Some("replace".into()),
                replacement: Some(ReplMatch::Exact("foo".into())),
                verified: Some(true),
            }),
            ..Default::default()
        };
        assert!(pass(&grade_diag_quality(
            &faults,
            Severity::Error,
            "CDZ0203",
            &want
        )));

        // Wrong kind → Fail; wrong replacement → Fail; wrong verified → Fail.
        let bad_kind = DiagExpect {
            fix: Some(FixExpect {
                kind: Some("wrap".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(matches!(
            grade_diag_quality(&faults, Severity::Error, "CDZ0203", &bad_kind),
            Grade::Fail(_)
        ));
        let bad_repl = DiagExpect {
            fix: Some(FixExpect {
                replacement: Some(ReplMatch::Exact("bar".into())),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(matches!(
            grade_diag_quality(&faults, Severity::Error, "CDZ0203", &bad_repl),
            Grade::Fail(_)
        ));

        // Substring replacement matches the exact spelling too.
        let contains = DiagExpect {
            fix: Some(FixExpect {
                replacement: Some(ReplMatch::Contains("fo".into())),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(pass(&grade_diag_quality(
            &faults,
            Severity::Error,
            "CDZ0203",
            &contains
        )));

        // no_fix on a fault that HAS a fix → Fail; no_fix on a fault with none → Pass.
        let no_fix = DiagExpect {
            no_fix: true,
            ..Default::default()
        };
        assert!(matches!(
            grade_diag_quality(&faults, Severity::Error, "CDZ0203", &no_fix),
            Grade::Fail(_)
        ));
        assert!(pass(&grade_diag_quality(
            &faults,
            Severity::Warning,
            "CDZ0305",
            &no_fix
        )));

        // COUNT: exactly two CDZ0305 warnings → Pass with (count 2); (count 1) → Fail.
        let count2 = DiagExpect {
            count: Some(2),
            ..Default::default()
        };
        assert!(pass(&grade_diag_quality(
            &faults,
            Severity::Warning,
            "CDZ0305",
            &count2
        )));
        let count1 = DiagExpect {
            count: Some(1),
            ..Default::default()
        };
        assert!(matches!(
            grade_diag_quality(&faults, Severity::Warning, "CDZ0305", &count1),
            Grade::Fail(_)
        ));

        // A fix assertion on an ABSENT code → Fail (found none); (count 0) on an absent code → Pass.
        assert!(matches!(
            grade_diag_quality(&faults, Severity::Error, "CDZ9999", &want),
            Grade::Fail(_)
        ));
        let count0 = DiagExpect {
            count: Some(0),
            ..Default::default()
        };
        assert!(pass(&grade_diag_quality(
            &faults,
            Severity::Error,
            "CDZ9999",
            &count0
        )));
    }

    /// seq-15 `known_leak_now_clean`: a known-leak case is a TIGHTEN CANDIDATE iff a heap trial ran and EVERY
    /// heap trial measured 0 (its reclaim fix landed). Any residual leak, or an all-no-heap case, is not.
    #[test]
    fn known_leak_now_clean_flags_only_fully_clean() {
        assert!(known_leak_now_clean(&[Some(0)])); // single heap trial, now clean
        assert!(known_leak_now_clean(&[Some(0), None, Some(0)])); // all heap trials clean, no-heap skipped
        assert!(!known_leak_now_clean(&[Some(0), Some(1)])); // one trial still leaks → not yet
        assert!(!known_leak_now_clean(&[Some(2)])); // still leaks
        assert!(!known_leak_now_clean(&[None, None])); // no heap trial measured → not a candidate
        assert!(!known_leak_now_clean(&[])); // no trials
    }

    #[test]
    fn parse_leak_ledger_reads_n_tab_description_skips_header_and_blanks() {
        let text = "# gate baseline leaks — true-leak counts (top-level result dropped)\n\
                    3\ta case that leaks three husks\n\
                    \n\
                    1\tanother leaking case\n\
                    # a comment mid-file\n\
                    12\ta big leaker\n";
        let m = parse_leak_ledger(text);
        assert_eq!(m.len(), 3);
        assert_eq!(m.get("a case that leaks three husks"), Some(&3));
        assert_eq!(m.get("another leaking case"), Some(&1));
        assert_eq!(m.get("a big leaker"), Some(&12));
        // a malformed line (non-numeric leak) is skipped, not panicked
        assert!(parse_leak_ledger("notanumber\tx").is_empty());
    }

    #[test]
    fn check_leak_ledger_reds_on_any_change_absent_means_zero() {
        let mut ledger = BTreeMap::new();
        ledger.insert("leaks 3".to_string(), 3u32);
        // tracked==observed → no mismatch (both a leaker at its count, and an absent case at 0)
        assert!(check_leak_ledger("leaks 3", 3, &ledger).is_none());
        assert!(check_leak_ledger("clean absent case", 0, &ledger).is_none());
        // GREW (regression) → mismatch
        assert!(check_leak_ledger("leaks 3", 5, &ledger).is_some());
        // SHRANK (progress) → ALSO mismatch (red-on-any-change, operator ruling)
        assert!(check_leak_ledger("leaks 3", 1, &ledger).is_some());
        assert!(check_leak_ledger("leaks 3", 0, &ledger).is_some());
        // an ABSENT case that now leaks → mismatch (unexpected leak vs the implicit-0)
        assert!(check_leak_ledger("clean absent case", 2, &ledger).is_some());
    }

    /// The #7527 scalar-return classifier + the `check_live_objects_scalar` skip: a later heap-RETURN trial
    /// is exempt from the must-reclaim-to-0 check (its reachable-return count is not a leak), while a
    /// scalar-return later trial + trial 0 stay strictly checked.
    #[test]
    fn scalar_return_discriminator_skips_later_heap_return_trials() {
        // Classifier: bare scalars (incl. ascription form) = true; heap/compound renders = false; non-Output = false.
        assert!(expect_is_scalar_return(&GExpect::Output("42".into())));
        assert!(expect_is_scalar_return(&GExpect::Output(
            "(: 42 Int64)".into()
        )));
        assert!(expect_is_scalar_return(&GExpect::Output("-3".into())));
        assert!(expect_is_scalar_return(&GExpect::Output("true".into())));
        assert!(expect_is_scalar_return(&GExpect::Output("'a'".into())));
        assert!(!expect_is_scalar_return(&GExpect::Output(
            "#list(5)".into()
        )));
        assert!(!expect_is_scalar_return(&GExpect::Output("(1 2)".into())));
        assert!(!expect_is_scalar_return(&GExpect::Output("\"hi\"".into())));
        assert!(!expect_is_scalar_return(&GExpect::Trap("boom".into())));

        // The 3-false-pos shape: trial 0 returns empty (0 cells), a LATER trial returns a heap value (2
        // reachable cells) — NOT a leak. With the discriminator (later trial = heap-return), it PASSES;
        // strict (no discriminator) FALSE-FAILS it.
        assert_eq!(
            check_live_objects_scalar(&[Some(0), Some(2)], Some(0), None, &[true, false]),
            None
        );
        assert!(check_live_objects(&[Some(0), Some(2)], Some(0), None).is_some()); // strict still fails

        // A genuine scalar-return later-trial leak (both trials scalar) is STILL caught.
        assert!(
            check_live_objects_scalar(&[Some(0), Some(5)], Some(0), None, &[true, true])
                .as_deref()
                .unwrap()
                .contains("call 1")
        );
        // Trial 0 is the always-checked calibration: a heap-return on trial 0 is NOT skipped.
        assert!(check_live_objects_scalar(&[Some(2)], Some(0), None, &[false]).is_some());
        // Empty per_trial_scalar = strict (no skip).
        assert!(check_live_objects_scalar(&[Some(0), Some(2)], Some(0), None, &[]).is_some());
        // A positional `(live-objects N1 N2)` case is unaffected by the discriminator (author-specified).
        assert!(
            check_live_objects_scalar(&[Some(0), Some(2)], Some(0), Some(&[0, 0]), &[true, false])
                .is_some()
        );
    }

    /// `check_live_objects` balances EVERY trial, not just call[0] — the fix for the systemic false-green
    /// where a multi-call case that balanced on the first call hid a leak on later calls.
    #[test]
    fn check_live_objects_balances_every_call() {
        // UNIFORM form (`per_call = None`). No-heap trials are skipped, regardless of `expected`.
        assert_eq!(check_live_objects(&[None, None], Some(0), None), None);
        assert_eq!(check_live_objects(&[], None, None), None);
        // A single heap trial at the expected count passes; the message on a miss is index-free (stable text).
        assert_eq!(check_live_objects(&[Some(0)], Some(0), None), None);
        assert_eq!(
            check_live_objects(&[Some(1)], Some(0), None).as_deref(),
            Some("live-objects mismatch: expected 0, got 1")
        );
        // Absent clause ⇒ opt-out default of 0.
        assert_eq!(
            check_live_objects(&[Some(2)], None, None).as_deref(),
            Some("live-objects mismatch: expected 0, got 2")
        );
        // THE #5008 FIX: a multi-call case that balances on call 0 but LEAKS on call 2 is now caught — the
        // historical first-call-only capture returned None here (false green).
        assert_eq!(
            check_live_objects(&[Some(0), Some(0), Some(0)], Some(0), None),
            None
        );
        assert_eq!(
            check_live_objects(&[Some(0), Some(0), Some(3)], Some(0), None).as_deref(),
            Some("live-objects mismatch on call 2: expected 0, got 3")
        );
        // A depth-scaling leak on the FIRST call still reports call 0 (multi-call message form).
        assert_eq!(
            check_live_objects(&[Some(1), Some(2)], Some(0), None).as_deref(),
            Some("live-objects mismatch on call 0: expected 0, got 1")
        );
        // A uniform expected N > 0 (an explicit `(live-objects N)` / known-leak N) holds across every call.
        assert_eq!(check_live_objects(&[Some(2), Some(2)], Some(2), None), None);
        // Interleaved no-heap trials don't shift the reported call index (index is the trial position).
        assert_eq!(
            check_live_objects(&[None, Some(0), Some(5)], Some(0), None).as_deref(),
            Some("live-objects mismatch on call 2: expected 0, got 5")
        );
    }

    /// PER-CALL positional counts (`(live-objects N1 N2 N3)`) — the arm-dependent balance a uniform count
    /// cannot express (FLETCHER-16: a leak that scales with input size). Each call is checked against its
    /// OWN count; a no-heap trial's slot is ignored; a length mismatch is a clear authoring Fail.
    #[test]
    fn check_live_objects_per_call_positional() {
        // FLETCHER-16 shape: three calls balancing 3 / 13 / 0 — the uniform form would fail calls 1+2.
        let fletcher = &[Some(3), Some(13), Some(0)];
        assert_eq!(
            check_live_objects(fletcher, Some(3), Some(&[3, 13, 0])),
            None
        );
        // Uniform-3 (per_call None) WOULD fail this — the phantom fail #5008 surfaced.
        assert!(check_live_objects(fletcher, Some(3), None).is_some());
        // A wrong per-call count is caught at the offending call.
        assert_eq!(
            check_live_objects(fletcher, Some(3), Some(&[3, 12, 0])).as_deref(),
            Some("live-objects mismatch on call 1: expected 12, got 13")
        );
        // A no-heap (None) trial's positional slot is ignored (skipped), not failed.
        assert_eq!(
            check_live_objects(&[Some(3), None, Some(0)], Some(3), Some(&[3, 99, 0])),
            None
        );
        // Length mismatch (list ≠ trial count) is an authoring Fail, not a silent under-check.
        assert!(check_live_objects(fletcher, Some(3), Some(&[3, 13])).is_some());
    }

    #[test]
    fn leak_ceiling_clamp_passes_fewer_and_fails_over() {
        // The 0023 corpus-cadenza tolerance: a KNOWN-LEAK ceiling N; the cadenza-hop reclaiming FEWER
        // (count <= N) is strictly safer → clamp-then-check PASSES; an over-count (> N) still FAILS.
        // uniform ceiling 69: hop-66 (3 fewer, the real 0023 profile) passes; direct-69 passes; over-72 fails.
        assert_eq!(leak_ceiling_clamp(&[Some(66)], 69, None), vec![Some(69)]); // clamped up to the ceiling
        assert_eq!(
            check_live_objects(&leak_ceiling_clamp(&[Some(66)], 69, None), Some(69), None),
            None
        );
        assert_eq!(
            check_live_objects(&leak_ceiling_clamp(&[Some(69)], 69, None), Some(69), None),
            None
        );
        assert!(
            check_live_objects(&leak_ceiling_clamp(&[Some(72)], 69, None), Some(69), None)
                .is_some()
        );
        // positional ceilings: each trial clamps to its own list[i]; a no-heap None stays None.
        let clamped = leak_ceiling_clamp(&[Some(30), None, Some(42)], 0, Some(&[33, 0, 45]));
        assert_eq!(clamped, vec![Some(33), None, Some(45)]);
        assert_eq!(
            check_live_objects(&clamped, Some(33), Some(&[33, 0, 45])),
            None
        );
    }

    /// seq-29: `decode_test_run` partitions an `(expect-error CODE "msg"* (not "phrase")*)` form into
    /// the code, the positive message substrings (bare string leaves), and the `(not …)` absence substrings.
    #[test]
    fn decode_reads_not_message_absence_pins() {
        use cadenza_syntax::ast::{Builder, Leaf};
        use std::sync::Arc;
        let mut b = Builder::new();
        let s = |b: &mut Builder, t: &str| b.atom_leaf(Leaf::Str(Arc::from(t)));
        let head = b.name("test-run");
        let dh = b.name("description");
        let dv = s(&mut b, "case");
        let desc = b.list(vec![dh, dv]);
        // (expect-error "CDZ0201" "malformed" (not "internal error") (not "panic"))
        let eh = b.name("expect-error");
        let code = s(&mut b, "CDZ0201");
        let pos = s(&mut b, "malformed");
        let nh1 = b.name("not");
        let nv1 = s(&mut b, "internal error");
        let neg1 = b.list(vec![nh1, nv1]);
        let nh2 = b.name("not");
        let nv2 = s(&mut b, "panic");
        let neg2 = b.list(vec![nh2, nv2]);
        let expect = b.list(vec![eh, code, pos, neg1, neg2]);
        let th = b.name("trial");
        let trial = b.list(vec![th, expect]);
        let trials_head = b.name("trials");
        let trials = b.list(vec![trials_head, trial]);
        let root = b.list(vec![head, desc, trials]);
        let bytes = codec::encode(&b.finish(root));
        let tr = decode_test_run(&bytes).unwrap();
        match &tr.trials[0].expect {
            GExpect::Error(code, msgs, not_msgs) => {
                assert_eq!(code, "CDZ0201");
                assert_eq!(msgs.as_slice(), ["malformed"]);
                assert_eq!(not_msgs.as_slice(), ["internal error", "panic"]);
            }
            _ => panic!("expected GExpect::Error"),
        }
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
        // Plain (uniform) count — one count, no per-call list.
        let tr = decode_test_run(&build(Some(&["0"]))).unwrap();
        assert_eq!(tr.live_objects, Some(0));
        assert!(!tr.live_objects_known_leak);
        assert_eq!(tr.live_objects_per_call, None);
        // seq-15 PURE-BINARY: a bare `known-leak` marker (no count) → flag set, count None.
        let tr = decode_test_run(&build(Some(&["known-leak"]))).unwrap();
        assert_eq!(tr.live_objects, None);
        assert!(tr.live_objects_known_leak);
        assert_eq!(tr.live_objects_per_call, None);
        // A legacy count-bearing marker still decodes (flag + count) — grading ignores the count.
        let tr = decode_test_run(&build(Some(&["known-leak", "3"]))).unwrap();
        assert_eq!(tr.live_objects, Some(3));
        assert!(tr.live_objects_known_leak);
        assert_eq!(tr.live_objects_per_call, None);
        // PER-CALL positional (2+ counts) → live_objects = first, live_objects_per_call = whole list.
        let tr = decode_test_run(&build(Some(&["3", "13", "0"]))).unwrap();
        assert_eq!(tr.live_objects, Some(3));
        assert_eq!(tr.live_objects_per_call, Some(vec![3, 13, 0]));
        assert!(!tr.live_objects_known_leak);
        // known-leak + per-call positional.
        let tr = decode_test_run(&build(Some(&["known-leak", "3", "13", "0"]))).unwrap();
        assert_eq!(tr.live_objects, Some(3));
        assert_eq!(tr.live_objects_per_call, Some(vec![3, 13, 0]));
        assert!(tr.live_objects_known_leak);
        // No clause.
        let tr = decode_test_run(&build(None)).unwrap();
        assert_eq!(tr.live_objects, None);
        assert!(!tr.live_objects_known_leak);
        assert_eq!(tr.live_objects_per_call, None);
    }

    /// `decode_trial` reads the diagnostic-QUALITY clauses `(fix …)` / `(no-fix)` / `(count N)` / `(once)`
    /// into `GTrial.diag` (a `DiagExpect`), and leaves it `None` when the trial pins only code + message.
    /// The decoded facets are exactly what `grade_diag_quality` grades against — this is the parse end of
    /// C1's "corpus expresses fixes" capability, the counterpart to the shred/authoring render in cdz-corpus.
    #[test]
    fn decode_reads_diag_quality_clauses() {
        use cadenza_syntax::ast::{Builder, Leaf};
        use std::sync::Arc;
        // Build a `(test-run … (trials (trial (expect-error CDZ0101) <extra…>)))` and return the decoded
        // trial's `diag`. `mk_extra` builds the trial's diagnostic-quality clause forms in the owned builder.
        let decode_diag = |mk_extra: &dyn Fn(
            &mut Builder,
        ) -> Vec<cadenza_syntax::ast::StructId>|
         -> Option<DiagExpect> {
            let mut b = Builder::new();
            let s = |b: &mut Builder, t: &str| b.atom_leaf(Leaf::Str(Arc::from(t)));
            let extra = mk_extra(&mut b);
            let head = b.name("test-run");
            let dh = b.name("description");
            let dv = s(&mut b, "case");
            let desc = b.list(vec![dh, dv]);
            let th = b.name("trial");
            let eh = b.name("expect-error");
            let ec = s(&mut b, "CDZ0101");
            let expect = b.list(vec![eh, ec]);
            let mut trial_kids = vec![th, expect];
            trial_kids.extend(extra);
            let trial = b.list(trial_kids);
            let trials_head = b.name("trials");
            let trials = b.list(vec![trials_head, trial]);
            let root = b.list(vec![head, desc, trials]);
            let bytes = codec::encode(&b.finish(root));
            decode_test_run(&bytes)
                .unwrap()
                .trials
                .into_iter()
                .next()
                .unwrap()
                .diag
        };

        // A full `(fix (kind replace) (replacement "foo") (verified))` + `(count 1)`.
        let diag = decode_diag(&|b| {
            let s = |b: &mut Builder, t: &str| b.atom_leaf(Leaf::Str(Arc::from(t)));
            let fh = b.name("fix");
            let kh = b.name("kind");
            let kv = s(b, "replace");
            let kind = b.list(vec![kh, kv]);
            let rh = b.name("replacement");
            let rv = s(b, "foo");
            let repl = b.list(vec![rh, rv]);
            let vh = b.name("verified");
            let ver = b.list(vec![vh]);
            let fix = b.list(vec![fh, kind, repl, ver]);
            let ch = b.name("count");
            let cv = s(b, "1");
            let count = b.list(vec![ch, cv]);
            vec![fix, count]
        })
        .expect("diag present");
        assert_eq!(diag.count, Some(1));
        assert!(!diag.no_fix);
        let fx = diag.fix.expect("fix present");
        assert_eq!(fx.kind.as_deref(), Some("replace"));
        assert_eq!(fx.replacement, Some(ReplMatch::Exact("foo".into())));
        assert_eq!(fx.verified, Some(true));

        // `(no-fix)` + `(once)` (== count 1).
        let diag = decode_diag(&|b| {
            let nfh = b.name("no-fix");
            let no_fix = b.list(vec![nfh]);
            let oh = b.name("once");
            let once = b.list(vec![oh]);
            vec![no_fix, once]
        })
        .expect("diag present");
        assert!(diag.no_fix);
        assert_eq!(diag.count, Some(1));
        assert!(diag.fix.is_none());

        // A bare `(fix (replacement-contains "bar") (unverified))`.
        let diag = decode_diag(&|b| {
            let fh = b.name("fix");
            let rch = b.name("replacement-contains");
            let rcv = b.atom_leaf(Leaf::Str(Arc::from("bar")));
            let rcontains = b.list(vec![rch, rcv]);
            let uvh = b.name("unverified");
            let unver = b.list(vec![uvh]);
            let fix = b.list(vec![fh, rcontains, unver]);
            vec![fix]
        })
        .expect("diag present");
        let fx = diag.fix.expect("fix present");
        assert_eq!(fx.replacement, Some(ReplMatch::Contains("bar".into())));
        assert_eq!(fx.verified, Some(false));
        assert_eq!(fx.kind, None);
        assert_eq!(diag.count, None);

        // No quality clause at all → diag is None (the common code+message-only form).
        assert_eq!(decode_diag(&|_| vec![]), None);
    }
}
