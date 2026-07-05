# Learnings

Dated post-mortems that drove this specification. Each entry follows the format in
[`templates/learning.md`](../../templates/learning.md): **What happened / Why / The requirement it
drove**. Learnings are descriptive, not normative — they carry no RFC-2119 requirements and are not
listed in the requirement gate. They are the one place a specification artifact may name a prior
prototype or a concrete implementation, because a learning is historical reference for why a durable
change exists.

The learnings here are the reasons this clean-room specification is shaped as it is. Earlier
generations of Cadenza taught these lessons the expensive way; the specification is the response.

- [The compiler core was restarted four times](./2026-07-02-compiler-core-restarted-four-times.md) —
  why the specification, not the compiler, is the durable artifact.
- [Component output never materialized](./2026-07-02-component-output-never-materialized.md) — why the
  component ABI and determinism are frozen contracts written before the capabilities.
- [Four parallel semantics drifted](./2026-07-02-parallel-semantics-drifted.md) — why there is one
  executable semantics, gated by execution.
- [Multiple front-ends diluted one surface](./2026-07-02-multiple-frontends-diluted-one-surface.md) —
  why there is one canonical representation with decoupled displays.
- [Verification was baked through the tree](./2026-07-02-verification-baked-through-the-tree.md) — why
  verification is progressive and meaning-preserving.
- [There was no line of sight to self-hosting](./2026-07-02-no-line-of-sight-to-self-hosting.md) — why
  the reference interpreter is the oracle and the seam to the flywheel.
- [A modeled subsystem passes a shape check](./2026-07-02-a-modeled-subsystem-passes-a-shape-check.md)
  — why behavior requirements are discharged by execution and every requirement binds to an enforcing
  line. (Adopted from the host project's own hard-won lesson.)
- [The seed is a dynamic interpreter](./2026-07-02-seed-is-a-dynamic-interpreter.md) — why the seed
  generation defers static typing and realizes evaluation dynamically to get the flywheel turning, and
  the Core Principle VII bootstrap carve-out that records the amendment.
- [The ignition path is de-risked](./2026-07-02-ignition-path-de-risked.md) — the two Phase-2 spikes:
  duvet's quoted-sentence gate works for Rust (but exits 0 on citation errors), and the
  source→derive→run→re-derive path is real and byte-reproducible in this environment.
- [Decouple the interpreter-wasm from the host](./2026-07-02-decouple-interpreter-wasm-from-host.md) —
  interpreted derivation embeds the interpreter *component* over the program's AST (so the component
  actually interprets, not replays a transcript); the host providing capability functions is a
  separate minimal artifact. Avoids the modeled-derivation trap.
- [Bootstrap is interpreter-first, not compiler-first](./2026-07-02-interpreter-first-not-compiler-first.md)
  — why a compiler-first self-hosting proposal was considered and rejected (it has no behavioral
  oracle and revives the meaning-in-the-compiler failure), while its compatible ideas were adopted;
  switching would be a deliberate constitution IX/XIV amendment.
- [An effect-only program had no normal-termination value](./2026-07-02-effect-only-programs-need-a-unit-value.md)
  — why a Unit value was pinned (additively) so event-emitting programs carry a definite terminal
  condition; surfaced by four corpus cases that had only an `(events …)` observation and no primary
  result clause.
- [Real components, not a bespoke module model](./2026-07-03-real-components-not-a-bespoke-module-model.md)
  — why the bootstrap uses real WebAssembly components (`wit-bindgen` core module → `wasm-tools
  component new`) rather than a hand-managed `wasm32-unknown-unknown` core module with an AST slot and
  trimmed imports; the WIT world makes "imports mirror the manifest" hold natively, which reverted the
  short-lived 0.3.0 import amendment. Includes the offline de-risk-spike findings.
- [The seed needs first-class functions](./2026-07-03-the-seed-needs-first-class-functions.md) — why the
  seed realizes functions and closures (core-semantics.md §Functions): the first Cadenza artifact is a
  compiler, which is not expressible without them.
