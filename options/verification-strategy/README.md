# Decision — Verification Strategy

**The decision.** The concrete shape of the optional verification layers above the mandatory core: how
a refinement predicate is stated and discharged, how a machine-checked proof is expressed, and how a
static discharge is recorded without entering the reproducible byte path. The constitution requires
that verification be progressive, meaning-preserving, and that a static discharge not change emitted
bytes, but it does not fix the logic or the certificate shape, which is what this decision pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Adding a verification layer never changes the runtime meaning or the emitted bytes of a program that
  already compiled (constitution VIII; verification-layers.md §Discharge Does Not Change Emitted Bytes).
- An obligation the compiler cannot discharge is a rejection, never silently ignored (constitution
  VIII).
- A static discharge is recorded as a reproducibly checkable certificate, so a nondeterministic solver
  never enters the byte path (constitution VIII; verification-layers.md).

## Choices

- [`liquid-refinements-extrinsic-proofs`](./liquid-refinements-extrinsic-proofs.md) — refinements are
  liquid types (decidable predicate logic, SMT-discharged into a checkable certificate); machine-checked
  verification is extrinsic (about a program's behavior, not propositions-as-types), which keeps
  `Type : Type` sound; discharge must be proof-producing. The first load-bearing use is fold
  order-independence (permutation invariance), discharged by property testing / liquid refinement /
  proof. **The default.**

DEFAULT: liquid-refinements-extrinsic-proofs
