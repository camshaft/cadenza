# A sum declaring a variant name twice is not rejected

*2026-07-08*

**What happened.** Adversarial probing of type declarations found that a sum declaring the same variant
name twice is silently accepted. `(type T (A Int64 | A Bool))` is accepted, and both `A`s coexist:
`(T.A 5)` constructs `(T.A 5)` and `(T.A true)` constructs `(T.A true)` — the variant `A` is bound twice
with two different payload types. The same-signature form `(type T (A Int64 | A Int64))` is accepted too.

**Why it is a break.** type-system.md #The Structural Types Are Record, Tuple, And Sum: a sum is "of
named variants" whose shape is "its variant names with their payload types"; #Structural Values Are
Comparable Only When Their Shapes Match speaks of a sum's "variant SET"; #A Match Is Exhaustive Against
The Sum Type's Variant Set checks exhaustiveness against that set. For the variant set to be well-defined
the variant names must be distinct, so a sum declaring `A` twice is ill-formed and MUST be rejected
(CDZ0201) — the same duplicate-member ill-formedness a record with a duplicate field, a module with a
duplicate definition, and an effect declaring an operation twice are rejected for. Two `A`s with two
payload types is an ambiguous variant the closed variant set forbids.

**Root cause (likely) — the sum-declaration elaboration registers variants without a duplicate-name
check.** The pass that reads `(type T (A … | A …))` and builds the sum's variant table inserts each
`(variant payload)` without checking whether the name is already declared in that sum, so the second `A`
overwrites/coexists with the first. The record path, and (as of c41 and c44) the module-definition and
effect-operation paths, already reject a duplicate member; the sum-variant path does not.

**The lesson (the recurring family) — the fourth closed name-set.** The language has FOUR closed
name-sets whose members must be distinct: record fields, module definitions, effect operations, and sum
variants. The duplicate-member rejection had landed for the first three (record always; module via c41;
effect op via c44) but not the fourth. This is the "a check proven on one form is not carried to its
sibling" family, here across the four kinds of closed name-set the language has. The fix is the same one
the other three got: check the variant names of one sum for duplicates as the variant table is built,
reusing the same duplicate-member rejection. The tell: `(record (a 1) (a 2))`, `(module … (def (f) 1)
(def (f) 2))`, and `(effect E (op f …) (op f …))` are all rejected, but `(type T (A … | A …))` — the same
duplicate in the same kind of set — is accepted.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a sum declaring a variant name twice is
a type error" — `(type T (A Int64 | A Bool))` MUST reject CDZ0201, the sum-variant companion of the
record-field duplicate cases above it (and of the module-definition and effect-operation duplicate cases
in their files). Gated `(needs sum-type-declaration)`, which the seed realizes, so the behavior gate runs
and catches it (expected reject CDZ0201, observed a running component). A generation that does not yet
detect a duplicate variant name declines rather than binding one.