- [Bootstrap targets the compiler directly](./2026-07-03-bootstrap-targets-the-compiler-directly.md) —
  why the staged path collapsed to seed interpreter → Cadenza compiler → self-hosting, dropping the
  re-author-the-interpreter-in-Cadenza rung; the reference interpreter stays the oracle, so IX/XIV hold.
- [The seed realizes a byte-sequence form so the Cadenza compiler emits component bytes](./2026-07-03-seed-realizes-bytes-so-the-compiler-emits-components.md)
  — why the codegen is authored in Cadenza (not the seed) and the seed realizes a `Bytes` value form:
  an attended halt when the build was about to write the codegen in Rust, and the seed↔compiler seam it
  hardened (bootstrap.md §"The Compiler Is Authored In Cadenza, Not In The Seed").
- [The Cadenza compiler emits the whole component](./2026-07-03-the-compiler-emits-the-whole-component.md)
  — why the compiler emits the complete component binary as a value rather than a core module a tool
  completes, so a derivation's bytes are a function of the Cadenza compiler alone and self-hosting is a
  clean fixpoint (no external wrapping tool in the byte path).
- [One accessor, everything is a record](./2026-07-03-one-accessor-modules-are-records.md) — why `.` is
  the sole record accessor (`a.b` is sugar for `(. a b)`), why modules/records/prelude namespaces are
  all records while maps stay dynamic, and the `(meta …)` metadata channel; killed a lowercase/uppercase
  dotted-atom heuristic that re-parsed meaning from an atom's spelling.
- [Exhaustion is a trap across the compiled seam](./2026-07-03-exhaustion-is-a-trap-across-the-compiled-seam.md)
  — why a derived component that exhausts the resource measure halts as a trap and is judged as agreeing
  with the interpreter's `exhausted` terminal condition; surfaced by the differential gate before growing
  the compiler to recursion, so the two recursion cases don't flip to a false `disagree`.
- [Decline, do not miscompile](./2026-07-03-decline-do-not-miscompile.md) — why a compiler grown
  incrementally MUST trap/decline a construct it cannot yet compile rather than emit divergent bytes or
  silently skip it, keeping "cannot yet" and "does wrong" observably distinct so a green differential gate
  means every compiled program agrees.
- [The corpus is a differential gate](./2026-07-03-the-corpus-is-a-differential-gate.md) — why the
  generated path is exercised against the oracle over every corpus case the compiler compiles, turning the
  executable-semantics corpus into a live regression surface as the compiler grows (agree/todo/skip/disagree).
- [The assembler lives in Cadenza](./2026-07-03-the-assembler-lives-in-cadenza.md) — why even the
  instruction-to-bytes assembly step is authored in Cadenza (a WAT-like structured layer folded to bytes),
  not delegated to a host `wat`-crate pass, so no part of the translation escapes the Cadenza compiler.
- [The compile seam is statically typed](./2026-07-03-the-compile-seam-is-statically-typed.md) — why the
  seed invokes the Cadenza compiler through a byte-to-byte interface (`compile : list<u8> -> list<u8>`)
  rather than through its dynamic value type, so no dynamic-language assumption is baked into the
  compiler's contract and a later generation can type-check the same seam; surfaced when the self-hosting
  harness needed the interpreted and compiled compilers to share one static type to be comparable.
- [Author Cadenza as static even though the seed is dynamic](./2026-07-03-author-cadenza-as-static-even-though-the-seed-is-dynamic.md)
  — why every line of Cadenza source is written as a well-typed static program (sum types + `match`, not
  runtime `Ast.is-*` kind-reflection) even though the seed defers type-checking, so the source is accepted
  unchanged by the later type-checking generation rather than rewritten and the §VII deferral stays a stage.
- [Uniform single-arity constructors eliminate cascading special cases](./2026-07-03-uniform-single-arity-constructors.md)
  — why all sum type constructors are single-arity functions (including "nullary" variants that take Unit),
  rather than nullary-as-pre-applied-Sums vs unary-as-Constructors, eliminating arity-based special cases in
  pattern matching, type synthesis, and compilation; the dual representation compounded (each feature checked
  "which kind?"), and adding unit broke all tests when one check was missed.
