# PR#945 (OracleGuard comment verbose, v-wasm-opt) + PR#946 (stale ml_test_jobs outer doc, v-fleet-tooling)

Two Copilot review comments, split by owner.

## Comment 1 (verbatim) — PR#945, select.rs:1197 (+19843, +19848) → v-wasm-opt

- (id 3687759295, select.rs:1197) "The rationale comment is very long/shouty and includes internal
  cross-reference noise; it makes the core intent harder to scan. Consider tightening it to just the
  panic-unwind guarantee and nesting safety. This issue also appears in the following locations of the
  same file: line 19843, line 19848."

### Liaison verification (confirmed on trunk 01842338f)

This is the RAII guard that FIXED the PR#942 finding (blame `82c092fb1` "panic-safe DUP_OCCURRENCE_ORACLE
via RAII restore-guard (PR#942 follow-up)"). The install-site comment (select.rs:1191-1197) is a ~7-line
all-caps rationale with an inline memory-note cross-ref + "(Copilot PR#942 id 3687515459.)". Copilot's
point: it's correct but verbose/shouty and buries the core intent (restore-on-unwind + nesting-safe).
STYLE-only — tighten to the guarantee, move the deep rationale/cross-refs out (or terser). :19843/:19848
flagged same-class (the `OracleGuard` type's own doc comments). Comment-only, behavior-neutral. Low-pri.

Owner 1: **v-wasm-opt** (select.rs, `82c092fb1`). Optional comment tightening.

## Comment 2 (verbatim) — PR#946, xtask/src/main.rs:731 → v-fleet-tooling

- (id 3687810109, xtask/src/main.rs:731) "The outer doc comment for `ml_test_jobs` (above this helper)
  still states the default is `min(cores, 2)` and includes a long 'WHY 2, not 4' rationale. With the
  default now restored to 4, that documentation is stale and contradicts the actual behavior; please
  update or remove the 2-cap explanation so operator-facing guidance matches the code."

### Liaison verification (confirmed on trunk 01842338f — CONFIRMED stale)

The restore commit (`9b63a87bd` "restore CDZ_ML_JOBS default 2→4") updated the INNER comment in
`ml_test_jobs_from` (main.rs:724-731: "Cap the default at 4 (restored from a temporary 4→2 downgrade)…")
AND the code (`cores.clamp(1, 4)`). BUT the OUTER doc comment on `ml_test_jobs` (main.rs:697-706) is STILL
stale: "the default is deliberately conservative — `min(cores, 2)`, not all cores. WHY 2, not 4 (pr-sync
systemic-timeout report…) … so 2 jobs still collapse the serial ~45min sum…". So the operator-facing doc
directly contradicts the now-4 default — the 2→4 restore updated the inner but not the outer comment. Fix:
update/remove the "min(cores, 2)" + "WHY 2, not 4" rationale in the 697-706 doc to match the restored-4
behavior (the inner comment at 724 already has the correct "why 4 now" story to mirror). Doc-only,
behavior-neutral.

Owner 2: **v-fleet-tooling** (`xtask/src/main.rs` gate harness, `9b63a87bd`). Update the stale outer
`ml_test_jobs` doc (min(cores,2)/WHY-2) to the restored-4 story.
