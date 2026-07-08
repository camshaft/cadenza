# An uncalled top-level definition is not scope- or type-checked

*2026-07-08*

**What happened.** Adversarial probing found that a top-level module definition that `main` never
calls is not scope-checked or type-checked at all. `(module m (def (bad) nonexistent) (def (main)
42))` compiles and runs to 42, even though `bad`'s body references the unbound name `nonexistent`
(an unconditional CDZ0101 error). The same escape covers every well-formedness check: an uncalled
`(def (bad) (+ 1 true))`, `(+ 1 "str")`, `(tuple.5 (tuple 1 2))`, `(if 1 2 3)`, `(: 5 Bool)` all
compile to a running component when `main` doesn't call `bad`. The identical ill-typed body is
correctly rejected the moment `bad` IS called.

**Why it is a break.** core-semantics.md #Binding Is Lexical: "A reference to a name with no
enclosing binding MUST be a compile-time error" — unconditional, with no reachability qualifier.
type-system.md line 12: a program is well-typed when "every expression has a statically determined
type," and line 24: "A program that is not well-typed MUST be rejected." A module's definitions are
its EXPORTS, each reachable by member access (#A Module Evaluates To A Record Of Its Exports; #A
Module's Exported Definition Is Reachable By Member Access), so `(def (bad) …)` is not dead code —
it is an export `(. m bad)` whose body must resolve and type-check. Checking only the functions
transitively called by `main` lets an ill-formed export through.

**The inconsistency that localizes it.** An inner-module sibling in the exact same shape IS checked
today: `(module lib (def (bad) (+ 1 true)) (def (ok) 5))` is rejected even when only `ok` is called.
So the module-body checker does type-check all of an inner module's definitions — but the top-level
module's checker only visits definitions reachable from `main`. The gap is specifically the
top-level module's uncalled definitions. Likely the top level drives compilation from `main` and
its transitive call graph (emitting only what it reaches), while an inner `(module …)` form runs a
whole-body check; the top level needs the same whole-definition-set check the inner module already
performs — scope- and type-check every definition, then emit (or dead-strip) as a separate step.

**Why it matters beyond pedantry.** Under self-hosting the compiler is a large module of many
mutually-referencing definitions; an unbound name or type error in a definition that a given entry
doesn't reach would ship silently, only to surface when a later change makes it reachable — the
opposite of "a well-typed program is rejected at compile time rather than compiled to a component
carrying a deferred [error]" (type-system.md line 24). And an export a program reaches by member
access (`(. m bad)`) would compile despite an ill-formed body until the access is added.

**The lesson.** "Reachable from main" is not "part of the program." A module's every definition is an
export and must be well-formed independently of whether `main` calls it — the binding and typing
rules are stated over expressions and definitions, not over the reachable subgraph. A compiler that
fuses type-checking with code emission (check-what-you-emit, emit-what-you-reach) inherits emission's
reachability pruning into checking, and drops the unreached definitions from both. Checking must be
over all definitions; emission may still prune. The tell was that the same ill-formed definition
flipped from accepted to rejected purely by adding a call to it from `main`.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"an unbound name in an uncalled
sibling definition is still rejected" — `(module m (def (bad) nonexistent) (def (main) 42))` MUST
reject CDZ0101, as the uncalled-definition companion of the direct unbound-name case. Native seed;
the behavior gate catches it (expected reject CDZ0101, observed a running component). The unbound
name is the sharpest witness (an unconditional error, no type-inference subtlety), but the gap
covers every well-formedness check on an uncalled top-level definition.
