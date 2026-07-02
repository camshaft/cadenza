# Command — gate

**Purpose.** Run the conformance gate and decide promotability. Cadenza judges a
generation by **two** gates, and this command runs both: the **requirement gate**
(extract every requirement from the specification tree and check that each
load-bearing one is discharged by an implementation citation and a test citation),
and the **behavior gate** (execute every case in the executable-semantics corpus
through the reference interpreter and confirm each reproduces its recorded output).
Both must pass. This is the bar a regeneration must clear before it is promoted.

**Agent-agnostic.** Neutral prompt body; assumes a shell and the `duvet` CLI.
The meaning of the gates is fixed normatively in
`spec/capabilities/conformance-gate.md` and `spec/capabilities/compiler-pipeline.md`;
this command is the procedure that runs them.

## Inputs

- `.duvet/config.toml` for a full regeneration, or `.duvet/bootstrap.toml` for
  the ignition subset (pass `--bootstrap` to select it).
- The generated compiler under `[[source]]` (in `implementation/`) with its
  requirement citations.
- The executable-semantics corpus: `spec/semantics/*.sexp`.
- The reference interpreter the generation realizes (the behavioral oracle,
  `spec/capabilities/self-hosting-and-bootstrap.md`).
- The pinned non-load-bearing set at `options/gate-non-load-bearing/`, subtracted
  from the load-bearing total.

## The requirement gate

1. Ensure the `[[source]]` blocks match the target language of the code under
   review (run `setup-gate` first if the language changed).
2. Run the report:
   - Full: `duvet report`
   - Bootstrap: `duvet report --config-path .duvet/bootstrap.toml`
3. Parse the emitted JSON report (`.duvet/reports/report.json`).
4. Compute coverage:
   - A requirement at MUST / SHALL level is **covered** only if it has both an
     implementation citation and a test citation (conformance-gate.md §"Coverage
     Requires Implementation And Test").
   - A requirement at SHOULD / MAY level is advisory (conformance-gate.md §"The Gate
     Is The Promotion Bar").
   - Subtract the change-process and human-governance requirements pinned in
     `options/gate-non-load-bearing/` from the load-bearing total, so a generation
     is not failed for lacking a runtime implementation of a rule about how the spec
     may change (conformance-gate.md §"A Change-Process Requirement Is Not
     Load-Bearing For A Generation").
   - Hold the requirements of any optional capability this build excluded
     non-load-bearing, and record which optional capabilities were included so the
     load-bearing set is reproducible; a build not told whether to include one
     counts it as included (conformance-gate.md §"An Excluded Optional Capability Is
     Not Load-Bearing", §"The Optional-Capability Set Is Recorded And Defaults To
     Included").
5. Detect broken citations: any citation whose quoted text no longer matches a
   requirement is a hard failure — duvet reports it with the exact source location
   (conformance-gate.md §"Requirements Are Written To Be Extractable": the gate
   treats a citation whose quoted text matches no requirement as a failure that names
   the offending location).
6. Judge coverage honesty, not just count: a citation discharges its own requirement
   only if it annotates the code that performs the behavior and its test fails when
   that behavior is removed (conformance-gate.md §"A Citation Discharges Its Own
   Requirement"). Flag two distinct requirements discharged by one shared check that
   cannot fail for one without failing for the other, and any behavioral requirement
   whose cited test still passes when its behavior is broken (§"A Behavior
   Requirement Is Covered Only By Execution", §"A Cited Behavioral Test Is Sensitive
   To Its Requirement").

## The behavior gate

1. Execute every case in `spec/semantics/*.sexp` through the reference interpreter
   (conformance-gate.md §"Every Case Executes To Its Recorded Output",
   compiler-pipeline.md §"The Corpus Is A Gate").
2. For each case, compare the run's terminal condition and result against the
   recorded expectation — the `output` value form under the canonical value form, or
   the expected `error` code, or the `trap` (per `spec/semantics/README.md`).
3. Fail the generation if any case does not reproduce its recorded output.
4. Confirm the corpus is complete: every behavioral requirement of an included
   capability is witnessed by at least one case (conformance-gate.md §"Every
   Behavioral Requirement Is Witnessed By A Case"). A generation whose corpus omits
   a witnessing case for a load-bearing behavioral requirement MUST NOT be promoted.
5. Judge the compiled derivation against the oracle where applicable: a compiled
   generation's observable behavior MUST agree with the reference interpreter over
   every case before it is promoted (`spec/bootstrap.md` §"Compiled Derivation Is An
   Oracle-Checked Optimization", constitution §XIV).

## Judging against an immutable specification

- Judge the generation against a content-addressed snapshot of the specification
  tree, and record the snapshot judged against, so the verdict can be reproduced
  against the same specification (conformance-gate.md §"A Gate Run Judges Against A
  Content-Addressed Snapshot").

## Verdict

- `GATE: PASS` — every MUST/SHALL requirement is covered, there are zero broken
  citations, AND every executable-semantics case reproduces its recorded output.
- `GATE: FAIL` — otherwise. List each uncovered load-bearing requirement (file,
  section, sentence), each broken citation (source location + the stale quote), and
  each behavior case that did not reproduce its recorded output (the case
  description, file, and the observed-versus-recorded result).

A generation MUST pass **both** the requirement gate and the behavior gate before it
is promoted; a generation that passes one while failing the other MUST NOT be
promoted (compiler-pipeline.md §"Both Gates Must Pass", conformance-gate.md §"The
Gate Is The Promotion Bar", constitution §XII).

## Notes

- The requirement gate is language-agnostic: it reads coverage from the report,
  never from a language toolchain. Only the `[[source]]` half of the config is
  language-specific, and `setup-gate` owns that half.
- The behavior gate is language-agnostic in a different sense: it reads meaning from
  the corpus and the reference interpreter, never from the compiler's own encoding of
  behavior, so the compiler and every tool agree with the corpus rather than with
  themselves (constitution §IX).
- Pinned to stable duvet report types (`json`, `snapshot`). The report JSON is the
  machine-readable input the requirement gate reads. The snapshot is a local
  coverage-regression check regenerated per build; it is gitignored, not a committed
  baseline (its coverage tags derive from citations in the gitignored
  `implementation/` code, so it is reproducible only where the build ran).
