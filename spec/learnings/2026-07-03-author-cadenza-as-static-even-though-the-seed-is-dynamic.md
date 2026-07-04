# Author Cadenza as a static language, even though the seed evaluates it dynamically

*2026-07-03*

**What happened.** The seed reference interpreter is dynamic — it realizes evaluation without
type-checking (the Core Principle VII bootstrap carve-out). The first Cadenza artifact, the compiler
(`cadenza/compiler.cdz`), was therefore written in a dynamic idiom: it takes apart the program AST
with 56 runtime kind-reflection calls (`Ast.is-int`, `Ast.is-list`, `Ast.is-name`, … then
`Ast.int-value`, `Ast.list-elems`, …), a chain of `if (Ast.is-X node) … (Ast.X-value node)` tests
that only makes sense in a language where a value's kind is inspected at runtime. That reads
naturally under a dynamic interpreter, but it is exactly the shape a static type system exists to
replace: over a proper AST sum type, the same dispatch is a single exhaustive `match` whose arms
bind the payload directly (`(match node ((Ast.int n) …) ((Ast.list es) …))`). The operator directed
that we write Cadenza *as if the type system were already working* — treat it as a static language —
so that the compiler's source is forward-compatible with the generation that will actually
type-check it. The seed stays dynamic (the §VII carve-out is untouched); what changes is the
authoring discipline for every line of Cadenza we write.

**Why.** Core Principle VII line 82 requires that the seed's deferral of static typing "be realized
by a generation derived after the seed, so that the deferral is a bootstrap stage rather than a
permanent downgrade." A deferral is only a *stage* if it is reversible cheaply. If the Cadenza
source is authored in dynamic idioms — runtime kind-reflection, values whose type varies by path,
constructs a checker would reject — then "realizing static typing later" means rewriting all of that
source, and the pressure at that point is to weaken the type system to accept the existing dynamic
code instead. Authoring the source as well-typed from the start makes the transition a check that
passes rather than a rewrite, and keeps the self-hosting climb monotonic: each construct the
compiler learns to emit is one a static Cadenza would also have. The specification recorded that the
seed *may* defer type-checking, but said nothing about how the Cadenza source it runs must be
*written*; that silence let the source accrete dynamic assumptions that would later have to be
unwound. This also converges with the queued AST-as-sum-type-plus-`match` work: writing the compiler
statically and retiring the `Ast.is-*` reflection are the same change.

**The requirement it drove.** Added to `spec/bootstrap.md` §"The Compiler Is Authored In Cadenza,
Not In The Seed" two requirements: the Cadenza source the bootstrap authors MUST be written as a
well-typed static program, as if Core Principle VII were already enforced, so the source is accepted
unchanged by a later type-checking generation rather than rewritten; and it MUST NOT rely on a
dynamic idiom a static type discipline would reject — such as runtime kind-reflection in place of a
sum type and a match — so that deferring type-checking in the seed is a stage the language climbs
out of rather than an assumption its source is built on. Complements the constitution's §VII
carve-out (which permits the seed to defer *enforcing* types) by constraining how the source is
*authored* so the deferral stays a stage.
