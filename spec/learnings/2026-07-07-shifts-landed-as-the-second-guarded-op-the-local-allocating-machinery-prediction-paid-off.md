# Shifts landed as the second guarded op — the local-allocating-machinery prediction paid off

*2026-07-07*

**What happened.** When the arithmetic-overflow fix (ask-37) closed, it was recorded as "the first faithfully
emitted GUARDED op — the local-allocating lower pass that shifts (`<< >>`) also need is now real, so shifts are
unblocked." This cycle shifts landed, and probing `compiler.cdz` confirmed the prediction exactly: `<< >>` now
emit through the same scratch-local machinery, with their guards, byte-faithful to native.

Verified against the running compiler (via `compile-run`, byte-comparing where relevant):

| shift | compiler.cdz | native |
|---|---|---|
| `256 >> 4` / `1 << 4` (in-range) | 16 | 16 |
| `1 << 64` (count = width) | TRAP | TRAP |
| `256 >> 64` / `1 << 65` (count ≥ width) | TRAP | TRAP |
| `1 << 63` (left-shift overflow past the sign bit) | TRAP | TRAP |

Both guards fire: the **count-range guard** (count ≥u 64 → trap, so wasm's mask-mod-64 never silently turns a
shift-by-64 into a shift-by-0) and the **left-shift overflow guard** (an overflowing `<<` traps like an
overflowing `*`, per #Overflow Is Defined). The byte gate ticked up (58 → 59 agree — a shift case is now
byte-identical to native), and the standing full-oracle dangerous-bucket sweep stayed **WRONG = 0** — shifts
landed with no miscompile, no regression.

**Why.** This is the payoff of naming an architectural requirement precisely when the first instance of it landed.
The shifts-decline learning ([[a-no-scratch-local-lir-must-decline-ops-that-need-guard-locals]]) identified that a
fold-only Lir with no scratch-local allocation *cannot* faithfully emit any guarded operation — shifts were the
first declined op, but the coverage gap it named was the **local-allocating lower pass**, and it listed the
declined guarded ops (shifts, checked arithmetic) as that pass's acceptance list. When checked arithmetic forced
that pass into existence (ask-37, with its own crash-then-fix arc over the scratch-local reservation), the
prediction was that shifts would follow "for free" — the machinery, not the operator, was the blocker. This cycle
confirms it: shifts reused the exact scratch-local mechanism (`sb`-based guards) checked arithmetic built, and
went straight to `correct` (no wrong-value, no crash intermediate) because the hard part — the local allocation —
was already solved and validated. The general lesson: **when the first instance of an architectural capability
lands, the operations that were declined *waiting on that capability* become cheap wiring, not fresh work — and
naming that acceptance list at decline time is what makes the second op a verification rather than a rediscovery.**
The two faithfully-emitted guarded ops now (checked `+ - *`, shifts `<< >>`) share one mechanism; the next guarded
op (a `bin`-segment fit check, a checked narrowing conversion) is the same shape again.

**The requirement it drove.** No new corpus case — the shift behavior is *already* completely pinned, const and
runtime, in-range and guarded-trap: `06-numeric-model.sexp` has `>> 256 7` / `<< 1 7` (in-range values),
`<< 4611686018427387904 1` (overflow trap), `<< 1 64` (count = width trap), `<< 1 -1` (negative count), the
type-rejection cases (`<< 1 2.0` → CDZ0301), AND their runtime companions (`(def (sh a b) (<< a b))` for the
by-width and overflow traps). The byte gate measured `compiler.cdz` against all of them (WRONG=0, +1 agree), so
the capability is verified with no corpus addition owed. The durable outputs: this learning, and a stale-comment
flag to the compiler agent — `compiler.cdz`'s header still says "NOT YET: shifts `<< >>` … read to an unknown head
→ KError → unreachable," which the code has outgrown (the LEB128 encoders `uleb`/`sleb` themselves now use `>>`).
General lesson, the loop's recurring one applied to the compiler's own comments: **a "NOT YET" banner is a claim
to re-probe, not inherit — the code landed the capability and the comment lagged, exactly as handoff docs lag the
seed.**
