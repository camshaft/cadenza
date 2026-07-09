## 43. ⚪ Right-shift over-declares one scratch local (3 vs native's 2) — a byte-fidelity gap keeping `>>` `soft` not `agree`

> **⏸️ DEPRIORITIZED 2026-07-07 (operator steer): byte-identity with native is NOT the near-term goal — "same
> results" is. Every gap in this ask is `soft` (value-correct, byte-different), so NONE of it changes results.
> Leave `soft` as-is for now; this ask is a future byte-fidelity cleanup, not current work. Recorded below for
> when byte-identity becomes a goal. The `disagree` bucket (results actually differ / native rejects, mine
> accepts) is where the real work is — see ask-30/ask-53.**

**Finding.** `compiler.cdz`'s shift emit reuses the checked-arithmetic scratch-local mechanism, which reserves a
uniform **3** scratch slots. But the two shift directions need different counts:
- **`<<` (left shift)** needs 3 (count-range guard + left-shift overflow guard) — mine declares 3, native
  declares 3, **byte-identical (agree)**.
- **`>>` (right/arith shift)** needs only 2 (count-range guard; right shift can't overflow) — native declares
  **2**, mine declares **3** (one unused slot), so `>>` is **`soft`** (value-correct → 16, but 129 B vs native's
  131 B — actually 2 bytes *shorter*; the local-count and a stash-order difference net out).

Verified (`(def (f a b) (>> a b)) (def (main) (f 256 4))` → 16 both):
```
NATIVE f: (local i64 i64)      local.get 0; local.set 2; local.get 1; local.set 3; ... i64.shr_s
MINE   f: (local i64 i64 i64)  local.get 0; local.get 1; local.set 3; local.set 2; ... i64.shr_s
```
Two differences: (1) mine declares 3 locals where native declares 2 for `>>`; (2) mine stashes both operands then
sets in reverse, native interleaves get/set. Both are value-correct.

**Why it's low priority.** This is a `agree`-vs-`soft` byte-fidelity gap, NOT a correctness bug — `>>` computes
the right value and traps correctly on out-of-range counts (verified Run 73/79). It only keeps the `>>` corpus
cases in `soft`/`disagree` instead of `agree` on the byte gate. It regressed `>>` from `agree` (Run 73, before
the shift emit shared the checked-arith 3-slot reservation) to `soft`.

**Fix.** Make the shift scratch-local count direction-specific: `>>`/`>>>` reserve 2 (count-range guard only),
`<<` reserves 3 (count-range + overflow). And optionally align the operand-stash order with native's interleaved
get/set. A small emit refinement in the shift-lowering path — no new machinery.

**Acceptance signal.** `compile-run <compiler.cdz>` on `(>> 256 4)`-shape declares 2 scratch locals and emits
byte-identical to native (`>>` moves `soft → agree`); `<<` stays agree. No value change (already correct).
No corpus case owed — the shift value/trap behavior is already fully pinned in `06-numeric-model.sexp`; this is a
byte-fidelity refinement measured by the byte gate's `agree` count.
Related: the checked-arithmetic scratch-local mechanism (ask-37) whose 3-slot reservation `>>` over-inherits;
the shifts-landing learning (`2026-07-07-shifts-landed-as-the-second-guarded-op…`).

**Update 2026-07-07 (Run 91) — the operand-stash-order half affects `<<` too (both shifts are `soft`).** Re-probed:
`(<< 1 4)` → 16 (correct) but 144 B vs native 148 B, and both declare 3 locals — so `<<`'s soft-ness is NOT the
local-count (that's `>>`-specific) but the OTHER difference this ask names: mine stashes both operands then sets
in reverse (`local.get 0; local.get 1; local.set …; local.set …`), native interleaves (`local.get 0; local.set;
local.get 1; local.set`). That stash-order difference applies to BOTH `<<` and `>>`, keeping both `soft`. So
ask-43 has two independent fidelity fixes: (a) `>>` scratch-local count 3→2, and (b) the operand-stash order
(both directions) → native's interleaved get/set. Both are byte-fidelity only (value-correct, WRONG=0); low
priority. (`<<` was byte-identical in Run 73 before the shift emit shared the checked-arith scratch mechanism;
the shared stash sequence is what drifted it to soft.)

---

**🔎 LOOP RE-PROBE 2026-07-07 (stable seed 17:35, NEW SHA `ac4e76b6…`) — the gap is BROADER than "`>>`
over-declares one": `scratch-count` is a flat 0-or-3, but native reserves PER-OP minima (`+`/`-`/`*`=1, `>>`=2,
`<<`=3). This affects byte-fidelity for ALL checked-arith + shift cases, not just `>>`. CONFIDENCE: HIGH
(disassembled both compilers, per-op).** Measured native's scratch-local count for each guarded op (isolating `f`
= the op, params runtime so nothing folds):

| op | native `f` scratch locals | compiler.cdz `f` scratch locals | over-declare |
|---|---|---|---|
| `+` `-` `*` (checked) | **1** (result slot only) | **3** | +2 |
| `>>` (arith right) | **2** (value, count) | **3** | +1 |
| `<<` (left) | **3** (value, count, result) | **3** | 0 (count matches) |
| `& \| ^` (no guard) | 0 | 0 | 0 |
| `(+ (>> a b) 1)` mixed | **2** (max of the ops present) | 3 | +1 |

