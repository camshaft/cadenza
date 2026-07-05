# Fold order-independence is the killer app for the verification layers

*2026-07-04*

**What happened.** The target requires a fold to produce **byte-identical output for the same set of
events regardless of delivery order** — a per-kind projection update must be **commutative or resolved
as latest-wins by the one order**, so that at-least-once and out-of-order delivery converge. This is a
*semantic* property strictly stronger than determinism and purity
([[2026-07-04-fold-modules-are-provably-pure]]): a pure fold can still be order-*dependent* (e.g. it
appends to a list in arrival order). Order-independence is exactly the kind of stated relationship the
**optional verification layers** (contracts, refinement/liquid types, property testing, proof) exist to
discharge — so the target gives those layers their first concrete, load-bearing application.

**Why this is the verification layers' killer app.** The layers were specced abstractly
([[2026-07-04-refinements-are-liquid-verification-is-extrinsic]]). The fold-order-independence
requirement makes them *necessary* for a real target module, and it exercises every tier:
- **Property testing** ([[2026-07-04-refinements-are-liquid-verification-is-extrinsic]] notes refinements
  drive generation): "for all event sets S and all permutations π, `fold(S) == fold(π(S))`" is a
  property with a seed-reproducible generator and shrinking — a fold's order-independence is checkable by
  generated permutations before any proof effort.
- **Refinement / liquid types**: a commutative-merge combinator can carry a refinement asserting
  commutativity of its step, checked by SMT over the decidable predicate logic — an *extrinsic*
  ([[2026-07-04-refinements-are-liquid-verification-is-extrinsic]]) property about the fold's behavior,
  discharged into a reproducibly-checkable certificate.
- **Proof**: the strongest folds (privileged projections the target *shadow-runs* before activation) can
  carry a machine-checked commutativity proof recorded as a witness.

**Why it fits the architecture.** Order-independence is a **CRDT-style** convergence property; the merge
must be a commutative, idempotent, associative combine, or a latest-wins resolution by the sequence
number. Expressing "this projection update is a commutative merge" and having it *checked* is precisely
"state an obligation, discharge it or reject" (Constitution VIII) — and the discharge is
**meaning-preserving and off the byte path** ([[2026-07-04-refinements-are-liquid-verification-is-extrinsic]]),
so a nondeterministic solver proving commutativity never enters the reproducible component bytes; only a
checkable certificate does. The verification layer earns its place by turning "please write a convergent
fold" into a guarantee, exactly as the effect system turns "please write a pure fold" into one.

**Consequences to hold.**
- **Order-independence is opt-in but role-motivated.** The verification layers are optional
  (Constitution VIII); a fold author engages the commutativity obligation to get it checked. The target
  can *require* the certificate for privileged folds via its activation gating (shadow-run +
  review), without the language making the layer mandatory — the layer stays optional, the target's
  governance decides when to demand it.
- **Latest-wins is the escape hatch.** Where a genuinely commutative merge is not natural, resolution by
  the one order as latest-wins is the sanctioned alternative — a total order the sequence number already
  provides, so the property is still checkable ("the update is either commutative or a latest-wins by
  sequence number").
- **This is the property-testing capability's first real corpus.** It gives
  `property-based-testing.md` a concrete, target-driven case: permutation-invariance of a fold.

**The requirements it drives.** `spec/capabilities/verification-layers.md` and
`spec/capabilities/property-based-testing.md` are annotated that a fold's order-independence
(commutativity or latest-wins-by-order) is a stateable, dischargeable obligation — by property testing
(permutation invariance), by a refinement asserting a commutative merge, or by a proof witness for a
privileged fold — discharged off the byte path into a reproducibly-checkable certificate. No new
mandatory-core requirement (the layers stay optional); this names the target-driven application that
motivates them. Composes with [[2026-07-04-fold-modules-are-provably-pure]] (purity is necessary,
order-independence is the sufficient-convergence property on top) and
[[2026-07-04-refinements-are-liquid-verification-is-extrinsic]].
