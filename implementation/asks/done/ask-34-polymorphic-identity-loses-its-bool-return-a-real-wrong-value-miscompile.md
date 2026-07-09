## 34. ✅→⏳ `compiler.cdz` MISCOMPILES a polymorphic identity applied to a Bool — FIXED via fix (2) DECLINE — awaiting loop re-probe

> **⏳ PENDING VALIDATION 2026-07-07 (compiler-side, fix 2 = decline).** The wrong-value miscompile and
> two sibling invalid-emissions are eliminated — all now clean VALID trapping DECLINES (never a wrong
> value, never invalid bytes):
> - `(id true)` — was `run()` = `1` → now traps.
> - `(pick x)=(if x true false)` on `(pick true)` — was an INVALID component (i64 param used as an `if`
>   condition) → now traps.
> - `(neg x)=(not x)` on `(neg true)` — was INVALID (i64 param as `not` operand) → now traps.
>
> **What landed (both faces of the i64-param model's kind-polymorphism gap):**
> - **Call side:** a `KCall` with any **Bool-kinded argument** DECLINES (→ `unreachable`) instead of
>   widening the Bool to i64 (`args-have-bool`). Widening lost the Bool on a pass-through (`id`) — the
>   wrong-value miscompile. (Replaces the previous `i64.extend_i32_u` Bool-arg coercion.)
> - **Definition side:** a `KIf` whose **condition** is not Bool-kinded, or a `KNot` whose **operand**
>   is not Bool-kinded (a bare i64 parameter used in Bool position), DECLINES — was emitting an
>   i64-where-i32-needed invalid body.
>
> **Regressions clean:** i64-arg calls (`id 42`→42, `dbl 21`→42, `add`→42, recursion `dec 5`→7), normal
> Bool-condition conditionals (`if (< 3 5) …`→10, `and/or/not` of comparisons), and the compiler's own
> `main` all unchanged. Value-harness still 23 agree / 9 soft / **0 hard / 0 error**.
>
> This is fix (2) from the two options below — the honest decline. Fix (1) (specialize the return kind
> to the applied argument's kind / per-param kind inference + monomorphization) is the eventual
> `agree` — a larger change. **To confirm → done:** the loop re-probes `(id true)` / `(pick true)` /
> `(neg true)` via `compile-run` and sees a trap (not `1`, not invalid), and the byte gate's
> single wrong-value disagreement is gone.

## 34. 🔴 `compiler.cdz` MISCOMPILES a polymorphic identity applied to a Bool — returns `1`, not `true` (the FIRST real wrong-value miscompile the byte gate found)

**Finding.** `(module m (def (id x) x) (def (main) (id true)))` — the polymorphic identity applied to a Bool.
Native compiles it correctly (`run()` → `true`, lifted type `bool`). `compiler.cdz` compiles it to a component
that **returns `1`, not `true`**:

```wat
(func (;0;) (result i64)          ;; run — WRONG, should be (result i32)/bool
  i32.const 1                     ;; the Bool `true`
  i64.extend_i32_u                ;; ...widened to i64
  call 1)                         ;; id
(func (;1;) (param i64) (result i64) local.get 0)   ;; id — framed i64
;; lifted type: (func (result s64))  — declares INTEGER return where native declares bool
```

So `run()` yields the integer `1`. This is a **genuine wrong-value miscompile** — the component runs and
produces the wrong observable — NOT a decline. It is the **first real miscompile the byte-level self-hosting
gate has surfaced** (found by running every `native=ok` disagreement; it was 1 of 153, invisible in the
aggregate).

**Root cause.** `id` is polymorphic — `x` is returned unchanged, so `id`'s return kind is *whatever its argument's
kind is*. But `compiler.cdz`'s return-kind machinery defaults an unconstrained function result to **i64** and does
not specialize `id`'s return to the Bool it is actually applied to. So the call `(id true)` is framed i64, `main`
is framed i64, and the Bool `1` is returned as a raw integer. The monotone return-kind fixpoint
(`build-ktab`/`ktab-iterate`) propagates a **body-shaped** Bool return (a function whose body is `(< a b)`) — see
the depth-1/2/3 chains that are byte-identical — but NOT an **argument-shaped** return (a function whose return
kind follows its parameter's kind at the call site). Polymorphism over the i64/i32 kind boundary is unhandled.

**Why it touches the spec / self-hosting.** A wrong-value miscompile is the worst reject-don't-miscompile
outcome — worse than accepting an ill-typed program (ask-30), because the program is well-typed and native
compiles it right; the self-hosted compiler silently changes its value. `compiler.cdz` cannot self-host while it
miscompiles a polymorphic identity — the compiler's own source is full of pass-through and kind-polymorphic
helpers.

**Two acceptable fixes (either closes the miscompile):**
1. **Specialize the return kind to the applied argument's kind** — the faithful fix: `id`'s return kind at a call
   site is the caller's argument kind (monomorphize per call, or infer a kind variable). Then `(id true)` frames
   i32/bool and matches native byte-for-byte.
2. **Decline** — if the return-kind machinery can't yet specialize a pass-through return across the i64/i32
   boundary, `compiler.cdz` should DECLINE `(id true)` (emit `KError → unreachable`), not mis-widen the Bool to
   i64. A decline is honest; a wrong value is not. This converts the case `disagree → decline` immediately, and
   fix (1) later takes it to `agree`.

**Acceptance signal.** `compile-run <compiler.cdz>` on `(module m (def (id x) x) (def (main) (id true)))` either
returns `true` (fix 1, byte-identical to native) or traps (fix 2, honest decline) — never `1`.
Corpus: already pinned — `09-functions.sexp` "the identity function applied to a boolean returns the boolean"
(→ `true`); the behavior gate is green because *native* handles it, so this is a `compiler.cdz`-only miscompile
the BYTE gate catches.
Learning: `spec/learnings/2026-07-07-the-byte-gate-found-its-first-real-miscompile-a-polymorphic-identity-loses-its-bool-return.md`.

**🟢 LOOP-CONFIRMED 2026-07-07 (Run 63) — miscompile eliminated via decline (fix 2).** Re-probed all three:
`(id true)` → TRAP (was VALUE=1); `(pick true)` = `(if x true false)` → TRAP (was INVALID); `(neg true)` =
`(not x)` → TRAP (was INVALID). All now clean VALID trapping declines — never a wrong value, never invalid bytes.
The dangerous wrong-value miscompile is GONE (reject-don't-miscompile restored). NOTE: this is fix (2) — a
decline, not the byte-identical `agree` fix (1) would give. The underlying polymorphic-return-kind specialization
(a fn whose return kind follows its argument's) remains a FOLLOW-ON for `agree`, tracked as a new lower-priority
ask (ask-35). Moved pending-validation → done.
