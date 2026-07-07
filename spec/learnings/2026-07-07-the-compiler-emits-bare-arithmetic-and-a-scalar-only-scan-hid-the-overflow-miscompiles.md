# The compiler emits bare arithmetic that wraps instead of trapping — and my own scalar-only scan hid it

*2026-07-07*

**What happened.** A quiet cycle (no new spike code) turned into the completeness sweep that matters most: run
every `native=ok` byte-gate disagreement and check for a wrong value — the dangerous bucket that must never
silently grow. The scan came back clean: **108 native=ok disagreements = 28 soft + 77 hidden declines + 3 other,
0 WRONG**, confirming ask-34's `(id true)` miscompile is now a decline. I nearly recorded "honest miscompile
count = 0" and moved on. But the **3 "other"** cases — the ones my scan bucketed as "no scalar oracle" and
skipped — were worth a direct look, and they were not harmless: they are TRAP-expecting corpus cases (their
oracle is `(trap …)`, which my scalar-oracle parser returned `None` for and the scan dropped), and `compiler.cdz`
**runs them to a value instead of trapping**:

- "a runtime multiplication that overflows traps" → `compiler.cdz` returns **-2**
- "a runtime subtraction that overflows traps" → returns **9223372036854775807**
- "min × -1 overflows" → returns Int64.min

Probing directly confirmed a whole class: `compiler.cdz` emits **bare `i64.add`/`i64.sub`/`i64.mul`**, which wrap
mod 2⁶⁴ and never trap, where the spec's default `+ - *` MUST trap on overflow. `(+ Int64.max 1)` → MIN, `(-
Int64.min 1)` → MAX, `(* Int64.max 2)` → -2; in-range arithmetic is fine. The disassembly is unambiguous — the
helper is `local.get 0; local.get 1; i64.mul`, no guard — and the const-folder doesn't trap either. Notably `/ %`
DO trap correctly here (zero-divisor and INT64_MIN/-1 handled), and the instruction set even has
`IXor`/`IEqz64` labelled "used by a checked_mul-style helper" — so the trapping discipline is present for
division but the overflow guard for `+ - *` was never wired; they lower to the bare opcode.

**Why.** Two lessons, and the second is on me.

*This is a wrong-value miscompile class, same severity as `(id true)`.* A well-typed program that must trap
instead silently computes a wrapped value — the worst reject-don't-miscompile outcome, and it is the arithmetic
core of the compiler, so it is high priority (ask-37). The fix mirrors ask-34's options: emit an overflow-checked
lowering (faithful, as `/ %` already do), or decline runtime `+ - *` until that exists (honest stopgap) — never
emit the bare wrapping opcode.

*My completeness scan had the exact blind spot this loop keeps documenting — a proxy leak in my own tool.* I
scanned for "wrong value" by comparing `compiler.cdz`'s run result to a **scalar** oracle, and filtered out cases
with no scalar oracle as "other." But a trap-oracle case IS a value check — the expected value is "traps" — and
by dropping them I dropped exactly the cases where a wrap-instead-of-trap miscompile lives. My "0 WRONG" was a
proxy for "0 wrong among cases with a scalar value oracle," not "0 wrong." This is the same failure the trap
oracle taught (ask-26: a decline and a semantic trap look alike) and the same failure "value threshold at 24" and
"decline = bare unreachable" taught — **a scan is only as complete as the oracle it consults, and filtering out an
oracle kind silently narrows the scan to where the bug isn't.** The correct scan classifies against the FULL
oracle — value cases (compare the value) AND trap cases (require a trap) — and treats "ran to a value where a
trap was required" as WRONG, not "other." The 3 I almost dismissed were the finding.

**The requirement it drove.** No new corpus case — the overflow-traps cases (const and runtime, for `+ - *`) are
already pinned in `06-numeric-model.sexp` (that is how the byte gate held them; the behavior gate is green because
*native* traps). The outputs: ask-37 (high-priority — the bare-arithmetic overflow miscompile, with the emit-a-
checked-lowering-or-decline fix), reported to the compiler agent; and a correction to this cycle's own scan
methodology, recorded here so the next completeness sweep classifies trap oracles as value checks, not "other."
General lesson, compounding the gate-discriminator ones: **the loop's own scans are gates too, and inherit the
same decline-vs-result / oracle-coverage blind spots — a green completeness result is trustworthy only if the
scan consulted every oracle kind, and "no scalar oracle → skip" is precisely how a trap-required miscompile hides
in the `other` pile.**
