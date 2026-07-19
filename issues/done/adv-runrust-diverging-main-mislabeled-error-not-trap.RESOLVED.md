# adv: `cdz run-rust` mislabels a Never-returning (diverging) main as `error`, not `trap`

## Minimal repro
    (do (def (main) (Option.expect (None) "m")) (export main))

## Observed (trunk ad69de3cf)
- **wasm** (`cdz compile | cdz-run`): `wasm trap: unreachable` — a clean TRAP verdict. CORRECT.
- **rust** (`cdz run-rust`): `error warning: unreachable statement` — an ERROR verdict (per run-rust's
  grammar, `error` = "the emitted .rs failed to rustc — a MISCOMPILE the fuzzer files"). WRONG verdict:
  the program TRAPS (diverges), it does not fail to compile in any meaningful sense.

## Diagnosis — it's the run-rust DRIVER, not the compiler emit
The compiler's emitted module is CLEAN and correct:
    pub fn main() -> ! {
        panic!("unreachable")
    }
`main` returns `!` (Never) because a const-folded `Option.expect (None)` provably diverges (spec: expect
on an absent optional traps; core-semantics §299 + the ratified `unreachable` trap-kind, see the green
sibling cases 02-binding-and-control ~:1237/:1281).

The `error` comes from the **run-rust driver wrapper**: it calls `main()` then runs render-the-result
code after it, but since `main(): !` never returns, that post-call driver code is UNREACHABLE, and the
driver is compiled with warnings-as-errors (`-D warnings` / `#![deny(warnings)]`), so rustc turns the
`unreachable statement` warning into a hard error. So run-rust reports `error` for what is really a `trap`.

## Why it matters
`cdz run-rust` is the fuzzer's rust-vs-wasm differential ORACLE. For ANY diverging program (a provable
trap: const-None expect, const div0, const overflow, `Result.expect` on a literal Err, etc.), the oracle
emits `error` where wasm emits `trap` — a spurious cross-backend "divergence" that would either (a) falsely
flag every diverging program as a rust miscompile, or (b) mask a real rust miscompile behind the noise.
The corpus SIDESTEPS this by testing expect only on RUNTIME optionals (02-binding :1477 explicitly:
"distinct from expect on a literal optional") — but the oracle defect remains for the const/diverging path.

## Fix direction (v-rust-backend / harness owns)
The run-rust driver should, when the emitted `main` returns `!` (Never), NOT emit unreachable
post-call render code — or allow `unreachable_code` (not deny it) in the driver wrapper, or detect the
diverging-main case and treat the panic as the trap verdict. A Never-returning main is a valid, expected
shape (a whole-program trap), not a compile error.

## Family
Fresh face of the rust-diverging-code-emit family (RESOLVED:
adv-nested-both-diverge-if-wasm-declines-rust-compiles-differential,
review-rust-nested-diverging-arith-emits-method-call-on-never-e0599). Those were emit bugs; this one is
the DRIVER/oracle, surfaced by the same Never-return shape.

---
ROUTED to v-cdz-tooling (corpus-bugfix 2026-07-19, VERIFIED): run-rust DRIVER defect (emit is clean, harness
is wrong). Diverging main:! -> wasm clean trap; cdz run-rust -> "error warning: unreachable statement"
because the driver emits post-main() render code (unreachable under main:!) + compiles -D warnings -> hard
error. So run-rust reports "error" for ANY diverging/provable-trap program. IMPACT: fuzzer's rust-vs-wasm
differential spuriously flags every diverging program (cc fuzzer). FIX (v-cdz-tooling driver): (a) skip
post-call render for a Never main; (b) #[allow(unreachable_code)]; or (c) map diverging-main panic -> trap
verdict (match wasm). Not a compiler soundness bug. Not spawning.

---
RESOLVED-PENDING-MERGE (v-cdz-tooling, 2026-07-19, MR e0602434f): the run-rust driver now special-cases a
!-return export (detected via the `// cdz-return[export]: !` note) — emits a driver that just CALLS main()
(NO render, since a ! result can't be bound/rendered), rustc-compiles clean, main() panics at runtime ->
compile_and_run_rust_driver maps the panic -> trap. NB: the #[allow(unreachable_code)] option I suggested was
NOT enough (silences the unreachable warning but then !/() doesn't impl Display -> a 2nd error; not-rendering
is the real fix). Verified Option.expect(None,"m") -> run-rust trap == wasm trap (was error); value/bool/tuple
controls unchanged. run_rust_cli regression test pinned. Retire on land.

---
LANDED + CONTENT-VERIFIED (corpus-bugfix 2026-07-19, trunk 41880b4ae): e0602434f on trunk. cdz run-rust on
(Option.expect (None) "m") now reports "trap unreachable" (was "error warning: unreachable statement") ==
wasm trap. The driver skips post-call render for a !-return export. The fuzzer's rust-vs-wasm differential
oracle no longer spuriously flags diverging programs. Fully resolved.
