## 36. 🟢 `compiler.cdz` emitted an INVALID component for a `let`-bound Bool — now DECLINES (compiler side) — awaiting loop re-probe

**Finding (pre-existing, surfaced by an over-decline audit after ask-34).** `(let ((b (< 1 2))) (if b 10
20))` — a `let` binding a **Bool** value — compiled to an INVALID component. The bound value `(< 1 2)`
lowers to an **i32** (Bool), but `let` locals are declared **i64** (`count-lets`/`locals-decl` declare
every let-local i64), so the emitted `local.set 0` stored an i32 into an i64 local → *"func 0 failed to
validate."* This is the `let` face of the same i64-parameter-model-vs-Bool collision as ask-34 (the
`if`-condition / `not`-operand / Bool-call-arg cases); it was NOT caused by ask-34 (the `let` path was
untouched) — it was a latent invalid-emission the value-harness's bucketing hid, found by probing Bool
values through every binding form.

**Fix (compiler side, decline-don't-miscompile).** `lower`'s `KLet` arm now checks the bound value's
kind: an i64 value stores normally; a **Bool-kinded** value DECLINES (→ `unreachable`) rather than emit
the mismatched `local.set`. Verified: `(let ((b (< 1 2))) (if b 10 20))` → a VALID trapping component
(clean decline), not invalid bytes; i64 `let`s unregressed (`(let ((x 5)) (+ x 6))` → 11, multi-binding
`(let ((x 5)(y 10)) (+ x y))` → 15); value-harness holds 23 agree / 9 soft / **0 hard / 0 error**.

**The faithful fix (deferred, ask-34/35 family).** A per-`let`-binding kind — declare the local `i32`
when the bound value is Bool — would make this `agree` (a `let`-bound Bool then used as an `if`
condition is valid Cadenza native compiles). Same underlying need as ask-35 (per-param/per-binding kind
instead of the blanket i64 model). LOW priority: the compiler's own source rarely `let`-binds a Bool;
the decline restores correctness (no invalid bytes), which is what matters.

**Acceptance signal.** `compile-run <compiler.cdz>` on `(module m (def (main) (let ((b (< 1 2))) (if b 10
20))))` traps (valid decline) — never an invalid component. Later, with per-binding kind, it returns 10
(agree with native). Corpus: pin a `let`-bound-Bool case in `02-binding-and-control.sexp` (native → 10;
today a `compiler.cdz` decline, not a miscompile).

**🟢 LOOP-CONFIRMED 2026-07-07 (Run 64).** Re-probed: `(let ((b (< 1 2))) (if b 10 20))` — was an INVALID
component (i32 Bool stored into an i64 let-local) — now cleanly TRAPS (valid trapping decline). No longer invalid
bytes; reject-don't-miscompile restored via decline (same i64-vs-Bool class + fix as ask-34). Full agreement
(a Bool-typed let-local) is the follow-on, subsumed by the broader i64/i32-kind work. Moved pending → done.
