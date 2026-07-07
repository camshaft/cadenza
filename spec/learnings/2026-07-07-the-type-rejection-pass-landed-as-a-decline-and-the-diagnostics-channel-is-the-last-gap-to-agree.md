# The type-rejection pass landed as a decline — and the diagnostics channel is the last gap from decline to agree

*2026-07-07*

**What happened.** ask-30's harder half — the type-inference/rejection pass — landed in `compiler.cdz` as a
`well-typed?` check run PRE-FOLD, and the loop verified it. This closes both compiler-side subsets of ask-30 (the
`read-app` arity check landed last cycle; the type check this cycle). Genuine type mismatches now **decline**
(trap via `KError → unreachable`) instead of being mis-accepted and compiled to a wrong-but-valid component.

The verification had to distinguish "declined because the type-checker rejected the mismatch" from "declined
because an operand type is unsupported" — a real trap, wrongly read, looks like either. The discriminators:

- `(if true 1 false)` → **DECLINE.** Both branches are *supported* types (i64 int, i32 bool); the ONLY reason to
  reject is the branch-type **mismatch**. An unsupported-operand decline could not catch this — so it is the
  type-checker.
- `(if true 1 2)` → **compiles** (`i64.const 1`). Well-typed (both int) → not rejected.
- `(if 1 2 3)` → **DECLINE.** Non-Bool condition (int `1` is supported), so only a type check catches it.
- `(if true (+ 1 1) false)` → **DECLINE.** The then-branch `(+ 1 1)` would fold to `2`; it still declines,
  proving `well-typed?` runs **before** the fold — the fold cannot erase a mismatch, exactly the placement the
  fold-stays-meaning-preserving discussion (SPEC-BACKLOG #9) argued for a certain-trap/type-rejection pass.

The mid-cycle trap: I first probed `(+ 1 true)` / `(+ 1 2.0)`, saw them trap, and nearly recorded "type-checker
landing" — then disassembled and saw bare `unreachable` / a float lowering to `unreachable`, and re-read it as
"just unsupported-operand declines, no type check." Both reads were incomplete; the `(if true 1 false)`
discriminator (both operands supported, mismatch-only) is what settled it as a genuine type-checker. Probe the
DISCRIMINATING case, not the ambiguous one — `(+ 1 true)` traps for two possible reasons and can't tell them
apart; `(if true 1 false)` traps for exactly one.

**Why the byte gate barely moved (61 agree, ~137 disagree — flat).** The 21 formerly-mis-accepted
native-rejected cases now **decline**, but `component-check` still scores them `disagree`: native emits a
**coded rejection** (`CDZ0201`/`CDZ0301`/`CDZ0210`), `compiler.cdz` emits a **decline (trap)** with no code, and
the gate cannot equate a coded-rejection outcome with a trap. So the cases moved from a *miscompile* (mis-accept
— the dangerous state) to an *honest decline* (reject-don't-miscompile satisfied) without moving on the gate's
agree/disagree line. This is the same decline≠coded-rejection gap the whole type-rejection story has: **decline
is the honest floor; `agree` needs the diagnostics channel.** `compiler.cdz`'s only failure channel is a trap;
to match native byte-for-byte it must return `result<_, list<diagnostic>>` with a constructed coded diagnostic.
That is now the sole blocker for these ~21 → agree, and it is a distinct ask (the spike filed ask-40, the
diagnostics channel).

**The requirement it drove.** No new corpus case — the type-error cases are all *already* pinned (native rejects
them); the byte gate measures them, and the loop verified the mis-accept → decline flip by direct disassembly
(the run-to-a-value → trap transition on the discriminating cases). WRONG stayed 0. ask-30 updated: both
compiler-side subsets (arity + type check) landed as declines; the residue to `agree` is the diagnostics ABI
(ask-40) plus the small `read-let` well-formedness tail (2 let-form cases the fixed-arity check didn't reach).
General lessons: (1) **a type-rejection pass belongs pre-fold, and the test that proves it is there is a mismatch
whose branch would fold to a value — if it still declines, the check ran before the fold could hide it.** (2)
**to tell a type-check from an unsupported-operand decline, probe the case where every operand is supported and
only the mismatch is wrong** — the ambiguous case (an unsupported operand in a mismatched position) traps for
either reason and proves neither. (3) The arc mis-accept → decline → agree is the reject-don't-miscompile ladder
at the whole-program level: landing the check reaches `decline` (safe); the diagnostics channel reaches `agree`
(faithful) — and `decline` is the milestone that matters, because it removes the miscompile.
