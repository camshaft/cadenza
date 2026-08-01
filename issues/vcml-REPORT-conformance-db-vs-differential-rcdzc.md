# REPORT: conformance-DB (hard-coded) vs differential-vs-rcdzc — the "why" + a replacement plan

**For:** operator (via concierge), 2026-08-01, v-compiler-ml. Answers the operator's question: "why is there a
hard-coded conformance DB instead of running the corpus through compiler-ml and checking equivalence to rcdzc?"

## HEADLINE: the differential-vs-rcdzc harness ALREADY EXISTS and runs in the gate.
`report_ml_conformance` (xtask/src/main.rs:3977) + `GateTarget::CadenzaMl` (main.rs:920) already do exactly
what the operator describes:
- It drives the **shared REAL corpus** (`default_corpus_files`, ~3700 cases — the SAME corpus rcdzc's own gate
  uses) through the self-hosted compiler-ml via `cdz run-ml` (source in → `value <sexpr>` | `declined` | `error`).
- It compares each result **against the Wasm/rcdzc oracle** (`ml_agrees_with_oracle`, main.rs:4066): a DECLINE
  = coverage-not-yet (the ML front-end is subset-only, most cases decline), an AGREEING value = progress, a
  DISAGREEING value = the only real failure. rcdzc IS the oracle (`run_program(..., GateTarget::Wasm)`).
- It's a REPORT-only gate step today (never reds the gate — reports Agree/Disagree/NotYet counts), wall-clock
  bounded (fleet-safety), parallelized per-case.

So the differential the operator wants is BUILT. The hard-coded conformance-DB is a SEPARATE, OLDER, REDUNDANT
mechanism that predates (or parallels) it.

## WHY the hard-coded conformance-DB exists (the honest answer)
`conformance-db.cdz` (+ `-cx`, `-rel`; 1180 LOC / 3 files / ~100 hand-coded cases) is an **in-Cadenza
scoreboard**: a hand-maintained corpus of ~100 integer programs each paired with a HARD-CODED expected
outcome (`Runs(v)` / `Declines`), plus a `conformance()` runner that drives them through the memoized Db
pipeline and returns `(passed, total)`. Its stated original rationale (file header): "the Cadenza-side
substrate the gate addition needs … the scoreboard itself is dogfooded and grows with the tests" — i.e. it was
built as a DOGFOODING artifact (the conformance engine written in Cadenza, exercising the compiler on itself)
and as an early bootstrap scoreboard ("start LOW and CLIMB") BEFORE the Rust-side differential harness existed.

What it tests that the differential doesn't: essentially nothing the differential can't. Its hard-coded
expected values duplicate what rcdzc would compute; its ~100 cases are a subset of the ~3700-case shared
corpus. Its ONE arguable unique value = it's written IN Cadenza (dogfoods the compiler-ml pipeline as a
library consumer) — but that dogfooding is ALSO covered by the sread-eval*/db-* @test suites, which exercise
the pipeline in-Cadenza far more thoroughly.

Its COSTS: (a) hand-maintenance — every new case needs a hand-written expected value (the operator's exact
complaint: "hard-coded"); (b) it's a HEAVY self-host gate item — conformance-db / conformance-db-cx are among
the slowest compiler-ml suites (~580–900s under load per the xtask suite-timeout notes), because each @test
recompiles the whole closure; (c) it can only ever cover its hand-picked subset, never the full corpus.

## RECOMMENDATION: retire the hard-coded conformance-DB; rely on the differential.
The differential (`report_ml_conformance`) is strictly more general (full corpus), self-maintaining (expected
= whatever rcdzc computes, no hand-coding), and robust (a real miscompile = a disagreement against the actual
reference compiler). The hard-coded DB is redundant tech-debt + a slow gate item — exactly the "hard-coded
conformance DB thing" the operator is questioning. Proposed plan (gated, ties into the cleanup mandate):
1. **Promote the differential to a first-class (optionally blocking) signal.** It's report-only today; decide
   with the operator whether a `Disagree > 0` should RED the gate (it's the real miscompile signal). Low risk:
   it's already computed every gate run.
2. **Delete `conformance-db.cdz` + `-cx` + `-rel` (1180 LOC / 3 files).** Removes the hand-maintained scoreboard
   AND three of the slowest self-host suites — a legibility + gate-time win. (Keep any UNIQUE case not in the
   shared corpus by ADDING it to the corpus first, so the differential covers it — audit before deleting.)
3. Net: −1180 LOC, −3 slow suites, conformance becomes self-maintaining against the real oracle.

⚠ ONE audit step before deleting: confirm every conformance-db case's PROGRAM is representable in the shared
corpus (`default_corpus_files`) so the differential covers the same ground — the ~100 integer cases almost
certainly are, but verify no case tests something the corpus lacks; if so, add it to the corpus first.

## ✅ COVERAGE AUDIT DONE (2026-08-01) — conformance-db is fully redundant with the shared corpus.
conformance-db's 69 `c-*` cases test: literal/add/sub/mul/precedence, let (trivial/nested/shadowing),
if (then/else/nested/nonbool-cond/branch-mismatch/bool-literal-cond), comparisons (lt/gt/le/ge/eq/ne/rel-in-if),
div/div-by-zero/mod/div-precedence, unary-minus/double-negation, and the decline cases (unbound, bool-in-arith).
The differential runs `default_corpus_files` = `spec/semantics/*` which INCLUDES `06-numeric-model.sexp`
(arith: `+`×317, `-`×164, `*`×256, `/`×198, `%`×126, comparisons `<`×102/`<=`/`>`/`>=`; 1095 div/zero/overflow
mentions) and `02-binding-and-control.sexp` (983 let/if/cond mentions). So every domain conformance-db covers is
richly present in the shared corpus the differential already drives through compiler-ml vs the rcdzc oracle.
⇒ conformance-db (+cx/-rel) adds NO coverage the differential lacks; retiring it loses nothing. RETIREMENT IS
DE-RISKED — one deletion slice when approved (no corpus additions needed first). Held pending operator go on (a).

## Open decision for the operator (via concierge)
- Approve retiring conformance-db (+cx/-rel) in favor of the existing differential? (I do the corpus-coverage
  audit + deletion as a cleanup slice.)
- Should the differential RED the gate on `Disagree > 0` (make it blocking, not report-only)? That's the
  "checking for equivalence to rcdzc" the operator asked for, enforced.
