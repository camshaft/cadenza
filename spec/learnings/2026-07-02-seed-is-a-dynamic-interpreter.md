# The seed is a dynamic interpreter; static typing is a follow-up generation

*2026-07-02*

**What happened.** While taking the specification to a working compiler in attended mode, the driver
set a guiding principle: get to a working interpreter as fast as possible, bootstrap the compiler, and
let it add features to itself, rather than building every language feature before the flywheel turns.
Applied to the type system, this means the operator-synthesized **seed** generation is a **dynamic
tree-walking interpreter**: it evaluates the core, enforces the mandatory capability floor, derives a
component by embedding itself over a program's AST, runs it, and serves as the behavioral oracle — but
it does **not** perform static type-checking. Static typing is realized by the first generation derived
*after* the seed. Several checks that a typed compiler makes at compile time (no implicit numeric
promotion, match exhaustiveness, nominal/structural distinction, unbound-name resolution) are therefore
not compile-time rejections in the seed; where they have a defined dynamic outcome the seed traps, and
where they require the type system the seed does not realize them at all until the typed generation.

**Why.** The value of this architecture is the flywheel — a minimal seed that derives the next
generation, each authored in Cadenza (overview §15; self-hosting-and-bootstrap.md). A feature-complete
seed delays the one thing that matters, a turning flywheel, and contradicts the interpreted-first
bootstrap strategy (`options/bootstrap-strategy/`). Static typing is Cadenza's headline guarantee, but
it is not what makes a *derived component* safe to run — determinism, capability-binding,
bounded-termination, and reproducibility are, and those the seed's output keeps (bootstrap.md §"The
First Toolchain Is Operator-Synthesized" already lists exactly those four and omits static typing).
Bootstrapping a strongly-typed language's first compiler in an untyped host and then self-hosting the
type checker is a well-trodden path; it fits interpreted-first derivation exactly.

**The requirement it drove.** A bootstrap carve-out to [Core Principle VII](../../constitution.md): the
seed generation MAY defer the static-typing obligations and realize evaluation dynamically; a
generation that defers MUST record it; and the obligations MUST be realized by a generation derived
after the seed, so the deferral is a bootstrap stage, not a permanent downgrade. This is a
constitutional amendment recorded here per the Amendment Discipline. It is operationalized by the
`realized-capability-set` decision (`options/realized-capability-set/seed-ignition-set.md`) and
conformance-gate.md §"A Generation Is Judged Against The Capabilities It Realizes", which scope a
generation's behavioral-witnessing obligation to the capabilities it realizes; by dropping
`type-system.md` from the ignition requirement subset (`.duvet/bootstrap.toml`); and by annotating the
corpus **inline** — the primary result clause of each case is the interpreter (the oracle), a
`(compiler (error …))` annotation records where a typed generation rejects a program the dynamic seed
still runs, and `(needs <capability>)` gates which generation runs a case (one flat corpus, no
`type-system`/`interpreter`/`compiler` directory split — the divergence is local and legible rather
than a second place behavior lives). The determinism, capability-safety, and reproducibility floors are
NOT downgraded — those remain mandatory for the seed and are governed by the never-downgradable
Governance Floor.
