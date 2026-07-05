# Ad-hoc polymorphism: traits are dictionaries-as-values, passed explicitly, not globally coherent

*2026-07-04 (resolution revised 2026-07-05)*

> **Revision (2026-07-05).** The dictionary insight below stands; the *resolution* half is
> sharpened. This learning proposed scoped implicit resolution (Scala-`given` shape) with explicit
> passing as an escape hatch. The operator inverted that: **explicit passing is the mandatory
> mechanism, and there is no resolution engine at all** — a constrained generic takes the trait
> instance as an ordinary explicit parameter. This sidesteps every implicit-resolution hazard
> (scoped-search order, ambiguity, orphan rules, global coherence) rather than defining them away, and
> it fits "no ambient authority" applied to the type system: what implementation a use site gets is
> visible at the call. The high-frequency case that motivates implicit resolution — operators like
> `+` — does not apply, because Cadenza numerics are built-in monomorphic ops, not trait-dispatched.
> An implicit-resolution convenience MAY be added later only as a meaning-preserving elaboration that
> desugars to explicit passing. The default choice is now `options/ad-hoc-polymorphism/` →
> `explicit-dictionaries`.

**What happened.** The language needs **ad-hoc polymorphism** — a way for a generic definition to
obtain *a type's operations* (`compare` for ordering, `+` for numerics, `display` for the result
boundary), not merely to *constrain* which types are allowed. The existing constraint mechanism
(`type-system.md` §"A Generic Constraint Is A Compile-Time Predicate Over Type-Values") does only
half the job: a predicate answers "is `T` allowed here?" (yes/no) but cannot *provide* `T`'s
operations. The resolution:

- **A trait is a record-of-operations type** — a *dictionary type*. `Ord T = record { compare : (T, T)
  -> Order }`. This is just a row-typed record ([[2026-07-04-records-are-rows-open-by-default]])
  parameterized by `T`; no new kind of thing.
- **An instance is an ordinary value** of that record type. `ord-int : Ord Int64 = record { compare =
  … }`.
- **A constraint is "a dictionary value must be available"**, satisfied either by **explicit passing**
  (fully uniform with type-valued parameters — you also pass value-valued parameters) or by a
  **bounded, deterministic compile-time search over in-scope instances**, then **monomorphized away**
  by the same compile-time reduction generics already use ([[2026-07-04-generics-are-type-valued-parameters]]).

**Why.** This gap is *forced* by operations already load-bearing in the spec, independent of whether the
word "trait" is ever used:
- **`display` is the result boundary.** A compiled program's result crosses the boundary as a resource
  owning a `display` method ([[2026-07-04-host-is-value-agnostic-compiler-owns-reader-printer]]).
  `display` for a user type *is* the "provide an operation per type" problem.
- **Ordering is stated but unprovided.** `core-semantics.md` §"Ordering Where Offered Is Total" speaks
  of "a type that offers an ordering" — *offers it how?* Undefined until a type can carry a `compare`.
- **`+` is ad-hoc polymorphism.** With no silent promotion and multiple numeric widths
  (`options/type-mapping/`), `+` is either one operator resolved by operand type or per-type operators —
  the same dictionary/resolution question.

**Where on the spectrum, and what is decided *against*.** The target is the **Scala 3
`given`/`using`** / **OCaml modular-implicits** / **F# SRTP** shape — principled ad-hoc polymorphism
whose instances are first-class values — **stopping well short of Haskell**:
- **Against global instance coherence** (Haskell's "exactly one instance per type per program"). That
  is a *whole-program* invariant, and it fights **(a)** content-addressed, independently-derived modules
  (`modules-and-namespaces.md` §Dependencies Resolve By Content Address) and **(b)** reproducibility of
  "the same program." Instances are **scoped** — in-scope / explicitly imported — the coherent extension
  of the rules the module system *already* enforces (explicit imports, colliding imports rejected, no
  ambient authority). Global coherence is the opposite of those rules.
- **Against orphan rules.** They exist only to approximate global coherence; without that goal they are
  unnecessary.
- **Against higher-kinded constraint variables for now.** A type constructor is already a compile-time
  `Type -> Type` function and `Type : Type` holds, so HKTs are *expressible*; but abstracting over a
  constructor *in a constraint* (Functor/Monad) is where inference gets hard. Deferred deliberately, not
  foreclosed.

**The determinism constraint that shapes resolution.** Instance resolution MUST be a deterministic
function of the source (Constitution II/III). Scoped resolution satisfies this **only if the search
order is pinned by source** — lexical scope and import order — never by hash-map iteration or discovery
order. This is a concrete requirement to write, and it is a second reason to reject whole-program
instance search: a global search is exactly the kind of thing that drifts between builds. An **ambiguous
resolution** (two equally-specific in-scope instances) is a compile-time rejection with a
machine-readable code, never a silent pick — the same posture as `modules-and-namespaces.md`
§"Colliding Imported Names Are Rejected."

**Debuggability payoff.** Because an instance is a *value* you can name, bind, pass, and inspect, there
is no invisible dictionary and no "where did this impl come from" mystery — the Rust-quality
debuggability the operator wanted, without C++ template instantiate-then-explode and without Haskell's
coherence machinery. Explicit passing is always available as the escape hatch when resolution is
ambiguous or surprising.

**The requirements it drives.** `spec/capabilities/type-system.md` — the §"A Generic Constraint Is A
Compile-Time Predicate Over Type-Values" section is *extended* (not replaced) so a constraint may also
**provide** operations: a trait is a dictionary record type, an instance is a value of it, a constrained
generic receives the dictionary (explicitly or by scoped resolution), and resolution is monomorphized
before the boundary. New requirements: instance resolution is a deterministic function of source
(lexical/import order, never iteration order); ambiguous resolution is a machine-readable rejection;
instances are scoped, not globally coherent (no orphan rule, no whole-program search). Recorded as a new
decision **`options/ad-hoc-polymorphism/`** — `scoped-dictionaries` as the default choice. Shares the
row substrate with [[2026-07-04-records-are-rows-open-by-default]]; the numeric-operator resolution it
implies is noted for `options/numeric-model/`.
