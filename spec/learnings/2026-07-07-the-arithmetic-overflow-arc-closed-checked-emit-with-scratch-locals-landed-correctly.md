# The arithmetic-overflow arc closed — checked emit with scratch locals landed, and the wrong-value frontier is now empty

*2026-07-07*

**What happened.** The runtime `+ - *` overflow miscompile (ask-37) — the compiler emitting bare
`i64.add/sub/mul` that WRAP on overflow where the spec requires a TRAP — is fixed and verified. This closes an
arc that ran across several cycles and is worth recording as a whole, because the intermediate states were
instructive:

1. **Miscompile** — bare opcode: `(* Int64.max 2)` → -2 (silent wrong value). The worst outcome.
2. **Crash** — a first checked-emit attempt added the overflow guard but with an unreserved scratch-local base
   `sb`, so `local.set (sb+2)` aliased a live local and the compiler.cdz component stack-overflowed on any runtime
   `+ - *`. Broken, but a TRAP — safe under reject-don't-miscompile.
3. **Reverted** — the crash was fixed by reverting to the bare opcode, which restored the miscompile (a step
   *backward*: a safe crash traded for an unsafe wrong value).
4. **Fixed** (this cycle) — the checked emit relanded with the scratch-local reservation correct: `sb` past
   params + let-locals, `locals-decl` declaring the 3 i64 scratch slots. Verified: overflow TRAPS (`* MAX 2`,
   `+ MAX 1`, `- MIN 1`, `min × -1` all trap), in-range computes (`- 10 2` → 8, `* 6 7` → 42), and NESTED checked
   ops share the scratch slots correctly (`(* (+ a b) c)` with 2 3 6 → 30, no aliasing, no crash). Byte-gate
   declines dropped 369 → 335 (34 runtime-arith cases left the frontier), and the corrected full-oracle
   dangerous-bucket sweep reports **WRONG = 0** — the arithmetic-overflow class is gone and nothing regressed in.

**Why.** The arc is a clean demonstration of the reject-don't-miscompile *ordering* the loop has been arguing:
`wrong-value < crash < decline < correct`. Every transition that moved *up* that ordering was progress even when
the code was still broken (miscompile → crash was progress; the crash was a worse *build* but a safer *failure*),
and the one transition that moved *down* (crash → reverted-miscompile) was the regression, even though it "fixed
the crash" and restored a green-looking state. The final fix is the top of the ordering (correct: overflow traps,
in-range computes), and it required the one piece the fold-only Lir historically lacked and that the
shifts-decline learning named first: **scratch-local allocation**. That is the deeper close — this is the first
guarded operation the compiler emits faithfully (`/ %` trapped via their opcode; `+ - *` now trap via an inline
guard over reserved scratch locals), so the "local-allocating lower pass" that shifts and checked arithmetic both
needed is now real, and the next guarded op (shifts) has the machinery it was waiting on. The nested-ops check
(`(* (+ a b) c)` → 30) is the load-bearing verification: it proves the scratch slots are *shared* correctly
across nested checked ops (a nested op fully drains its scratch before the enclosing op uses it), not just
allocated for a single op — the exact thing an unreserved or per-op-fresh allocation would get wrong.

**The requirement it drove.** No new corpus case — the overflow-traps cases (const and runtime, all of
`+ - *`) and the in-range arithmetic cases were already pinned; the byte gate measured the fix directly (34 cases
moved decline → the value/trap-correct side, WRONG=0). ask-37 moved open → done with the verification evidence
(all three ops, in-range, overflow, and nested). The durable output is this arc summary and the confirmation that
the **wrong-value frontier is now empty**: for several cycles the honest miscompile count was exactly the ask-37
arithmetic class; it is now zero, so where `compiler.cdz` emits a runnable component, the value is correct — the
strongest state under reject-don't-miscompile. General lesson, the arc's throughline: **when a faithful fix needs
new machinery, the intermediate states matter and their safety is ordered — prefer to move up the
wrong-value < crash < decline < correct ordering at each step, never down; a broken-but-trapping state is a
better waypoint than a working-but-wrong one, and the fix isn't done until it reaches `correct`, verified on the
composed cases (nested), not just the single-op ones.**
