# Bootstrap is interpreter-first, not compiler-first

*2026-07-02*

**What happened.** While synthesizing the seed, a reviewing agent proposed replacing the
reference-interpreter bootstrap with a **compiler-first** one: write the ahead-of-time compiler
(`ast → component bytes`) in Cadenza directly, run it on the foreign-language seed interpreter, and
reach self-hosting when that compiler compiles itself (the classic self-hosting fixpoint). Its
supporting arguments were sound in isolation — the component boundary should be `bytes → bytes` (no
recursive values in WIT), the AST is just values, a reader/printer pair gives a cheap `read(print v) ==
v` oracle, and one should not build a throwaway meta-circular interpreter as a rung with no consumer.
Those compatible points were all adopted. The core proposal — drop the reference interpreter as the
first Cadenza artifact and the behavioral oracle — was **considered and rejected**.

**Why.** Compiler-first collides head-on with ratified invariants and with the reasons this whole
reboot exists:

- **Constitution IX** ("Behavior Has One Executable Semantics") and **XIV** ("The Language Has A Line
  Of Sight To Self-Hosting") fix that the language's meaning is one executable semantics *realized as
  the reference interpreter*, which is the behavioral oracle a compiled program must *agree with*.
  Compiler-first has no oracle: the compiler's output would define behavior, and the behavior gate
  would have nothing independent to check against.
- It is the exact failure two prior learnings recorded:
  [parallel semantics drifted](./2026-07-02-parallel-semantics-drifted.md) (meaning must not live in
  the compiler) and [the compiler core was restarted four times](./2026-07-02-compiler-core-restarted-four-times.md)
  (the compiler is disposable, not the artifact of record). "Let the compiler shake out the semantics"
  is precisely the original sin the architecture was rebuilt to prevent.
- It is also unnecessary for the stated goal. Interpreted derivation (embed the interpreter over a
  program's AST) reaches "runs as wasm" **without** writing an ahead-of-time code generator, which is
  less work and matches the "working interpreter ASAP" principle
  ([seed is a dynamic interpreter](./2026-07-02-seed-is-a-dynamic-interpreter.md)). Ahead-of-time
  compilation stays a later, oracle-checked optimization (bootstrap.md §"Compiled Derivation Is An
  Oracle-Checked Optimization"), never the bootstrap's critical path.

Where the reviewer was right and it changed nothing: we do **not** build a throwaway meta-circular
interpreter with no consumer — the Cadenza-authored reference interpreter *is* the artifact (oracle
and, embedded, the derivation mechanism), not a showcase layer; and AOT is already deferred in the
spec. The disagreement reduces to one question — is the first Cadenza-authored artifact the AOT
compiler or the reference interpreter — and this repo answers: the reference interpreter.

**The requirement it drove.** No new requirement — the decision *upholds* the existing ones
(constitution IX/XIV; bootstrap.md interpreted-first; self-hosting-and-bootstrap.md). It is recorded
here so the fork is not silently relitigated: switching to compiler-first would be a deliberate
constitutional amendment to IX and XIV, made with explicit human approval under the Amendment
Discipline, not an implementation choice. The compatible improvements the reviewer offered are realized
in [decouple the interpreter-wasm from the host](./2026-07-02-decouple-interpreter-wasm-from-host.md),
`spec/capabilities/bootstrap-interpreter.md`, and `options/bootstrap-interpreter-surface/`.
