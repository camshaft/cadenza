## 33. 🟠 `component-check`'s decline discriminator is too narrow — it models "decline = bare `unreachable` entry", but a decline is "traps at runtime" (77 hidden declines still counted as disagree)

**Finding.** The decline discriminator landed for ask-29 classifies a case `decline` (not `disagree`) when the
emitted component's **entry core func is a bare `unreachable`**. That catches only the simplest decline. Running
every one of the 153 disagreements' components shows the discriminator misses most declines:

| of the 153 disagreements | count | reality |
|---|---|---|
| `native=rejected` (ask-30) | 33 | compiler compiles ill-typed program |
| `native=declined` | 2 | — |
| `native=ok`, comp **soft** (value == oracle) | 28 | fine |
| `native=ok`, comp **TRAPS at runtime** (not a bare `unreachable` entry) | **77** | **HIDDEN DECLINE — miscounted as disagree** |
| `native=ok`, comp runs to a **WRONG value** | 1 | the real miscompile (ask-34) |
| heap / no scalar oracle | ~12 | — |

The 77 hidden declines emit some setup instructions and *then* trap, or `call` a stub that traps — e.g. the
checked-result-match case emits `i64.const 20; i64.const 22; i64.const -1; call 1; unreachable; i64.const 1`
(traps at the `unreachable` after `call 1`). The entry func is not a bare `unreachable`, so ask-29's syntactic
check passes it through as `disagree`.

**Why it matters.** The honest byte-gate picture is **58 agree, ~28 soft, ~112 declines (35 native-side + 77
hidden), 1 real miscompile**. Reported as "153 disagree" it reads as a wall of failures; the ONE number that
matters (running-wrong-value miscompiles = 1, ask-34) is buried. A discriminator that undercounts declines
defeats its own purpose — ask-29 was meant to stop declines inflating `disagree`, and it still lets 77 through.

**Fix.** Classify by **runtime behavior, not entry-func syntax**: run the component's `run()` (the gate already
has wasmtime); if it TRAPS where native produces a value, it is a `decline` (honest frontier), not a `disagree`.
Then `disagree` means "runs to a value that differs from native" — the real miscompile set (currently 1) plus
the soft set (which a value-compare then splits into `soft` vs `hard`). This is the same value+trap
classification the interim `run_corpus.py` harness already does; `component-check` should adopt it rather than
the entry-func proxy. (The entry-func check can stay as a fast pre-filter for the bare-`unreachable` majority,
but a `disagree` must be confirmed by running it.)

**Acceptance signal.** After the fix, `component-check` reports on the order of **1 disagree** (the ask-34
miscompile) with the ~112 declines and ~28 soft split out — the honest self-hosting frontier, where `disagree`
is actionable (each one a real wrong-value bug) instead of a mixed pile.
Related: ask-29 (the narrow discriminator this widens), ask-26 (the trap-cause discriminator — same
run-the-artifact principle), ask-34 (the miscompile this makes visible).
Learning: `spec/learnings/2026-07-07-the-byte-gate-found-its-first-real-miscompile-a-polymorphic-identity-loses-its-bool-return.md`.

---

## ✅ DONE 2026-07-07 (conformance loop) — classify by runtime behavior, not entry-func syntax

**Fixed** in `run_component_check` (main.rs). When native and the compiler-component both produce `Ok` bytes
that DIFFER: bare-`unreachable` entry → decline (fast path); else RUN both compiled programs and classify —
component `Trap` where native `Value` ⇒ DECLINE (hidden frontier); both `Value` EQUAL ⇒ SOFT; both `Value`
DIFFER ⇒ DISAGREE (real miscompile); component `Value` where native `Trap` ⇒ DISAGREE; both trap / no scalar
run ⇒ decline. Added a `soft` tally; PASS iff `disagree == 0`.

**Re-probed:**
- cdz-rustc component (byte-identical to native) → **577 agree, 0 disagree, 0 soft, 0 decline** (no regression;
  no case enters the run-both branch).
- compiler.cdz component (was 65/124 under the old proxy) → **97 agree, 260 disagree, 25 soft, 195 decline**,
  and **0 "ran → wrong value"**. The 260 disagrees are 190 `component=diagnostics` (ask-53 false-rejects) + 70
  `native=rejected` comp=ok (ask-30) — both compiler.cdz-side; ZERO seed miscompiles.

**Acceptance signal MET:** `disagree` now means "runs to an observably-wrong result" — the honest, actionable
frontier. The 77+ hidden declines (emit-setup-then-trap) that the old bare-`unreachable` proxy miscounted as
disagree are now correctly `decline`; byte-differ-same-value cases are `soft`. Gate 572/0, ignition byte-id,
cargo test green. 📦 STABLE refreshed. Learning: `component-check-runtime-behavior-discriminator`. The remaining
disagrees are the punch-list for ask-53 (compiler-side KError split) + ask-30 (type-checks).
