# Nominal is an orthogonal tag over any structural type (record, tuple, sum)

*2026-07-04*

**What happened.** The type system gains a representational commitment: **nominal-versus-structural
is one orthogonal axis over every structural type**, not a property of records alone. The structural
types are record (named fields), tuple (positional elements), and sum (named variants); their identity
is their shape. **Nominal** is a modifier that tags *any* of them with a name — so nominal record,
nominal tuple, and nominal sum all exist, and structural sum exists too. A nominal value is just its
underlying structural value plus a compile-time, type-system-only name tag; one runtime mechanism.
(This generalizes the first cut, which spec'd nominal-ness only for records — leaving nominal-tuple
and structural-sum as unfilled cells of the 3×2 grid the operator pointed out.)

**Why.** The corpus case *"same-shape nominal types are distinct to the compiler, structural to the
dynamic interpreter"* (`(= (Point (x 0) (y 0)) (Vector (x 0) (y 0)))` → `true` in the seed) forced
the question of how nominal and structural records relate. Two options: (a) nominal and structural
are separate kinds with separate representations, or (b) nominal is structural-plus-a-tag. Option (b)
is the uniformity the language already favors (one mechanism, no special case), and it explains the
corpus behavior directly: the tag lives only in the type system, so a generation that tracks types
(a later, typed compiler) sees `Point ≠ Vector` and rejects the comparison (CDZ0202), while a
generation that does not (the dynamic seed) sees only the shared shape `{x, y}` and compares
structurally → `true`. The divergence is not two behaviors; it is the same structural value seen with
and without the tag.

**Consequences.**
- The name tag adds **no field** to the runtime value — a nominal record and its underlying
  structural record have identical runtime representation and identical component-ABI lowering.
- Record equality in the dynamic seed is structural (shape + field values, sorted by key), name tag
  ignored — which is exactly what `cdz-rustc`'s constant-folding record equality already does, so the
  `Point`/`Vector` case compiles to `true` once record-constructor syntax `(Name (field val)…)` is
  read as a structural record with the nominal name dropped at the value level.
- A typed generation realizes the tag as a compile-time attribute on the record type, checked at
  comparison/annotation sites; it is never serialized into the value.

**Nominal types are NOT comparable across their boundary.** The point of declaring a type nominal
is that its values are *not interchangeable* with same-shape values — so a comparison across a
nominal boundary (two different nominal types, or a nominal vs. the bare structural shape) is not
"false," it is **not a valid comparison**: a type-tracking generation rejects it as a type error
(the corpus already carries `(compiler (error CDZ0202))` for the `Point`/`Vector` case). The seed's
recorded `true` is the *deferred-typing* outcome — the dynamic seed has no tags to see, so it
compares the two `{x,y}` shapes structurally — not a claim that nominal records are structurally
comparable. If a program *wants* structural comparison, it must explicitly **strip the tag** to
recover the underlying structural record (a compile-time reinterpretation, same runtime value — not
a copy). And records themselves are comparable **only when their field-name sets are identical**;
different-name records are different shapes with no meaningful equality (type error when typed; the
dynamic seed gives a defined not-equal so it stays total).

**The requirements it drove.** [type-system.md](../capabilities/type-system.md) §"User Types Are
Declarable As Nominal Or Structural" gains:
- §"A Nominal Record Is A Structural Record Carrying A Name Tag" — nominal = structural + compile-
  time tag; tag is type-system-only (no runtime field).
- §"Nominal Types Are Not Comparable Across Their Boundary" — a typed generation rejects
  nominal↔nominal and nominal↔structural comparisons; the untyped seed falls back to structural.
- §"A Nominal Record Is Convertible To Its Underlying Structural Record" — an explicit tag-strip
  escape hatch; a compile-time reinterpretation, not a value copy.
- §"Records Are Comparable Only When Their Field-Name Sets Are Identical" — mismatched field sets
  are a type error (typed) / a defined not-equal (untyped seed, stays total).

Realized in `cdz-rustc` by reading `(TypeName (field val)…)` as a structural record for value-level
purposes (nominal name dropped at the value level), and record `cval_eq` compares sorted `(key,val)`
pairs so differing key sets are not-equal (never a trap). Post the static-typing amendment
([[2026-07-04-static-typing-is-mandatory-post-pivot]]) the CDZ0202 rejection is the seed compiler's
OWN behavior, not a deferred later-generation concern.

**Nominal identity is the fully-qualified name.** A nominal type's tag is its *module path + declared
name*, not the bare name — so two sum types or two nominal records declared in different modules are
distinct even with identical structure AND identical local name. Otherwise a module could forge
another module's nominal type by re-declaring a same-shape, same-name type. This holds uniformly for
nominal records and nominal sum types. Drove type-system.md §"A Nominal Type's Identity Is Its
Fully-Qualified Name" (3 requirements). Ast, being a sum type, is itself nominal and qualified the
same way.
