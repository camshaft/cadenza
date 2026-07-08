# Compound coverage lands const-first because folding needs no runtime heap — the agree count on compound cases can rise while the hard part hasn't started

*2026-07-07*

**What happened.** The compound-types coverage frontier (the largest decline cluster, ~139) has been advancing:
3 → 6 → 9 agree over the last few cycles, byte gate holding 0 disagree. But probing WHICH compound cases land
shows a sharp gradient. This cycle's win was "constant-compound projection folding" — `(tuple.0 (tuple 7 9))` →
7, a projection of a LITERAL compound to a scalar, which now agrees. Meanwhile the two forms that touch a
RUNTIME-built compound both still decline:

- `(def (mk n) (tuple n 9)) (tuple.0 (mk 5))` — project a scalar off a *runtime-built* tuple → decline
- `(def (f n) (tuple n 1)) (f 3)` — return a *runtime* tuple as the result → decline

So the compound cases fall in a specific order: **compile-time-KNOWN compound operations that fold to a scalar
land first; anything involving a runtime-built compound (as a projection source OR as a result) waits.**

**Why.** The order is not arbitrary and not effort — it is what each case REQUIRES of the runtime. A constant
compound projection needs NO runtime compound at all: `(tuple.0 (tuple 7 9))` is fully known at compile time, so
the compiler const-folds it to the scalar `7` and emits `i64.const 7` — the tuple never exists at run time, no
value-heap allocation, no tagged node, no renderer. It is a compound case in SYNTAX only; in the emitted code it
is a scalar. A runtime compound is the opposite: `(mk 5)` produces a tuple whose element isn't known until run
time, so the value must actually be BUILT on the value heap (a tagged node, an allocation), then either projected
from at run time (`tuple.0` indexing a heap slot) or rendered as the program result (the type-directed renderer
walking the heap node). That is the whole runtime-compound machinery — the M2 target — and none of it is needed
for the const case. So "compound coverage" splits cleanly into two tiers with a large capability gap between them:
the const-foldable tier (cheap, no runtime support) and the runtime-heap tier (the substantial cascade).

The measurement consequence, and the reason to write this down: **the agree count rising on compound cases can
mask that the hard part hasn't started.** 3 → 6 → 9 agree in 05-compound-types reads like steady progress on "the
compound frontier," but every one of those wins is in the const-foldable tier; the runtime-heap tier (the bulk of
the 132 still declining, and the thing the coverage map called the highest-leverage cascade) is untouched. A loop
reporting "compound coverage advancing, 3→9" without distinguishing the tiers would overstate how close the M2
runtime-compound capability is — the cheap tier fills in first and inflates the count while the expensive tier,
which is most of the cluster AND the cross-file cascade (strings/bytes/list results ride the same value-heap
machinery), sits at zero. So when tracking coverage of a feature that has a const-foldable subset, split the
count: const-folded wins are real but say nothing about the runtime capability; only a runtime-built instance
moving decline → agree signals the hard machinery landed. The discriminator is exactly the probe pair above — a
literal compound vs. a call-produced one — and the gap between their verdicts measures the distance to the real
capability.

**The requirement it drove.** No new corpus case — both tiers are already pinned (the corpus has const-compound
projections AND runtime-compound results/projections, which is precisely why the byte gate could show the const
tier landing while the runtime tier held). The output is this learning and the confirmed gradient (const tuple
projection → agree; runtime tuple result and runtime tuple projection → both still decline), plus the sharpened
coverage-tracking rule for the ask-57 frontier map: count const-folded compound wins separately from
runtime-compound wins, because only the latter measure progress toward M2. General lesson: **a feature with a
const-foldable subset covers const-first, because folding emits a scalar and needs none of the runtime machinery
the general case does; so a rising agree count on that feature can be entirely the cheap tier while the expensive
tier (the actual capability, often the larger share and the cross-cutting cascade) sits at zero — track the two
tiers separately, and use a literal-vs-call-produced probe pair as the discriminator for which one is actually
landing.**
