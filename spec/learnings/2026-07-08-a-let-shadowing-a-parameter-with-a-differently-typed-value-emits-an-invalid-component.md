# A let shadowing a parameter with a differently-typed value emits an invalid component

*2026-07-08*

**What happened.** Adversarial probing of shadowing found another invalid-component break (the
worst outcome class). `(def (f x) (let ((x true)) x))` applied to `(f 99)` produces a component
that fails wasm validation ("failed to compile: wasm[0]::function[0]"). The parameter `x` (used
at Int64 by the call) is shadowed by `let x = true` (Bool), and the body returns the inner `x`,
so `f` returns the Bool `true`. The program is well-typed — the different-name analogue `(def (f
x) (let ((y true)) y))` returns `true`, and the non-parameter nested shadow `(let ((x 99)) (let
((x true)) x))` returns `true`. Only a same-name shadow of a *parameter* with a differently-typed
value emits the invalid component (both `let` and `do`-def shadow forms; a match-arm let shadow
works).

**The distinguishing conditions, isolated.** The invalid component requires all three:
- the shadowed binding is a **function parameter** (not a nested `let`);
- the shadowing binding has the **same name** as the parameter (a different name works);
- the shadow's value needs a **different wasm valtype** than the parameter's slot (Int64 param
  → Bool or Float let both invalidate; a Bool param → Int64 let and an Int64 param → Int64 let
  both work).

So the compiler reuses the parameter's local SLOT — allocated with the parameter's valtype (e.g.
i64 for an Int64 argument) — for the shadowing binding's value of a different valtype (i32 for a
Bool, f64 for a Float), then stores/returns that value through the wrongly-typed slot, producing
a wasm type error that fails validation.

**Why it is a break.** core-semantics.md #Shadowing Is Well-Defined: a shadowing binding takes
effect for references in its scope — and a `let` may bind a value of any type, so shadowing a
parameter with a different type is well-defined, not ill-formed. self-hosting-and-bootstrap.md #An
Unsupported Construct Is Declined, Not Miscompiled: a not-yet-handled construct MUST decline, never
emit an invalid or divergent component. An invalid component is neither a decline nor a valid
component — the floor is a decline, and this is below it.

**Root cause — slot reuse keyed on name, not on (name, type).** The local-slot allocator appears to
give a parameter-shadowing `let` the parameter's existing slot (same name → same slot), but a slot
carries a fixed wasm valtype. When the shadow's value has a different valtype, the store/load
against the parameter's slot is ill-typed wasm. A non-parameter nested shadow allocates a fresh
slot (so it works), and a different-name binding allocates a fresh slot (works) — only the
same-name-parameter path reuses. The fix is to allocate a fresh local for a shadowing binding whose
type differs from the shadowed one, rather than reusing the shadowed binding's slot by name.

**The lesson (a third invalid-component finding, same shape as the tuple.N-param one).** Two things
that can differ — a binding's name and its wasm valtype — were conflated: the slot allocator keyed
reuse on the name alone, so a same-name/different-type shadow collided. The tell was the asymmetry:
same-name shadow invalid, different-name shadow fine; different-type shadow invalid, same-type
shadow fine. When a slot is reused by name, its type must match, or a fresh slot is required —
"same name" does not imply "same representation." A decline is the floor; emitting a component that
fails validation is below it, and the working non-parameter and different-name paths prove the value
is representable.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"a let shadowing a parameter
with a differently-typed value is not an invalid component" — `(def (f x) (let ((x true)) x))`
applied to `(f 99)` MUST yield `true` (or decline), never emit an invalid component. Native seed;
the behavior gate catches it (expected output true, observed "emitted invalid component: … failed
to compile"). A generation that cannot yet allocate a fresh slot for a differently-typed
parameter-shadow declines rather than emitting an invalid component.
