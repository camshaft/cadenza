# Generics are type-valued parameters, not a separate polymorphism mechanism

*2026-07-04*

**What happened.** The type system's generics are pinned to a specific design: a generic
definition is **an ordinary definition that takes type-valued parameters**, and monomorphization is
**the compile-time reduction the compiler already performs** on any definition applied to
compile-time-known arguments. There is no separate `∀`/parametric-polymorphism construct and no
separate trait-resolution engine.

**Why.** Two commitments the language already made combine to *give* generics for free:
1. **Types are first-class values** whose type is `Type` (§Types Are First-Class Values). `Int64` is a
   value; `Type` is its type.
2. **The compiler does compile-time evaluation** — const-folding and lambda inlining/beta-reduction
   are already how cdz-rustc specializes a definition applied to known arguments.

Given those, a generic is just a definition with `Type`-valued parameters, and a parameterized type
like `Option Int64` is `Option` (a compile-time **function from types to a type**) *applied* to
`Int64` — ordinary application, not special syntax. Monomorphizing `f<Int64>` is the same reduction
as inlining `(f Int64)` with `Int64` bound. This keeps the language uniform (one mechanism: first-
class values + compile-time application) instead of bolting a second, parallel type-level computation
system onto it.

**The rules this forces.**
- **Type parameters resolve at compile time.** A type-value must never flow from runtime data into a
  type-determining position — otherwise monomorphization (and the type-erasure the component ABI
  requires) is impossible. Every generic use reduces to a concrete type before the boundary.
- **Constraints are compile-time predicates over type-values**, checked by the same compile-time
  evaluation — not a trait/typeclass resolution pass. A failing constraint is a compile-time
  rejection with a machine-readable code, never a runtime failure.
- **Monomorphization is not a distinct lowering path.** It is the compiler's existing compile-time
  application/reduction, so a generic instantiation and an ordinary compile-time-applied definition
  compile the same way. (In cdz-rustc terms: the same `resolve_lambda` / const-reduction machinery
  that already inlines statically-known applications — see the compound/lambda folding — is what
  specializes a generic; no new pass.)

**Relationship to the type universe.** Generics compose with the orthogonal nominal/structural axis
([[2026-07-04-nominal-is-orthogonal-tag-over-structural-types]]): a generic type constructor produces
a structural type (record/tuple/sum) parameterized by its arguments, and a nominal wrapper over it
tags the *instantiated* type with its fully-qualified name. `Ast`, being a sum, needs no generics; a
`List<T>` is a type constructor applied at compile time.

**The requirements it drove.** [type-system.md](../capabilities/type-system.md) §"Generics Are
Type-Valued Parameters, Not A Separate Polymorphism Mechanism" (3 reqs: type-valued parameters;
compile-time resolvable; type constructors are compile-time type→type functions), §"A Generic
Constraint Is A Compile-Time Predicate Over Type-Values" (2 reqs), §"A Generic Definition Is
Monomorphized Before The Component Boundary" (2 reqs) — replacing the prior thin three-line
"Generics Are Parameterized And Monomorphized". Not yet exercised by cdz-rustc (no generic corpus
case realized without `(needs …)`); this is the design the type-checker realizes when generics land.
