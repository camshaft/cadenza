# Bootstrap is interpreter-first, not compiler-first

*2026-07-02*

> **Annotation (2026-07-03) — staging collapsed; this learning still holds, with one refinement.**
> The staging was later shortened: the intermediate rung "re-author the reference interpreter *in
> Cadenza*" was **dropped**, and the **first Cadenza artifact is now the compiler**, derived directly
> by running the foreign-language (Rust) seed reference interpreter over the compiler's source
> ([bootstrap.md](../bootstrap.md) §"The Line Of Sight";
> [self-hosting-and-bootstrap.md](../capabilities/self-hosting-and-bootstrap.md) §"Each Generation Is
> Derived By The Previous"). This does **not** revive compiler-first, because the distinction this
> learning draws is preserved: the **reference interpreter remains the single behavioral oracle** (it
> stays authored in the foreign seed language and defines behavior; the compiler must *agree* with it),
> so Core Principles IX and XIV are intact. What changed is only *which Cadenza artifact is authored
> first* — the compiler, not a redundant Cadenza re-implementation of the interpreter — which removes a
> rung with no consumer, exactly the "no throwaway meta-circular interpreter" point this learning
> already endorsed. The sentence below asserting "the first Cadenza-authored artifact is the reference
> interpreter" is superseded by "…is the compiler". **Two other points below are also superseded:** the
> body says ahead-of-time compilation "stays a later, oracle-checked optimization … never the
> bootstrap's critical path" and cites bootstrap.md §"Compiled Derivation Is An Oracle-Checked
> Optimization" — but the seed's derivation mode is now **compiled codegen** (that section was renamed
> to §"Compiled Derivation Produces The Component And Agrees With The Oracle"), so component generation
> *is* the seed's path, not a deferred optimization; and "interpreted derivation … is the first working
> derivation mode" is superseded by interpreted derivation being **optional/later** (bootstrap.md
> §"Interpreted Derivation Is An Optional Mode"). What still stands is this learning's load-bearing
> claim: the **reference interpreter remains the single behavioral oracle**, so this is not
> compiler-first. See [bootstrap targets the compiler directly](./2026-07-03-bootstrap-targets-the-compiler-directly.md)
> and [real components, not a bespoke module model](./2026-07-03-real-components-not-a-bespoke-module-model.md).
>
> **Annotation (2026-07-04) — the load-bearing claim above is now itself SUPERSEDED.** The two-compiler
> pivot ([2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md](./2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md);
> Constitution Amendment 0.3.0) drops the *seed* interpreter too: the seed is now a reference
> *compiler*, the **behavioral oracle is the conformance corpus** (not a reference interpreter), and the
> judgment's independence comes from **two implementations of the compiler** that must agree. The
> bootstrap is therefore now **compiler-first** in shape (two compilers, no required interpreter), while
> the deeper lesson this learning taught — never let meaning live in the compiler alone; keep an
> independent behavioral authority — is *upheld*, now discharged by the corpus + two-compiler
> differential rather than by an interpreter. The 2026-07-03 annotation's renamed-section citations are
> also stale: §"Compiled Derivation Produces The Component And Agrees With The Oracle" is now
> §"Compiled Output Agrees With The Recorded Semantics", and §"Interpreted Derivation Is An Optional
> Mode" is now §"A Reference Interpreter Is An Optional Independent Oracle". Retained as historical
> reference; do not cite the interpreter-as-oracle claim as current architecture.

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
