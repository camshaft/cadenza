# Decision — Realized Capability Set (per generation)

**The decision.** The concrete set of capabilities a generation *realizes*, against which its
behavioral-witnessing obligation is judged (conformance-gate.md §"A Generation Is Judged Against The
Capabilities It Realizes"). This is the behavior-gate analogue of the requirement-gate's ignition
subset in `.duvet/bootstrap.toml`: the requirement gate scopes which *requirements* a generation must
cite; this scopes which capabilities' *behaviors* a generation must witness in the corpus.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A generation's behavioral-witnessing obligation ranges over the capabilities it realizes, not every
  included capability (conformance-gate.md §"A Generation Is Judged Against The Capabilities It
  Realizes").
- The seed generation's realized set is pinned at this declared-default location so two seed builds
  judge behavioral witnessing against an identical set (same section, 4th sentence).
- A capability the language includes but a generation does not realize is not load-bearing for that
  generation's behavior gate (same section, 2nd sentence).

## Distinction from `included` and `excluded`

- **Included / excluded** (build-modes.md; conformance-gate.md §"An Excluded Optional Capability Is
  Not Load-Bearing") is about the *language* a build targets — an optional capability is in or out.
- **Realized** is about a *generation* — which capabilities that generation's compiler actually
  implements. The seed generation realizes a subset of the included language; later generations
  realize more, until the language is fully realized. A capability can be *included* (in the language)
  yet *not realized* (by the seed) — e.g. effect-tracking and generics are included by default but are
  later-generation work per the staged plan (`options/bootstrap-strategy/`).

## Choices

- [`seed-ignition-set`](./seed-ignition-set.md) — the capabilities the operator-synthesized seed
  realizes: the ignition subset plus the numeric, collection, and compound-type behaviors the committed
  corpus already exercises. **The default.**

DEFAULT: seed-ignition-set
