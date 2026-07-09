## 37. 🟢 `compiler.cdz` emits bare `i64.add/sub/mul` — runtime `+ - *` WRAP silently on overflow instead of trapping (a wrong-value miscompile class)

> ⏸️ **FIX-1 ATTEMPTED 2026-07-07, REVERTED (self-trap integration bug).** Built the inline
> overflow-checked lowering (fix-1): 3 scratch locals per function (`sb = nparams + count-lets`),
> `checked-add`/`checked-sub`/`checked-mul` emitting the seed's exact guard sequences (compute wrapping
> result to scratch, sign-bit test / div-back, trap on overflow), routing `KAdd`/`KSub`/`KMul` →
> `checked-binop`, threading `sb` through `lower`/`binop`/`lower-args`, and `encode-body-of` declaring
> the scratch locals. The **guard sequences are correct in isolation** (unit-tested as `run()`
> components: `checked-add(20,22)`→42, `checked-mul(6,7)`→42). Caught + fixed one real bug on the way:
> `checked-mul`'s `a==0` fast path copied the seed's `return`, but the seed's is a STANDALONE function
> while this is INLINE — `return` would exit the ENCLOSING function; rewrote it as a value-yielding
> `if (result i64) … else …`. **But the compiler then SELF-TRAPPED** when compiling any `+`/`-`/`*`
> program: `compile-run` on `(add 20 22)` → *"error while executing"* in the compiler's own recursive
> reader (func 64 → 63, hitting an `unreachable`). Plain `binop` compiles the same program fine (110-byte
> component), so the checked-arith change is the cause — an INTEGRATION bug (likely the `sb` scratch-base
> vs. let-local layout, or nesting of checked ops sharing the 3 slots) that corrupts `compiler.cdz`'s OWN
> runtime arithmetic. Not root-caused under the loop; REVERTED the `KAdd/KSub/KMul` arms to plain `binop`
> and the local counts to `count-lets` only. Compiler builds + self-hosts again (23 agree/9 soft/0 hard/0
> error). The checked-op builders (`checked-add/sub/mul`, `count-checked`, `scratch-count`) remain in the
> file, dormant, for the next attempt. **Next time:** debug the `sb`/scratch-slot layout in the FULL
> compiler (not isolation) — dump `compiler.cdz`'s own emitted WAT for a `let`+arithmetic function and
> check the scratch slots don't overlap a let-local; consider a fresh scratch slot PER checked op
> (bump-allocated, not 3 shared) to rule out nesting collisions.


**Finding.** The spec's default integer `+ - *` MUST TRAP on overflow (numeric-model.md #Overflow Is Defined; the
trapping default that `checked-`/`wrapping-` opt out of). `compiler.cdz` emits **bare `i64.add`/`i64.sub`/
`i64.mul`**, which wrap mod 2⁶⁴ and never trap — so every runtime `+ - *` whose result overflows produces a
**wrong value** where native produces a trap. Verified against the running seed:

| program (helper so operands are runtime, not const-folded) | native | compiler.cdz |
|---|---|---|
| `(+ a b)` with `a=Int64.max, b=1` | trap | **-9223372036854775808** (MIN) |
| `(- a b)` with `a=Int64.min, b=1` | trap | **9223372036854775807** (MAX) |
| `(* a b)` with `a=Int64.max, b=2` | trap | **-2** |
| `(+ 20 22)` (in-range control) | 42 | 42 ✓ |

Disassembly confirms the mechanism — the helper is just `local.get 0; local.get 1; i64.mul`, no overflow guard.
The **const-folder also doesn't trap**: `(* Int64.max 2)` folds to `-2` rather than declining/trapping.

**Severity.** This is a wrong-value miscompile — the same class as ask-34 (`(id true)` → `1`), the worst
reject-don't-miscompile outcome (a well-typed program silently computes the wrong answer). It is the arithmetic
core of the compiler, so it is high priority. It was hidden from a scalar-oracle completeness scan because these
are TRAP-oracle corpus cases (the scan filtered out non-scalar oracles) — the byte gate's `disagree` bucket held
them the whole time; see the trap-cause/discriminator asks (ask-26, ask-33) for why a value+trap classification
is needed to surface them automatically.

