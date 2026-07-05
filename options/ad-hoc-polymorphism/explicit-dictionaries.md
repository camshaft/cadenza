# Ad-Hoc Polymorphism — Choice: explicit-dictionaries

> **The default choice for the `ad-hoc-polymorphism` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins how an operation that varies by type is
> defined and supplied. Rationale:
> `spec/learnings/2026-07-04-traits-are-dictionaries-scoped-not-coherent.md` (the dictionary shape),
> resolved to explicit passing to avoid an implicit-resolution engine entirely.

## A trait is a dictionary record type; an instance is a value

A trait is an ordinary record type whose fields are the operations it declares (a *dictionary*). An
instance is an ordinary value of that record type, holding the concrete operations for a given type.
There is no separate trait/instance namespace and no resolution engine — a trait falls out of records
and first-class values, the same way generics fall out of type-valued parameters
(`spec/learnings/2026-07-04-generics-are-type-valued-parameters.md`).

## The caller passes the instance explicitly — nothing is resolved

A definition polymorphic over a trait takes the instance as an **ordinary explicit parameter**,
alongside any type-valued parameters. The caller supplies the specific instance it means. The compiler
resolves nothing from ambient or global scope.

This deliberately sidesteps the entire family of problems implicit trait/instance resolution creates:

- **No global coherence, no orphan rule.** Global coherence assumes one canonical instance per type
  across the whole program — an assumption a content-addressed module system cannot honor, since two
  content-addressed modules may each legitimately define an instance without either being "the"
  canonical one. With explicit passing the question never arises: there is no canonical instance to
  find, only the one the caller passed.
- **No ambiguity, no scoped-search order to define.** Two in-scope instances are not a conflict,
  because the compiler is not searching — the call names the instance.
- **No ambient authority in the type system.** Which implementation a use site gets is visible at the
  call site and never resolved behind the program's back, the same discipline "no ambient authority"
  applies to capabilities.

The usual reason to add implicit resolution — pervasive operators like `+` — does not apply: Cadenza's
numerics are built-in monomorphic operations (checked `Int64`, etc.), not trait-dispatched, so the
high-frequency case needs no dictionary at all. Ad-hoc polymorphism is left for genuinely
user-defined, lower-frequency cases, where an explicit argument is easy to produce — especially for an
agent author, for whom an explicit parameter is more legible than reasoning about what is in scope.

## Monomorphized away before the boundary

An explicitly passed dictionary is inlined at compile time by the same monomorphization that
specializes any definition applied to compile-time-known arguments, so a monomorphic component carries
no runtime dictionary lookup and no dispatch the manifest did not declare. Passing the dictionary is
therefore zero-cost at runtime.

## An implicit convenience, if ever added, is meaning-preserving sugar

If a later generation wants the convenience of implicit resolution, it MAY add it only as an optional
elaboration that provably rewrites to the explicit passing above and changes no emitted bytes — the
same meaning-preserving-layer discipline the constitution applies to verification. The mandatory
mechanism stays explicit; implicit resolution is never the floor.
