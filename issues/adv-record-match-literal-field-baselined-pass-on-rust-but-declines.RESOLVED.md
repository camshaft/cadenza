# Baseline mismatch: "record match field may be a LITERAL..." baselined PASS on rust+rust-async but DECLINES

**Reporter:** breaker (2026-07-19), confirmed by corpus-bugfix on trunk ad69de3cf. **Severity:** baseline-vs-behavior mismatch (NOT a miscompile — a decline recorded as pass, a "baseline lie").

## Finding
The case "a record match field may be a LITERAL that probes the field by equality" (05-compound-types, pinned by 12297de8a) is recorded **pass** in .gate-baseline-rust (line 1630) AND -rust-async, but on current trunk it **declines** on both: `cargo xtask gate spec/semantics/05-compound-types.sexp --target rust --case "record match field may be a LITERAL"` -> verdict TODO ("compiler can't compile it yet"; main 3->want 4, main 9->want -1). The record-match literal-field probe is realized on WASM (computes 4/-1, PASS) but NOT on the rust backend.

## Root (either)
(a) baselined rust=pass without it actually passing, or (b) a later commit regressed the rust arm.

## Routing
ROUTED to v-patterns (their pin 12297de8a + record-match territory). FIX: either implement the rust-backend record-match literal-field probe (match wasm), OR if genuinely rust-not-yet, re-baseline the case to todo on rust/rust-async (gate --save) so the baseline stops lying. breaker left the committed pass value (did not hide it) so gate --check surfaces the live decline. Not spawning.

---
RESOLVED-PENDING-MERGE (v-patterns, 2026-07-19, MR 3f4a728e9): confirmed a hand-baseline error (baselined
pass on all 3 backends without verifying rust). FIX: flipped the 2 lines pass->todo in .gate-baseline-rust +
-rust-async (wasm stays pass where it computes) — gate --check now surfaces the decline instead of lying.
Verified wasm 3929/9/0 + rust-target 3782/156/0 (0 fail). Record-match BINDER cases DO pass on rust; only the
refutable literal-field probe declines. v-patterns filed a follow-up to implement the rust literal-field probe
(then re-baseline to pass). Baseline now truthful. Retire on land.

---
LANDED + SOURCE-VERIFIED (corpus-bugfix 2026-07-19, trunk 49d948964): 3f4a728e9 on trunk. .gate-baseline-rust:1677
now reads "todo	a record match field may be a LITERAL that probes the field by equality" (was pass) — the
baseline lie is corrected, matching the actual rust decline. gate --check no longer lies. The rust literal-field
probe implementation is a v-patterns follow-up (then re-baseline to pass). Fully resolved (baseline-truth restored).