**Context — the intent exists but isn't wired.** `compiler.cdz`'s instruction set already has `IXor`/`IEqz64`
commented "used by a checked_mul-style helper", and division/modulo DO trap correctly (a zero-divisor and the
INT64_MIN/-1 overflow are handled — compiler.cdz:948+). So the trapping discipline is present for `/ %` but the
overflow guard for `+ - *` was never emitted — they lower to the bare opcode.

**Two acceptable fixes (either closes the miscompile):**
1. **Emit an overflow-checked lowering** for `+ - *` (a guard that traps on overflow, as native does, and as
   `/ %` already do here). Reaches `agree`/`soft` — the faithful fix.
2. **Decline** runtime `+ - *` until the checked lowering exists — trap via `KError` rather than emit a
   silently-wrapping opcode. Honest (reject-don't-miscompile) but coarse (it would decline a lot of arithmetic).
   Given how central arithmetic is, fix (1) is strongly preferred; fix (2) is the stopgap if (1) can't land soon.

**Acceptance signal.** `compile-run <compiler.cdz>` on a runtime `+/-/*` that overflows either TRAPS (matching
native) or `agree`s byte-for-byte — never returns the wrapped value. In-range arithmetic stays value-correct.
Corpus: already pinned — `06-numeric-model.sexp` has the overflow-traps cases (both const and runtime) that the
byte gate flags; behavior gate is green because *native* traps correctly. This is a `compiler.cdz`-only
miscompile.
Learning: `spec/learnings/2026-07-07-the-compiler-emits-bare-arithmetic-and-a-scalar-only-scan-hid-the-overflow-miscompiles.md`.

**Loop re-verification 2026-07-07 (Run 65).** Ran the CORRECTED dangerous-bucket sweep (classifying trap oracles
as value checks, per the methodology fix): across all byte-gate disagreements, the ONLY wrong-value miscompiles
are this arithmetic-overflow class — `WRONG=3` (`* MAX 2` → -2, `- MIN 1` → MAX, `min × -1` → MIN), all overflow
traps compiler.cdz runs to a wrapped value; the `+`-overflow cases are the same class via the const/checked path.
**No other wrong-value miscompile class exists** — ask-37 is the complete open miscompile frontier. Still
unfixed as of compiler.cdz 101672 (11:05). Byte gate: 58 agree / 140 disagree / 369 decline.

**⚠️ UPDATE 2026-07-07 (Run 66) — a FIX ATTEMPT LANDED but REGRESSED (still open).** compiler.cdz (108176,
11:23) added a checked-arithmetic emit path: `lower`'s `KAdd/KSub/KMul` now route through `checked-binop` →
`checked-add`/`checked-sub`/`checked-mul`, which emit inline overflow guards over 3 scratch locals at base `sb`
(the design is sound: compute wrapping result to a scratch local, test signed overflow, `unreachable` if
overflowed). BUT it regressed: a runtime `+`/`-`/`*` now makes the **compiler.cdz component itself TRAP at
runtime** (infinite recursion — wasm function 64 self-calls to stack overflow), instead of emitting. Isolated:
`id`/`<`/`&` (non-checked ops) still compile fine; only the 3 checked ops error. The component BUILDS and
VALIDATES (31081 bytes) but crashes when its own `+`/`-`/`*` code path runs — so the scratch-local base `sb` is
likely wrong (not reserved past params+lets by `locals-decl`/`count-lets`, or the count wasn't grown by 3), or
the `sb`-threading recurses. Byte gate regressed **140 → 172 disagree** (declines 369 → 337): the runtime-arith
cases that were `soft`/`decline` now error. **This is NOT a miscompile** (it traps/crashes, never a wrong value —
reject-don't-miscompile preserved) but it is a functional regression on the arithmetic core, so it stays TOP
priority. **Agent action:** the emit sequence is right; the bug is the scratch-local reservation — ensure
`locals-decl` declares 3 i64 locals past the params+let-locals so `sb`/`sb+1`/`sb+2` (`ISet (+ sb 2)` etc.) are
valid, distinct slots, and `sb` = params + let-count. Repro: `compile-run <compiler.cdz> '(module m (def (f a b)
(+ a b)) (def (main) (f 3 5)))'` → stack-overflow trap (want 8, or byte-identical to native's checked emit).

**UPDATE 2026-07-07 (Run 67) — the crash was fixed by REVERTING the emit to bare opcodes; the miscompile is
BACK.** compiler.cdz (109582, 11:33) resolved last cycle's stack-overflow regression not by fixing the
scratch-local reservation but by reverting `lower`'s `KAdd/KSub/KMul` arms to the bare `binop a b (IAdd/ISub/IMul)`
(verified: emitted `f` for `(+ a b)` is `local.get 0; local.get 1; i64.add`, no guard). Net state:
- ✅ crash gone — in-range `+`/`*`/`-` compute correctly (42), no stack overflow; byte gate back to 140 disagree.
- ❌ the ORIGINAL miscompile is back — `(+ MAX 1)` → MIN, `(* MAX 2)` → -2, `(- MIN 1)` → MAX (wrap, not trap).
- ⚠️ **This is a step BACKWARD on reject-don't-miscompile**: it traded a safe CRASH (a trap) back for an UNSAFE
  silent WRONG VALUE. The crash was the safer state; the revert restored the wrong value.
- ⚠️ Stale artifacts: the `lower` doc comment above the arms still says "OVERFLOW-TRAP via inline checked
  guards" (no longer true), and `checked-binop`/`checked-add`/`checked-sub`/`checked-mul` are now DEAD defs.

**The fix is still the scratch-local reservation, not staying on bare opcodes.** The checked-emit defs are
correct and present (just unwired); re-route `KAdd/KSub/KMul` → `checked-binop` AND make `sb = params +
let-count` with `locals-decl` declaring 3 more i64 slots. If that can't land immediately, the honest interim is
to DECLINE runtime `+ - *` (ask-37 option 2 — trap via `KError`), NOT emit the bare wrapping opcode: a decline is
reject-don't-miscompile-safe, the bare opcode is a miscompile. Repro unchanged: `compile-run <compiler.cdz>
'(module m (def (f a b) (* a b)) (def (main) (f 9223372036854775807 2)))'` → -2 (want TRAP).

**UPDATE 2026-07-07 (spike Run — re-attempted fix-1 with the scratch-decl corrected; STILL self-traps; sharpened diagnosis).**
Re-wired `KAdd/KSub/KMul` → `checked-binop` AND fixed `encode-body-of` to declare `count-lets +
scratch-count` (= let-locals + 3) i64 locals with `sb = nparams + count-lets`. It STILL self-traps
(`compile-run '(f 3 5)'` → "error while executing"), so the scratch-DECLARATION count was not the (whole)
bug. What the investigation newly established:
- The self-trap is in `compiler.cdz`'s OWN runtime — its `lower` running `checked-add` on the input's
  `KAdd` — NOT in the emitted output. (The seed compiling compiler.cdz uses the SEED's codegen, so the
  seed's own `checked_add` helpers — the `(local i64)` / `i64.xor` funcs in the dump — are unrelated;
  don't be misled by them.)
- The checked-add/sub/mul INSTRUCTION sequences are verified correct in isolation (run() components:
  `checked-add(20,22)`→42, `checked-mul(6,7)`→42), and the `checked-mul` `return`→`if (result i64)` inline
  fix is correct. So the bug is NOT the emitted guard bytes; it is `compiler.cdz` EXECUTING its own
  `checked-binop`/`checked-add`/`seq`/`seq-go`/`count-checked`/`scratch-count` logic while lowering — one
  of those Cadenza functions traps or infinite-recurses when run.
- Prime suspects for NEXT time: (a) `seq-go` builds a `Code` from a `(list Instr)` by `List.at`+index
  recursion — verify it terminates and that a `(list Instr)` holding a nested `IIf (Kind …)` element is
  indexed correctly (heterogeneous heap-sum list); (b) `count-checked`/`scratch-count` recursion over the
  Core (does it terminate on every Core shape, incl. `KCall`'s arg list?); (c) whether `checked-binop`
  is reached with `sb` from a context where the 3 scratch slots (`sb..sb+2`) alias a live let-local or the
  recursion's own param in a RECURSIVE compiler function (the agent's stack-overflow observation — a
  clobbered induction variable). **Concrete next step:** add a minimal standalone Cadenza program that
  runs the `seq`/`checked-add` logic directly (not through the whole compiler) and `compile-run` it to see
  if THAT traps — isolating "the checked-emit Cadenza code, executed" from "the whole compiler". Current
  state: reverted to bare opcodes (Run 67); dormant checked defs retained.

**🟢 LOOP-CONFIRMED FIXED 2026-07-07 (Run 70).** compiler.cdz (110273, 12:02) landed the checked-arithmetic emit
with the scratch-local reservation fixed. Re-probed: runtime `+`/`-`/`*` overflow now TRAPS (`* MAX 2`, `+ MAX 1`,
`- MIN 1`, `min × -1` all trap), in-range computes correctly (`- 10 2`→8, `* 6 7`→42), and NESTED checked ops
share scratch correctly (`(* (+ a b) c)` with 2 3 6 → 30 — no crash, no aliasing). Byte gate declines 369 → 335
(34 runtime-arith cases moved off decline). The corrected full-oracle dangerous-bucket sweep reports **WRONG=0** —
the arithmetic-overflow miscompile class is gone and no new wrong-value regressed in. This closes the sole
wrong-value frontier that stood for several cycles (the crash-then-revert arc: emit added → stack-overflow → bare
revert → miscompile back → now correctly emitted). Moved open → done.

**🔎 TRUE ROOT CAUSE of the self-trap (spike, 2026-07-07) — it was NOT the scratch scheme.** The three
crash-then-revert cycles above all blamed the scratch-local layout / `sb` aliasing. That was a red herring.
Re-attempting fix-1 and bisecting properly (revert the `lower` arms while keeping the defs → builds+runs;
wire ONE arm → self-traps) narrowed it to the emit path, and the trap was a **non-exhaustive `match`**: the
checked guard sequences emit `Instr.IIfVoid` (`if` with a void blocktype), but `emit-instr` — labelled "the
exhaustive backend map" — had **no `IIfVoid` arm** (nor `IReturn`/`IExtendU`). When `serialize` folded a
checked sequence and reached `IIfVoid`, the unmatched variant trapped `compiler.cdz`'s OWN runtime (a seed
`match` that hits an unhandled variant traps — confirmed in isolation). The scratch reservation was correct;
the emit map was incomplete. Fix = three arms: `IIfVoid`→`0x04 0x40`, `IReturn`→`0x0F`, `IExtendU`→`0xAD`.
Re-verified end-to-end (extract the emitted component, run via `wasmtime --invoke run()`): in-range 8/42/7,
overflow `+`/`-`/`*` all wasm-trap, const `(+ MAX 1)` traps too; harness 0 hard / 0 error, trap-ok 0→1.
**LESSON (generalizes): when a lowering change makes the self-hosted compiler SELF-TRAP, first grep every IR
constructor the new path emits against the serializer's arms — a non-exhaustive `match` on a newly-emitted
variant looks exactly like an "integration bug" but is a one-line omission.** Bisection hygiene also cost a
round: `component-check` takes a compiled `.wasm`, NOT a `.cdz` (feeding it source → "failed to parse
WebAssembly module" for ANY input), and grepping its "not valid" output for "valid" matches "not **valid**" —
use `compile-run` (which prints `VALID compile component` + `compile → Ok (N bytes)`) as the build/run signal.
