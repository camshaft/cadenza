# The byte gate found its first real miscompile — a polymorphic identity loses its Bool return — and the decline discriminator is too narrow to see it clearly

*2026-07-07*

**What happened.** A sibling landed new numeric primitives (bitwise `^`, `Int64.checked-add/-sub/-mul` returning
`Option`, `Int64.wrapping-add/…` modular) — 12 corpus cases, 2 new `numeric-model.md` MUSTs, all passing the
behavior gate (549 → 561). The byte gate moved 58/152/344 → **58/153/355** (the new numeric cases mostly land as
declines — `compiler.cdz` doesn't read `^` or the `Int64.method` form yet — plus one disagreement). Rather than
stop at "declines, expected," I ran every one of the 118 `native=ok` disagreements through `compile-run` and ran
its output, classifying by actual behavior. The result was sharper than the gate's buckets:

- **28 soft** — `compiler.cdz`'s value equals the oracle (the fold-vs-overflow-helper middle ground). Fine.
- **77 hidden declines** — the component **traps at runtime**, but its entry func is NOT a bare `unreachable`
  (it has setup instructions, or a `call` to a trapping stub, before the trap). So ask-29's decline discriminator
  — which only checks "is the entry a bare `unreachable`?" — **misses them and counts them as `disagree`.**
- **1 REAL MISCOMPILE** — `(def (id x) x) (def (main) (id true))` compiles to a component that returns **`1`**,
  not **`true`**.
- ~12 heap/no-scalar-oracle.

The miscompile, disassembled: `compiler.cdz` types `run` as `(result i64)` and emits `i32.const 1;
i64.extend_i32_u; call 1` — it pushes the Bool `1`, widens it to i64, calls `id`, and returns an **i64**, so
`run()` yields the integer `1` and the component's lifted type is `(result s64)` where native's is `bool`. Root
cause: `id` is polymorphic (`x` is returned unchanged), but `compiler.cdz`'s return-kind machinery defaults an
unconstrained function result to i64 and does **not specialize `id`'s return to the Bool it is actually applied
to** — so calling `id` with a Bool still frames the call (and the whole program) as i64, and the Bool is returned
as the raw integer. This is the polymorphic-identity return-kind gap: the monotone return-kind fixpoint
([[the-return-kind-table-is-a-monotone-fixpoint-and-it-propagates-bool-to-any-depth]]) propagates a *body-shaped*
Bool return (a function whose body is `(< a b)`), but not an *argument-shaped* one (a function whose return kind
is whatever its argument's is).

**Why.** This is the payoff of pushing past the gate's headline buckets — and a caution about the discriminator
that made the last cycle's number honest. Two things:
1. **The byte gate found its first real miscompile**, and it took running the disagreements to find it — the
   gate said "disagree," which conflates 77 declines + 28 soft + 33 rejections + 1 miscompile. The one that
   matters (a component that *runs to a wrong value*) was 1 of 153, invisible without executing each. The
   value-first interim harness would also have caught this one (it runs `run()`), which is the reminder: **the
   byte gate is strongest for `agree` and for catching rejections/declines, but a running-wrong-value miscompile
   still needs the value check on top — no single gate dominates.**
2. **ask-29's decline discriminator is too narrow.** "Entry is a bare `unreachable`" catches only the simplest
   decline; a decline that emits a few instructions and then traps, or calls a stub that traps, slips through as
   `disagree`. 77 of the 153 disagreements are these. The discriminator should classify by **whether the
   component traps at runtime** (run it), not by a syntactic look at the entry func — then `disagree` would mean
   "runs to different bytes AND a different value," i.e. the 1 real miscompile plus the 28 soft, and the honest
   miscompile count would be ~1, not 153.

**The requirement it drove.** No new corpus case — `(id true)` is *already* pinned (`09-functions.sexp` "the
identity function applied to a boolean returns the boolean" → `true`), which is exactly how the byte gate found
the miscompile; the behavior gate stays green because *native* handles it, and the gap is purely in the
self-hosted `compiler.cdz`. Two asks handed to the compiler agent: **ask-34** — the polymorphic-return-kind
miscompile (a function whose return kind is its argument's kind, not a defaulted i64; the return-kind fixpoint
must specialize a pass-through return to the applied argument's kind, or such a call must decline rather than
mis-widen a Bool to i64) — the FIRST genuine wrong-value the byte gate has surfaced, so highest priority; and
**ask-33** — widen the decline discriminator to a runtime-trap check so the 77 hidden declines stop inflating
`disagree` (the byte gate's honest miscompile count is ~1, not 153). General lesson: **a gate's discriminator is
only as good as the failure shape it models — ask-29 modeled "decline = bare unreachable entry," but a decline
is really "traps at runtime," and the gap between the two hid 77 declines and made the one real miscompile
harder to find, not easier.** Run the artifact; the entry-func shape is a proxy, and proxies leak (the same
lesson as gap-3n's "value threshold" and the trap-oracle's "does it trap").
