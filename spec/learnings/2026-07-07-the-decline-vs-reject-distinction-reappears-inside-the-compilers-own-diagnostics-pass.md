# The decline-vs-reject distinction reappears inside the compiler's own diagnostics pass — and a working mechanism can still be unshippable

*2026-07-07*

**What happened.** The whole seed-side pipeline for effect-based diagnostics landed this session — the artifact
ABI (ask-41), the recursive-effectful handler at both entries (ask-45/46/49), the ABI detection recursing through
a `handle` (ask-51), runtime record field access (the input read). Wiring compiler.cdz's `compile` to the
`Diag`-handler `compile-output` record proved the MECHANISM end-to-end: well-typed `(+ 3 5)` → `Ok` component,
ill-typed `(+ 1 true)` / `(if true 1 false)` / `(+ 1)` → `Diagnostics: [("CDZ0201", …)]`. Diagnostics-via-effects
is real and runs.

And yet the byte gate did not move — 65 agree / 124 disagree, exactly as before, with `compile` still emitted as
bare `Bytes`. The reason (ask-53, the sibling's, verified here): activating the diagnostics handler drove
`component-check` to **441 disagree**, because the coarse `check-node` pass FALSE-REJECTS programs native
compiles. Its `((Core.KError _) (emit-diag))` arm emits `CDZ0201` for EVERY `KError` — but `KError` has two
sources that resolve to the same node:

- a **genuine rejection** — malformed arity (`(+ 1)`), an unknown head, a real type mismatch — which native also
  rejects with a CDZ code (a diagnostic is correct); and
- an honest **decline** — a float literal, a string, `unit`, a runtime `list` — constructs native COMPILES and
  compiler.cdz simply lacks (`4.5` → native `ran → Value("4.5")`), which the emit path already lowers to
  `unreachable`. Emitting `CDZ0201` here is a FALSE rejection.

I verified the two sources have opposite correct outcomes in native (`(+ 1)` → rejected; `4.5` → runs to `4.5`),
so a pass that emits a diagnostic for both is wrong for the declines. The sibling correctly kept `compile`
bare-Bytes rather than ship a handler that turns declines into diagnostics — the mechanism is proven and dormant,
awaiting the check pass learning to tell its two `KError` sources apart (split into `KReject` vs `KDecline`, or
tag it).

**Why.** Two lessons compound here.

First: **the decline-vs-reject distinction, which this loop has been fighting across three measurement gates, is
not a measurement artifact — it is intrinsic, and it reappears wherever a tool must classify its own inability to
proceed.** The value gate needed it (a decline that lands on a trap oracle looks like a semantic trap, ask-26);
the byte gate needed it twice (a decline emitting bare `unreachable` looked like a miscompile, ask-29; a decline
that traps at runtime was miscounted, ask-33); `diagnostics.md` made it a normative requirement (a
machine-branchable rejection/decline/trap kind, ask-48). Now it appears a fourth time, at a new layer — inside the
self-hosted compiler's OWN diagnostics pass, where "I can't type this because it's a construct I don't support"
(decline) and "this is ill-formed" (reject) both resolve to one `KError` and the pass can't tell them apart. The
same conflation, one level down. The distinction is a property of the compilation relation itself — every stage
that can say "no" must say WHICH no — so it will keep surfacing at each layer until the representation carries the
kind. The durable fix is never a better heuristic for distinguishing them after the fact; it is making the two
outcomes DIFFERENT VALUES at the point they are produced (a `KReject` node vs a `KDecline` node), so no downstream
consumer has to re-derive the distinction from indirect evidence — the same conclusion ask-48 reached for the
compiler's external diagnostics, now owed internally.

Second, and sharper for the loop's own reporting discipline: **a mechanism can be proven end-to-end and still be
unshippable, and "the mechanism works" is a claim about capability, not about correctness on the corpus.** Every
piece of the diagnostics pipeline works — I can exhibit a program that emits exactly the right diagnostic. It
would have been easy, and wrong, to report "diagnostics landed, the payoff is here." The payoff is gated not on
the mechanism but on the check pass being CORRECT across the whole corpus, and there the working mechanism makes
things worse (152 → 441 disagree) until the decline-vs-reject scoping lands. The loop's job at a "mechanism works"
moment is to run the FULL gate before calling it a win — a capability demo on a hand-picked input is the easiest
thing in the world to over-read as done.

**The requirement it drove.** No corpus case — the corpus already pins both sides (the ~20 ill-typed rejection
cases native rejects with a CDZ code, and the float/string/unit/list cases native compiles); ask-53 is a
compiler.cdz check-pass scoping bug measured entirely by the existing byte gate (`component-check`), and the
acceptance signal is exactly "rejections → agree, declines HOLD as declines" against that corpus, no new case
needed. The output is this learning and the confirmation on ask-53 (the two `KError` sources verified to have
opposite native outcomes; the mechanism verified working; `compile` correctly held at bare-Bytes so the gate
stays green). General lesson: **the decline-vs-reject distinction is intrinsic to a compilation relation and
reappears at every layer that classifies a "no" — carry the kind as a distinct value where the "no" is produced,
don't re-derive it downstream; and when a mechanism first works end-to-end, run the full gate before reporting a
payoff, because a capability demo on one input says nothing about corpus-wide correctness.**