- [Types first-class in the dynamic seed sets up static self-hosting](./2026-07-03-types-first-class-in-dynamic-seed.md)
  — why the seed makes types first-class values even though it's dynamically checked (§VII defers checking,
  not types themselves), and why the AST is quotable as a sum type: compiling dynamically-written code to
  static is incredibly hard, but runtime-checked types written with type annotations transition smoothly to
  compile-time checking (move validation earlier, not infer what wasn't written); quote/unquote lets the
  compiler operate on AST values natively rather than string-tagged reflection.
- [Quasiquote for programmatic AST construction](./2026-07-03-quasiquote-for-programmatic-ast-construction.md)
  — why quasiquote with selective evaluation (`,` unquote, `,@` splice) is necessary once the compiler
  operates on AST values: `quote` is uniform (never evaluates), but instruction construction needs to embed
  computed values; without quasiquote, building `(+ x 10)` where `x` varies means verbose
  `(Ast.List (list ...))` calls; `` `(+ ,x 10)`` reads like the instruction and makes the compiler maintainable.
- [AST construction vs AST evaluation: the compiler needs construction only](./2026-07-03-ast-construction-vs-ast-evaluation.md)
  — why the compiler needs quasiquote (AST construction) but not `eval` (AST execution): inside quasiquote,
  `,expr` evaluates `expr` normally to embed its value (statically checkable); top-level `(eval ast-value)`
  executes AST as code (meta-interpretation, needs embedded interpreter, hard to do statically). The compiler
  constructs and analyzes AST but never executes dynamically-constructed AST. Eval is optional for macros/REPL.
- [Two compilers, not an interpreter and a compiler; the runtime is wasm](./2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md)
  — why the seed stops being a reference *interpreter* and becomes a reference *compiler* (`cdz-rustc`): the
  runtime is wasm, an interpreter and a compiler share almost nothing, and codegen was being grown blind. The
  oracle becomes the conformance corpus, and independence comes from two implementations of the compiler that
  must agree — in place of an interpreter-vs-compiler differential (Constitution Amendment 0.3.0).
- [Static typing is mandatory once the seed is a compiler](./2026-07-04-static-typing-is-mandatory-post-pivot.md)
  — why Constitution Amendment 0.4.0 retires the Principle VII dynamic-seed carve-out: the carve-out was
  conditioned on realizing evaluation dynamically, which the two-compiler pivot removed, so the seed compiler
  must reject ill-typed programs with a machine-readable code (incrementally, reject-don't-miscompile) rather
  than defer typing; the corpus `(compiler …)` clauses become the seed's own rejections.
- [Nominal is an orthogonal tag over any structural type](./2026-07-04-nominal-is-orthogonal-tag-over-structural-types.md)
  — why nominal-versus-structural is one orthogonal axis over every structural type (record, tuple, sum), a
  nominal value being its structural value plus a compile-time, fully-qualified name tag that adds nothing to
  the runtime representation; nominal types are not comparable across their boundary, and identity is the
  module path plus declared name.
- [Generics are type-valued parameters, not a separate polymorphism mechanism](./2026-07-04-generics-are-type-valued-parameters.md)
  — why generics fall out of first-class types plus compile-time evaluation: a generic is an ordinary
  definition taking type-valued parameters, a type constructor is a compile-time type→type function, and
  monomorphization is the existing compile-time reduction — no separate polymorphism or trait-resolution engine.
- [The host is value-agnostic; the compiler owns the reader and printer](./2026-07-04-host-is-value-agnostic-compiler-owns-reader-printer.md)
  — why a compiled program's result crosses the boundary as its proper component type, exported as a resource
  owning a `display` method, rather than teaching the host Cadenza's value shapes or collapsing the boundary to
  a string; the reader/printer are compiler-exposed text↔binary surfaces so the host stays value-agnostic.
- [Type inference is Hindley-Milner](./2026-07-04-inference-is-hindley-milner.md)
  — why inference is unification over type variables yielding principal types with let-generalization, not
  ad-hoc guessing from a single call site; a parameter's type is the solution derived from all its uses at
  once, contradictory constraints are a compile-time rejection, and let-generalization is the same mechanism as
  generics being type-valued parameters.
- [An immutable heap is acyclic, so reference counting is complete](./2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete.md)
  — why immutability + strict evaluation forbid heap cycles, which makes reference counting sound AND complete
  (no tracing GC, no cycle collector); the allocator is emitted into the component so the host provides only
  linear memory; Perceus-style in-place reuse makes persistence free when unshared. Drives
  memory-and-resource-model.md and the new `options/memory-ownership-model/`.
- [Effects are algebraic; a capability is a boundary effect; mutation is a State effect](./2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects.md)
  — why the effect open question resolves to algebraic handlers unified with capabilities (the manifest is the
  effect row that escapes to the host), mutation re-enters as a pure-state-passing `State` effect, and
  continuations are one-shot (affine) to keep fuel accounting and RC sound. Drives capabilities-and-effects.md
  and `options/effects-model/`.
- [Records are rows: row polymorphism does triple duty](./2026-07-04-records-are-rows-open-by-default.md)
  — why records gain row polymorphism (open over fields), which also types effect rows and preserves principal-
  type inference; subset comparison is explicit projection-then-`=`, never an overloaded `=`; row variables are
  monomorphized to closed shapes before the boundary. Drives type-system.md §The Declarable Type Universe.
- [Ad-hoc polymorphism: traits are dictionaries, scoped, not coherent](./2026-07-04-traits-are-dictionaries-scoped-not-coherent.md)
  — why a trait is a dictionary record type and an instance an ordinary value (Scala-`given`/OCaml-implicits/
  F#-SRTP shape), resolved by deterministic source-ordered scoped search and monomorphized away — NOT Haskell
  global coherence or orphan rules, which fight content-addressed modules. Drives type-system.md and
  `options/ad-hoc-polymorphism/`.
- [The refinement layer is liquid types; verification is extrinsic](./2026-07-04-refinements-are-liquid-verification-is-extrinsic.md)
  — why refinements are liquid (decidable predicate logic, SMT-discharged into a checkable certificate) and
  machine-checked verification is extrinsic (about behavior, not propositions-as-types), which is what keeps
  `Type : Type` sound; discharge must be proof-producing. Drives verification-layers.md, type-system.md, and
  `options/verification-strategy/`.
- [Linearity is surgical, not core; graded types are the aim](./2026-07-04-linearity-is-surgical-not-core.md)
  — why linear/affine types are NOT mandatory core (immutability + RC already cover memory) but ARE used
  surgically (one-shot continuations, linear capability handles, an optional usage layer); graded/quantitative
  types with an erased `0` multiplicity are the course to aim at. Course-setting; drives annotations across
  memory/effects/verification specs.
- [HM inference and first-class types meet at a bidirectional boundary](./2026-07-04-inference-meets-first-class-types-at-a-bidirectional-boundary.md)
  — why principal-type HM inference and computable first-class types are reconciled: HM over a non-computational
  term core, with a bidirectional-checking boundary at type-valued-parameter positions (synthesized by
  monomorphization or checked against an annotation), closing a literal contradiction in type-system.md §Inference.
- [Compile-time evaluation is one tier](./2026-07-04-compile-time-evaluation-is-one-tier.md)
  — why macros, generics, monomorphization, and const-folding are the SAME pure, bounded, deterministic
  compile-time evaluation (one mechanism, not four subsystems that drift); a macro is an ordinary phase-1 Cadenza
  function over Ast, and the tier runs in the empty effect row so purity is a consequence of the effect model.
- [Macros are typed (Expr[T]) and hygienic (sets-of-scopes)](./2026-07-04-macros-are-typed-and-hygienic.md)
  — why the static spine forces typed quotes (Expr[T] over the untyped Ast analysis substrate, so ill-typed
  macro output is rejected at the macro, not downstream) and why hygiene is realized by Racket's set-of-scopes
  model; drives an ADDITIVE ast-encoding.md extension (identifiers carry scope sets), operator-approved to enact.
- [Macro phases; the reader stays fixed](./2026-07-04-macro-phases-and-the-reader-stays-fixed.md)
  — why macros are dispatched by binding (not a call-site heuristic), a minimal two-phase (runtime/compile-time)
  model with expand-to-fixpoint before type-checking, and the deliberate exclusion of reader macros (syntax grows
  at the Ast level, keeping the reader out of the trusted path) — a principled contrast with the LISP inspiration.
- [A rejection carries a verified route to a compliant program](./2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program.md)
  — why a diagnostic must carry not just a reason but a machine-applicable fix (a structural AST edit), verified
  by apply-and-recompile where the repair is determinable (capability delta, match arms, conversions) and marked
  with an applicability marker where it is a guess — stronger than Rust's suggestions. Drives Constitution XI
  Amendment 0.5.0.
- [Diagnosis is complete and cascade-aware](./2026-07-04-diagnosis-is-complete-and-cascade-aware.md)
  — why the compiler must recover and report the maximal set of independent problems in one pass (not first-error),
  mark primary vs. derived so an agent fixes root causes, and expose a machine-branchable rejection/decline/trap
  kind so the agent routes around compiler limits instead of chasing fixes for them. Drives diagnostics.md + XI.
- [Type errors report the minimal conflict, both sites](./2026-07-04-type-errors-report-the-minimal-conflict.md)
  — why an HM type rejection must report the minimal unsatisfiable constraint set naming BOTH disagreeing
  locations (type-error slicing), not one blamed site — the bidirectional boundary decides the phrasing; showing
  both ends of the contradiction IS the fix. Drives type-system.md §Inference reporting discipline.
- [Program transformation is a program](./2026-07-04-program-transformation-is-a-program.md)
  — why refactoring is a Cadenza component over the AST (the same rep→rep seam as `compile`), the structural
  edit ops are a library of `Ast` functions, and text patching is never the mechanism; the tools that modify
  programs are themselves gated programs, generalizing the flywheel. Drives agent-authoring.md + structural-interface.
- [The compiler is a queryable oracle](./2026-07-04-the-compiler-is-a-queryable-oracle.md)
  — why an agent queries the compiler for any static fact (type of any node, name resolution, inferred manifest/
  effects, solved constraints) — total, deterministic, agreeing with a full compile — instead of instrumenting
  the program to learn it; generalizes the machine-readable-output + tooling-is-one-compiler reqs. Drives tooling-and-lsp.md.
- [Deterministic replay is the debugger](./2026-07-04-deterministic-replay-is-the-debugger.md)
  — why determinism (adopted for safety) buys lossless replay and fuel-indexed time-travel debugging for free
  (record only inputs + capability responses), so the agent observes runtime facts by replay not by inserting
  prints — and why the debug view is a tool-time projection NOT part of observable behavior. Drives tooling-and-lsp.md.
- [Capabilities attenuate: a handler forwards a narrower row](./2026-07-04-capabilities-attenuate-a-handler-forwards-a-narrower-row.md)
  — why a handler may grant a sub-computation FEWER capabilities than it holds (never more): object-capability
  attenuation realized as the effect-row-subset relationship handlers already track, making "no ambient authority"
  transitive; required by the target's cross-participant/tool-invocation model. Drives capabilities-and-effects.md.
- [The host interface IS the effect vocabulary](./2026-07-04-the-host-interface-is-the-effect-vocabulary.md)
  — why the abstract effect/capability labels are anchored to the four concrete frozen host operations
  (read-projection, emit-event, read-blob, invoke-tool): the manifest is the escaping effect row over that
  vocabulary, purity is the empty row, and the operation set is pinned once in options/execution-model/. Target-anchored.
- [Cadenza and its target share one seam](./2026-07-04-cadenza-and-the-target-share-one-seam.md)
  — why Cadenza is the source language + derivation tool for a specific target system (behavior-is-data over an
  event log), the derivation/host-interface/manifest touchpoints already correspond, and both must be ONE shared
  definition (not two that drift) with consistent governance floors across the seam. Drives the two frozen contracts + traceability.
- [Durable execution is effects + determinism](./2026-07-04-durable-execution-is-effects-plus-determinism.md)
  — why the target's suspend-record-resume-anywhere agent step (Temporal-style durable execution) falls out of
  algebraic effects (a boundary effect is a suspension point) + determinism (replay from recorded effect responses)
  + one-shot continuations (resume exactly once); demands a durable continuation capture only canonical-form data + manifest caps.
- [A fold module is provably pure; role bounds the effect row](./2026-07-04-fold-modules-are-provably-pure.md)
  — why a module's role fixes its mandatory effect profile (fold = empty row / pure; agent-step quarantines
  nondeterminism into a recorded reasoning tool call), and the compiler must REJECT a fold that reaches a forbidden
  effect AND emit a machine-readable purity certificate the activation review trusts. Target-anchored; drives capabilities-and-effects.md.
- [Fold order-independence is the verification layers' killer app](./2026-07-04-fold-order-independence-is-a-verified-property.md)
  — why the target's byte-identical-regardless-of-delivery-order fold rule (a CRDT-style commutative/latest-wins
  convergence property, stronger than purity) is the first load-bearing use of the optional verification layers:
  discharged by property testing (permutation invariance) / liquid refinement / proof, off the byte path. 
- [Open vocabulary needs open sums + schema-typed payloads](./2026-07-04-open-vocabulary-needs-open-sums-and-schema-typed-payloads.md)
  — why the target's open event-kind space (a fold is inert to unknown kinds) makes OPEN sum types (polymorphic
  variants, the sum dual the rows learning deferred) REQUIRED — exhaustiveness via a mandatory open-tail arm — and
  makes payloads schema-typed (bytes decoded against a run-time-resolved schema → typed Result). Ast stays a closed sum.
- [Host functions are un-named; the language binds any WIT-typed function](./2026-07-05-host-functions-are-un-named-the-language-binds-any-wit-function.md)
  — why the four concrete host ops (read-projection/emit-event/read-blob/invoke-tool) are a target leak removed from
  the language: the sole requirement is binding to WIT-typed host functions (complete signature), the vocabulary is the
  target's, the manifest is the escaping row, purity is the empty row, and the compiler imports nothing. host-interface-binding v2.
- [A host call suspends and resumes by replay from the host's log](./2026-07-05-host-calls-suspend-as-replay-from-the-hosts-log.md)
  — why every host call is a mandatory suspension point resumed by Temporal-style replay: the program holds no resume
  state, the host owns the response log, the continuation is (component + input + log) canonical data resumable on any
  federated host, and resumption strategy (in-process / live / teardown) is the host's determinism-guaranteed choice. component-abi v2.
- [The seed stays Rust, not Lean](./2026-07-05-the-seed-stays-rust-not-lean.md)
  — why the seed's implementation language is orthogonal to Cadenza's verification aims (the seed is disposable, off the
  critical path, and independence comes from two compilers agreeing against the corpus, not a trusted verified seed);
  Rust's wasm/bytes/component ecosystem wins on fit; Lean is admissible only as an optional third oracle. Confirms the default.
- [Bool offers a total order, with false less than true](./2026-07-05-bool-offers-a-total-order.md)
  — why the conditional "ordering where offered is total" invariant needed a ground clause fixing which primitive types
  offer an order; an adversarial corpus case `(< true false)` had no definite outcome because Bool's ordering was never
  stated. Drove a sentence in core-semantics.md §"Ordering Where Offered Is Total" (Bool is totally ordered, false < true),
  witnessed by cases in the equality-and-observation corpus.
- [The value-heap runtime is a shared component](./2026-07-05-the-value-heap-runtime-is-a-shared-component.md)
  — why a program's runtime values (tuples, records, sums, …) do not live in each program's own component but in a single
  shared value-heap runtime the program imports and the host composes: the heap/reference-counting machinery is growing
  code better authored once and linked than open-coded per compound type, and because the runtime owns the storage behind
  an opaque handle its representation can evolve (Perceus RC, CHAMP/RRB) with no change to emitted programs. Drove
  component-abi.md v3 §"The Value-Heap Runtime" and the pin-by-content-address / build-pair rules.
- [The runtime is name-free; rendering is type-directed](./2026-07-05-the-runtime-is-name-free-rendering-is-type-directed.md)
  — why `render` was removed from the runtime: at run time a record is a positional product and a sum an integer tag, so
  the runtime holds no field or variant names and cannot render; rendering is type-directed code the compiler emits into
  the program, which walks the value through the runtime's accessors and returns an ordinary string. Refined
  component-abi.md v3 (§"The Runtime Does Not Name Or Render Values", §"A Compound Result Is Rendered By Compiler-Emitted
  Code").
- [Emitting a component that imports is a fixed envelope around a variable core module](./2026-07-05-emitting-a-component-with-an-import-is-a-fixed-envelope.md)
  — the engineering technique for self-contained component emission with an import: bake a `wasm-tools`-validated
  reference as fixed HEAD/TAIL byte constants around a compiler-built core module (no compile-time tooling), and shift
  every defined-function index by a fixed base because imports occupy the low index space. Realizes
  reproducible-derivation.md §"Derivation Is A Function Of Source And Toolchain" and component-abi.md §"The Value-Heap
  Runtime Crosses By A Well-Known Import" in the emitter.

## Open spec gaps (found by adversarial-corpus probing; awaiting a clarity pass)

These entries record behavior the specification has **not yet fixed** — an adversarial corpus run reached
a construct with two or more defensible, observably-distinct outcomes and no requirement selecting between
them, so the corpus records no oracle for it. Unlike the resolved learnings above, each of these defers its
requirement edit to a follow-up clarity agent: the entry names the gap, the candidate readings, and the
recommended resolution, but does not itself change a requirement. Resolving one means adding the RFC-2119
sentence the entry describes and the witnessing corpus case, then moving it into the resolved list above.

- [Spec gap: `let` binding sequencing is unspecified](./2026-07-05-spec-gap-let-binding-sequencing.md)
  — whether multiple bindings in one `let` are sequential (`let*`, each initializer sees the earlier names) or
  parallel (each evaluated in the enclosing scope) is undetermined; `(let ((x 1) (y (+ x 1))) y)` is 2 under one
  reading and an unbound-name rejection under the other. Recommended: sequential (matches the seed and the
  functional-language default).
- [Spec gap: duplicate pattern binder](./2026-07-05-spec-gap-duplicate-pattern-binder.md)
  — whether a pattern may bind the same name twice (`(tuple x x)`), and if so whether it shadows, errors, or
  imposes an equality constraint, is unspecified. Recommended: a repeated binder is a compile-time error
  (linear patterns).
- [Spec gap: String/Bytes indexing lacks a total-or-trap requirement](./2026-07-05-spec-gap-string-bytes-total-or-trap.md)
  — only *list* indexing has a dedicated total-or-trap MUST; String and Bytes out-of-bounds reads rely on the
  weaker general partial-operations clause, which permits a trap *or* a defined value, so the corpus's recorded
  traps are one permitted choice rather than required behavior. Recommended: add a total-or-trap requirement
  covering String and Bytes indexing (or generalize the list one).
- [The behavior gate is not byte-exact for floats](./2026-07-05-behavior-gate-not-byte-exact-for-floats.md)
  — the whole-float renderer uses `f as i64`, which *saturates*, so distinct floats ≥ 2^63 (1e19, 1e20, 1e100)
  collapse to one canonical form — violating deterministic-value-form injectivity. The gate can't catch it: it
  renders both sides through the same `display_float`, so it is not byte-exact for floats (contradicting the
  corpus README's "byte-exact" claim), and the existing anti-saturation case is a false guard. Needs BOTH a
  renderer fix (`{:.0}` not `f as i64`) AND a gate that compares float output byte-exact against the recorded
  literal text. No corpus case can express this until the gate is fixed.
