# Decision — Units Of Measure

**The decision.** The surface and semantics of Cadenza's optional dimensional-analysis layer — how a
program attaches a unit of measure to a numeric value, how the compiler composes and checks the
dimensions of an arithmetic expression, and how the whole apparatus is erased before emission. The
constitution and the units-of-measure capability fix the *behavior* — dimensional consistency is
checked at compile time, a dimensional mismatch is a compile-time error, and a unit never survives
into the emitted component or changes a value's numeric byte form (units-of-measure.md;
[Core Principle VIII](../../constitution.md)). They do not fix the *surface*: how a unit is written,
how derived dimensions compose, how a quantity type reads, and which diagnostic a mismatch carries.
That surface — deliberately left open by the capability, which "states the behavior of the layer, not
its surface" — is the choice this decision pins.

**Why the language wants it.** Dimensional analysis is "the one piece of earlier Cadenza's identity
that survives the clean room" (units-of-measure.md): a compile-time-only check that a length is never
added to a time, a velocity is `length / time`, and a force is `mass · length / time²`, that costs
nothing at runtime because units are erased before emission. It directly serves the verification north
star — a whole class of physical-modelling and numerically-heavy bugs becomes a compile error instead
of a wrong answer — and it costs nothing a program that does not use it pays for, because it is an
optional layer over the numeric core, never baked into it.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Dimensional consistency MUST be checked at compile time, and a unit or dimension MUST NOT appear in
  the emitted component — it is erased after checking (units-of-measure.md #Dimensions Are Checked
  Then Erased). This is the refinement-erases-to-its-base-type discipline (verification-layers.md #A
  Refinement Erases To Its Base Type) applied to units: a quantity erases to its underlying numeric
  value.
- Combining quantities of incompatible dimension MUST be a compile-time error carrying the
  machine-readable diagnostic for the unsatisfied dimensional constraint (units-of-measure.md
  #Dimensional Mismatch Is An Error), and an operation that derives a dimension MUST produce the
  dimension its rule defines rather than discard the dimensional information.
- Adding a unit to a numeric value MUST NOT change the value's numeric byte form and MUST NOT change
  its runtime behavior (units-of-measure.md #Dimensional Analysis Does Not Alter The Numeric Core).
  This is the meaning-preserving obligation on every verification layer (verification-layers.md #Layers
  Preserve Meaning): the same program with every unit stripped compiles to the same bytes and runs
  identically.
- Whether the dimensional obligation is discharged MUST NOT change the emitted bytes
  (verification-layers.md #Discharge Does Not Change Emitted Bytes) — dimensional checking is a pure
  compile-time predicate that sits off the reproducible byte path, so a component derived with the
  capability included is byte-identical to one derived with it excluded from the same well-dimensioned
  source.
- The capability MUST be optional — a build may include or exclude it — and when a build is not told,
  it MUST include it (units-of-measure.md #This Capability Is Optional, #The Declared Default Is
  Include).

**Why this is an isolated decision.** A unit of measure is a compile-time-only refinement over the
numeric core: it adds a checked static layer and erases to the underlying numeric value, so it touches
no numeric byte form and no frozen contract. It rides the machinery the language already has —
type constructors are compile-time functions from compile-time values to a type (the `(Int N)`
width-indexed integers are exactly this, indexed by a compile-time natural; a quantity type is the
same shape indexed by a compile-time *unit*). So the surface adds no new core syntax: a unit is an
ordinary compile-time value, a quantity type is an ordinary type-constructor application, and
dimensional checking is an ordinary compile-time predicate over those type-values (type-system.md
#Generics Are Type-Valued Parameters, #A Generic Constraint Is A Compile-Time Predicate Over
Type-Values). It needs exactly one new diagnostic code (`CDZ0501`, the dimensional mismatch) and no
new trap — a dimensional error is always a compile-time rejection, never a runtime halt, because units
are erased before the program runs. It is a verification layer a later generation realizes, not the
seed (`options/realized-capability-set/`); until then its corpus cases carry `(needs units-of-measure)`
and the seed's behavior gate skips them.

## Choices

- [`erased-compile-time-quantity`](./erased-compile-time-quantity.md) — a quantity is a value of the
  type constructor `(Qty T u)` pairing an underlying numeric type `T` with a compile-time unit `u`
  drawn from a free abelian group over named base units (`Unit.one`, `Unit.*`, `Unit./`, `Unit.^`);
  arithmetic composes dimensions by the group operation, addition/subtraction/comparison require equal
  dimension (mismatch is `CDZ0501`), and `(Qty T u)` erases to `T` before emission so the numeric byte
  form and runtime behavior are unchanged. **The default.**

DEFAULT: erased-compile-time-quantity