**Root, pinpointed in `compiler.cdz`:** the `scratch-count` def returns `(if (< (count-checked node) 1) 0 3)` — a
flat **0 or 3**, declaring 3 whenever ANY checked/shift op is present, regardless of which. It should return the
**max scratch any single op in the body needs**: 0 if none, 1 if the heaviest is `+`/`-`/`*`, 2 if the heaviest
is `>>`, 3 if any `<<` is present (slots are shared across ops, so it's a max, not a sum). The `count-checked`
def already walks the Core and could return a per-op *weight* (1 for KAdd/KSub/KMul, 2 for KShr, 3 for KShl)
reduced by `max` instead of `+`. (Line numbers omitted — the file was actively churning during this probe;
grep the `scratch-count`/`count-checked` def names.)

**Deeper cause of the `+`=1-vs-3 gap (a lowering-strategy difference, not just a count):** native's checked-`+`
KEEPS operands on the wasm stack and reuses the function PARAMS for the overflow guard —
`local.get 0; local.get 1; i64.add; local.set 2; local.get 0; local.get 2; …` — so it needs only the result
slot (1 local). compiler.cdz's `checked-guard` def STASHES both operands to scratch first
(`local.set 3; local.set 2; …`) then works from scratch, needing 2 operand slots + 1 result = 3. Verified
disassembly of `(def (f a b) (+ a b))`:
```
NATIVE f: (local i64)          get0; get1; i64.add; set2; get0; get2; get1; get2; …    ← 1 scratch, operands on stack
MINE   f: (local i64 i64 i64)  get0; get1; set3; set2; get2; get3; i64.add; set4; …    ← 3 scratch, operands stashed
```
So matching native's local COUNT for `+`/`-`/`*` requires adopting native's stash-free strategy (reuse
params/stack), OR at minimum the `scratch-count` max-fix trims `>>` (3→2) and leaves the arith stash strategy for
a separate refinement. **The `scratch-count` max-fix alone moves `>>`-only bodies `soft→agree` (3→2) and is
independent/cheap; the arith `+`/`-`/`*` `1-vs-3` gap is the bigger, stash-strategy refinement.**

**The stash-ORDER half (ask (b), still current):** confirmed both `<<` and `>>` stash-both-then-set-reverse
(`get0; get1; set(sb+1); set sb`) via the `shift-count-guard` def (`(ISet (+ sb 1)) (ISet sb)`), while native
interleaves (`get0; set sb; get1; set(sb+1)`). This is a consequence of the `shift-binop` def pushing BOTH
operands (`lower a` ++ `lower b`) before the guard pops them; native lowers-and-stashes one operand at a time.
Same root as the arith stash difference above.

**Byte sizes (all value-correct, WRONG=0):** `>>` mine 129 B vs native 131 B; `<<` mine 144 B vs native 148 B —
mine is SHORTER (the extra local decl is outweighed by the more compact stash sequence), so `soft` here means
"bytes differ," not "bytes longer." Pure `agree`-vs-`soft` fidelity; no correctness impact. **Priority stays low**,
but the fix scope is: (a) `scratch-count` def → per-op max [cheap, moves `>>` to agree]; (b) the checked-op +
shift stash strategy (`checked-guard`/`shift-count-guard`/`shift-binop` defs) → native's stack-keeping/interleaved
form [larger, moves `+`/`-`/`*` and both shifts to agree]. Both are byte-fidelity refinements measured by the byte
gate's `agree` count.

---

**🔎 LOOP MEASUREMENT 2026-07-07 (stable seed 17:44, component-check over spec/semantics: 95 agree / 25 soft /
94 disagree / 364 decline / 204 skip). The `soft` bucket has (at least) THREE independent byte-fidelity causes —
this ask (scratch-locals) is only one. ALL are value-correct; in ALL THREE compiler.cdz is MORE COMPACT than
native. So `soft` reflects the seed being deliberately verbose, not compiler.cdz being wrong. CONFIDENCE: HIGH
(disassembled each).** Per-op agree/soft measured directly (mine vs native bytes, runtime params so nothing folds):

1. **Scratch-locals + stash strategy (THIS ASK):** every checked `+`/`-`/`*` and both shifts are `soft`; every
   non-scratch op (`&`/`|`/`^`, comparisons, identity, `/`, `%`, call-chain, nested-`if`) is `agree`. So the
   scratch-local divergence is the SOLE byte-fidelity blocker for the whole checked-arith + shift family (and it
   propagates: an `if`/`let` wrapping a `+` is `soft` too). `+`=148→140B, `-`=148→140B, `*`=151→143B, `>>`=131→129B,
   `<<`=148→144B (mine shorter each).
2. **`not` lowering (SEPARATE soft source):** `(not (< a b))` → native `i64.lt_s; if (result i32) i32.const 0
   else i32.const 1 end` (desugars `(not x)` to `(if x false true)`); mine `i64.lt_s; i32.eqz` (the direct
   instruction, 7 B shorter). Value-identical; mine is arguably better but `disagree`s the seed's if/else desugar.
3. **Local-group ENCODING (SEPARATE soft source, invisible in WAT):** native declares ONE local group per
   let-binding/local (`01 7e 01 7e 01 7e` for 3 lets), compiler.cdz COALESCES same-type locals into one group
   (`03 7e`). The disassembly is byte-identical (`(local i64 i64 i64)`), only the binary local-decl encoding
   differs — 2×(N−1) bytes for N locals. A pure serializer choice (one-group-per-local vs coalesce-by-type).

**Upshot for whoever eventually does byte-fidelity:** matching native's `agree` requires making compiler.cdz
LESS compact in three places (verbose scratch stash, if/else `not`, one-group-per-local) — i.e. copying the
seed's exact serialization choices. This is why it's a "port the seed's bytes" task, not an optimization. Under
the current "same-results, not byte-identity" steer it is DEFERRED; the measurement is here so the scope is known
(3 causes, not 1) when it's picked up.
