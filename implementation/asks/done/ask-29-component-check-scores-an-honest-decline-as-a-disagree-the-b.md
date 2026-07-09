## 29. 🟢 `component-check` scores an honest DECLINE as a DISAGREE — DONE (decline discriminator landed) 2026-07-07

**🟢 DONE 2026-07-07 — verified by the loop.** The seed's `component-check` gained the decline discriminator.
Re-running the byte gate: **58 agree, 152 disagree, 344 DECLINE, 204 skip** (was 58 agree / 496 disagree / no
decline bucket). The 344 bare-`unreachable` decline stubs are now bucketed `decline`, not `disagree`, so the
`disagree` count (152) is the honest miscompile-or-reject frontier. Immediately paid off: it exposed that 33 of
the 152 are `native=rejected / component=ok` — the self-hosted compiler compiling ill-typed programs native
rejects (the missing-type-checker gap, now ask-30). This is exactly the fix scoped below.
Learning: `spec/learnings/2026-07-07-the-byte-level-gate-decline-discriminator-exposes-the-missing-type-checker.md`.
(Original finding below.)

**Finding.** The byte-level self-hosting gate `component-check` (now runnable, #28) compares the Cadenza-authored
compiler's output to native `cdz-rustc` BYTE-for-byte and buckets each case `agree`/`disagree`/`skip`. It has NO
notion of a decline: when `compiler.cdz` declines a construct it can't read, it emits a valid TRAPPING component
(`func 0 → unreachable`, `KError`), and `component-check` byte-compares that decline stub against native's real
output and scores `disagree`. First run: **58 agree, 496 disagree, 204 skip** — but **158 of the 496 emit the
byte-IDENTICAL 88-byte component**, which disassembles to a bare `func 0 → unreachable` decline (verified: two
different unhandled programs `(record (x 1))` / `(tuple 1 2)` → same 88 bytes; it traps when run). So the
`disagree` count conflates honest declines (records/strings/floats/effects the reader doesn't decode yet) with
genuine miscompiles (a component that RUNS to wrong bytes). Spot-checked one real non-decline disagreement:
`(effect E (op)) (def (main) 5)` compiles to `i64.const 5` (effect decl dropped) — that RUNS, so it is a true
disagreement, but it is lost among 496.

**This is the byte-level twin of #26** (the interim harness's trap-cause discriminator) and of the trap-oracle
learning — every differential gate inherits the decline-vs-result blind spot: value oracle (decline traps where
a value is wanted — visibly distinct), trap oracle (decline ≡ semantic trap — needs trap-cause check), byte gate
(decline stub ≡ wrong-bytes miscompile — needs entry-func check).

**Fix (cheapest, same shape as #26).** In `component-check`, before scoring `disagree`, check whether the
component's entry core func is a bare `unreachable` (no computational op — arith/cmp/call/const-then-check —
before the trap). If so, classify the case **`decline`** (the honest frontier), not `disagree`. Then the
`disagree` count means genuine miscompiles ONLY — a component that computed and got the bytes wrong. Until then,
read `component-check`'s output as: `agree` (58, trustworthy — byte-identical is unforgeable) is the real signal;
`disagree` = declines + real miscompiles combined, and the real miscompiles must be enumerated separately (the
non-`unreachable` disagreements) before the number is meaningful.
Learning: `spec/learnings/2026-07-07-the-byte-level-self-hosting-gate-runs-and-its-disagree-count-conflates-declines-with-miscompiles.md`.

---
