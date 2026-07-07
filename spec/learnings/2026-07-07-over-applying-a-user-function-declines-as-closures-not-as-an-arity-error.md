# Over-applying a user function declines as "partial application needs closures" — not the arity error the corpus says it mirrors

*2026-07-07*

**What happened.** A mid-refactor `compiler.cdz` (the spike was adding a second parameter to `kind-of`
but hadn't updated its definition) called a 1-parameter function with 2 arguments, and the seed
declined: *"call to `kind-of` with 2 args, expected 1 (partial application needs closures)."* Reducing
that to a minimal probe — `(def (f x) (+ x 1)) (f 5 9)` — reproduces the exact decline. The finding is
an asymmetry with what the corpus records for the parallel constructor case:
- `(Some 1 2)` (over-applying a single-arity **constructor**) → the corpus records `(error CDZ0201)` —
  a **type error** (applying a non-function: `(Some 1)` is a complete Sum, applied to `2`), and its
  prose (`09-functions.sexp` §"over-applying a single-arity constructor") explicitly states it is
  "arity-checked **the same way** an over-applied user function is (`(f 5 99)`)".
- `(f 5 9)` (over-applying a **user function**) → the seed **declines** "partial application needs
  closures", and there is **no corpus case** pinning it — despite the prose claiming the equivalence.

So the corpus *asserts* the two are arity-checked identically but only *pins* the constructor case, and
the seed's actual behavior on the user-function case diverges: it frames the extra argument as an
attempted *partial application* (needing runtime closures it lacks) rather than as the
*apply-a-non-function* type error it is (`(f 5 9)` = `((f 5) 9)`, applying the Int64 `6` to `9`).

**Why.** Two points, one about the language surface and one methodological. First, over-application of a
user function *is* the apply-a-non-function error, by the same single-arity desugaring the constructor
case invokes (`(f a b)` = `((f a) b)`), so its recorded outcome should be `CDZ0201`, matching the
constructor — the corpus prose is right that they are "the same way," but the seed treats them
differently, classifying the user-function over-application as a closure-feature gap instead of a type
error. That is a real divergence between the recorded semantics and the seed, on a case the corpus
gestures at but never pins. Second — and this is why it stayed a learning rather than a corpus case —
**pinning the user-function over-application case FAILed the gate through a cross-case interaction**:
adding `(def (f x) …) (f 5 9)` flipped an *unrelated* passing case (`(let ((ctor None)) (ctor unit))`,
which binds the prelude constructor `None` and applies it) to a wrong rejection *"CDZ0401: undeclared
capability: ctor"*. The seed's classification of a name in **head position** — is it a bound value, a
constructor, a capability/effect, or an over-applied function? — is evidently sensitive to corpus-wide
state in a way that a new over-application case perturbs. That fragility is itself the deeper signal:
head-position name resolution has overlapping, order-sensitive classification paths (value vs.
constructor vs. capability vs. arity-error), and the over-application decline ("needs closures") is one
symptom of the same tangle that misclassifies `ctor` as a capability.

**The requirement it drove.** **No corpus case** — pinning `(f 5 9) → (error CDZ0201)` broke the gate
via the cross-case interaction above, and the corpus discipline forbids leaving a FAIL; so the finding
is recorded here and as **SPEC-BACKLOG item 21** rather than as a case (it can be pinned once the seed
classifies user-function over-application as `CDZ0201` and the head-position classification is
robust). The requirement it argues for: over-applying a user function should carry the **same `CDZ0201`
outcome** the corpus already records for over-applying a constructor (they are the same
apply-a-non-function error), and the seed's "partial application needs closures" decline should become
that arity/type rejection — with the caveat that the fix must also stabilize head-position name
classification, since a naive addition of the case destabilizes the `constructor`/`capability`/`value`
disambiguation (the `ctor`-misread-as-capability regression). The minimal reproducer and the
cross-case interaction are recorded in the backlog so the seed fix is scoped: it is not just "emit
CDZ0201 for over-application" but "make head-position name classification total and order-independent
across value / constructor / capability / over-applied-function." Meanwhile the spike's own trigger was
a transient WIP inconsistency (a `kind-of` call/def arity mismatch mid-edit), not a durable compiler
state — noted so it isn't mistaken for a compiler regression.
