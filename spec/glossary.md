# Cadenza — Glossary

> **What this document is.** The controlled vocabulary for the whole specification. Every
> term used normatively in the constitution, the frozen contracts, the capability
> specifications, and the bootstrap specification is defined here exactly once, so the specs use
> words consistently. This
> document is descriptive, not normative: it carries no RFC-2119 requirements. When a term is
> defined here, other specs use it with this meaning and do not redefine it.

---

## The language and its programs

- **Cadenza** — the programming language this specification defines, and the derivation tool
  that compiles it; designed to be written and read by agents, read by humans, verified for
  properties, and compiled to sandboxed components.
- **Program** — a set of Cadenza source modules that compile together into one component.
- **Source** — the authoritative textual or structural form of a program; the sole authority
  over the program's meaning, from which the runnable form is derived.
- **Module** — a named unit of source that declares definitions and its required capabilities;
  the unit of namespacing and import.
- **Definition** — a named binding introduced by a module: a value, function, type, or
  capability declaration.
- **Documentation** — prose attached to a definition, carried in the canonical representation rather
  than as discarded lexical trivia; preserved through round-trip and structural edits, exposed
  machine-readably, and never affecting a program's runtime meaning.
- **Comment** — a human annotation attached to the part of a program it describes, parsed into the
  canonical representation as a node rather than discarded as lexical trivia, so that it is stored in
  the binary AST and preserved through round-trip and structural edits; semantically inert, affecting
  neither a program's runtime meaning nor its types. Distinct from **documentation**, which is prose
  attached specifically to a definition and exposed machine-readably by the compiler.
- **Canonical representation** — the homoiconic, typed, code-as-data abstract syntax tree that is a
  program's durable form: the sole target of structural manipulation, hashing, the executable
  semantics, and verification. Every textual syntax is a projection of it.
- **Binary AST** — the stable binary serialization of the canonical representation; the canonical
  stored form of a program, and the form a program is hashed as and handed to the compiler as. Fixed
  by the ast-encoding contract.
- **Symbol prelude** — the list, carried by a binary AST, of the symbols its nodes reference; it makes
  a file self-contained, since a node names its kind by referencing a prelude symbol by index rather
  than by an external registry.
- **Symbol** — a namespaced, optionally versioned name a node references to say what kind of node it
  is; language-defined symbols live in the core namespace, and a macro introduces symbols in its own
  namespace so the two cannot collide.
- **Namespace** — the qualifier that scopes a symbol or an imported name, so that a name defined by
  the language, by a module, or by a macro is distinct from an identical name elsewhere.
- **Textual syntax** — a parser that converts text to the binary AST and a printer that converts the
  binary AST to text; more than one may exist, none is the stored form, and moving between them never
  changes the program.
- **Reader** — the parser half of a textual syntax: a function that converts the text of a program to
  its canonical representation; kept out of the compiler's trusted derivation path.
- **Printer** — the inverse of the reader: a function that renders a program's canonical
  representation as text that the reader converts back to the same canonical representation, so that
  reader and printer round-trip.
- **Display** — a textual syntax used for reading or writing; a rendering of the canonical
  representation, not the program's identity.
- **Homoiconicity** — the property that a program is itself a value of the uniform data structure the
  language manipulates, so that textual syntax and stored form decouple and code is data.
- **Structural interface** — the documented interface through which an agent reads and rewrites the
  canonical representation of a program without textual patching and without re-parsing unrelated
  code.
- **Macro** — a compile-time transformation that receives and produces values of the canonical
  representation, transforming a program as data before it is type-checked and compiled.
- **Hygiene** — the property of a macro that a name it introduces neither captures nor is captured by
  a name at its use site unless the macro explicitly requests it.

## Types and values

- **Value** — a datum a program computes with; every value has a type and a canonical byte
  form.
- **Type** — a static classification of expressions that the compiler determines before
  compilation; types are erased from the runnable form.
- **Inference** — the process by which the compiler determines an expression's type without an
  explicit annotation, by unification over type variables that yields the principal (most general)
  type consistent with every use, with let-generalization.
- **Principal type** — the most general type of an expression, the one from which every other valid
  type of that expression is an instance, so that inference commits to no more than the program's
  uses require.
- **Unification** — the solving of the type equalities a program's structure imposes by assigning
  each unknown a type variable and reconciling every constraint on it, so that a type is derived from
  all of a binding's uses at once rather than guessed from one.
- **Annotation** — an explicit type written in source; it constrains inference and is rejected
  on conflict, never silently overridden.
- **Erasure** — the property that no type information the compiler cannot remove survives into
  the runnable form, so the component carries no runtime type reflection.
- **Nominal type** — a structural type tagged with a name, where nominal-versus-structural is an
  orthogonal choice available over any structural type (record, tuple, or sum); its identity is its
  fully-qualified name — the module path in which it is declared together with its declared name —
  distinct from any same-shape type of a different qualified name, and the tag adds nothing to the
  value's runtime representation.
- **Structural type** — a type whose identity is its shape, equal to any type of the same
  shape.
- **Sum type** — a type that is exactly one of several named variants, each optionally carrying
  data; the basis for typed results and error handling.
- **Refinement** — a predicate attached to a type that constrains which of its values are
  admissible; part of the verification layers, not the mandatory core.
- **Structural equality** — the equality under which two values are equal when they share a type and
  their contents are equal component-wise; the equality the canonical byte form agrees with, treating
  a negative zero as distinct from a positive zero and all not-a-number values as equal.
- **Scrutinee** — the value a match examines to select a branch.
- **Aliasing** — the existence of more than one reference to the same value; the memory model
  disciplines it so a value is never observably mutated through one reference while read through
  another in a way the executable semantics leaves unspecified.
- **Canonical byte form** — the single byte encoding of a value used wherever the value is
  hashed, compared across a boundary, or serialized; values equal under structural equality encode to
  identical bytes.

## Determinism, capabilities, and resources

- **Determinism** — the property that the same input and the same responses to a program's declared
  capabilities produce byte-identical output on every conforming runtime; made legible by requiring
  every source of nondeterminism to be a declared capability rather than forbidding it.
- **Nondeterminism** — any dependence of a result on something a program did not obtain through its
  declared inputs and capabilities: uninitialized memory, thread scheduling, an unspecified numeric
  result, or an outside influence such as a clock or randomness reached without declaring it.
- **Capability** — an authority a program must declare in order to reach a host operation; the
  unit of what a program is permitted to do.
- **Capability manifest** — the enumeration a program carries of every capability it requires;
  it is both a description and the enforcement boundary.
- **Ambient authority** — any authority a program can exercise without having declared it;
  Cadenza has none, because the means to reach an undeclared operation is simply not present in
  the emitted component.
- **Host operation** — an operation a component may import to interact with its runtime, drawn
  from a versioned host interface; each import is bound only when the manifest grants it.
- **Host interface** — the versioned set of host operations a component may import; a component
  names the exact interface version it targets.
- **Resource measure** — the deterministic quantity against which execution is accounted, so
  that termination does not depend on wall-clock time; commonly called fuel.
- **Fuel** — the resource measure a running component consumes; exhausting it halts execution
  at a defined point.
- **Capability honesty** — the property that a component's imports mirror its manifest exactly and
  the compiler adds none of its own, so the system running the component can decide what to permit
  from the manifest alone.
- **Capability-safety** — the guarantee that a program can reach a host operation only through a
  capability it declares, so a component carries no authority beyond its manifest; one of the three
  mandatory core guarantees, alongside static typing and determinism.
- **Effect** — an observable action a function performs through a capability; the opt-in effect-
  tracking layer annotates functions with the effects they perform and checks that they perform no
  other.

## The runnable form

- **Component** — the runnable form of a program: a content-addressed binary that imports only
  its declared host operations and runs in a sandbox behind a versioned host interface.
- **Content addressing** — identifying a component by a cryptographic hash over its own bytes,
  so that a source maps to a stable, verifiable component identity.
- **Component ABI** — the frozen mapping from Cadenza types to host-interface types, the
  calling convention across the component boundary, and the boundary memory layout; the
  byte-level contract an old component and a regenerated compiler must agree on.
- **Boundary** — the interface between a component and its runtime or another component, across
  which values are lowered and lifted.
- **Lowering** — the total function that converts a Cadenza value to its boundary
  representation.
- **Lifting** — the inverse of lowering, converting a boundary representation back to a Cadenza
  value.
- **Monomorphization** — replacing a generic definition with concrete specializations by the same
  compile-time reduction that specializes any definition applied to compile-time-known arguments,
  done before emitting a component interface because generics do not cross the boundary.
- **Type-valued parameter** — a parameter whose argument is a type-value, by which a generic is
  expressed as an ordinary definition taking types as arguments rather than through a separate
  parametric-polymorphism construct.

## Compilation and derivation

- **Compiler** — the tool that turns Cadenza source into a component; itself a regenerable
  projection of this specification and itself a component.
- **Derivation** — the act of turning source into its runnable, content-addressed component; a
  pure function of the canonical source and the pinned toolchain.
- **Reproducible derivation** — the property that the same canonical source and the same pinned
  toolchain produce byte-identical component output, so anyone can re-derive and verify.
- **Toolchain** — the pinned set of tools that performs a derivation; its identity is recorded
  alongside the component it produces.
- **Provenance** — build-environment information such as a timestamp, an absolute path, or a
  producer string; stripped or normalized so it cannot vary the output.
- **Compiled derivation** — deriving a component by lowering the program's canonical source to
  component code that runs on the one runtime; the sole normative derivation path, whose output must
  agree with the conformance corpus and with the other compiler implementation over the same input.
- **Phase** — a stage of the compiler with a defined input and output contract; each phase is a
  deterministic function of its input.
- **Diagnostic** — a machine-readable message the compiler emits about a program, carrying a
  stable code, a precise source span, and the rule it enforces.
- **Source span** — the precise region of source a diagnostic or structural element refers to.

## The executable semantics and the oracle

- **Executable semantics** — the single source of truth for what every language construct does,
  expressed as runnable cases; the compiler and every tool agree with it rather than encoding
  their own behavior.
- **Conformance corpus** — the collected executable-semantics cases, held as s-expression files
  parseable by a minimal reader; each case pairs an input with its expected result, a promoted
  compiler reproduces every recorded result, and the corpus is the behavioral oracle for the
  language.
- **Case** — one executable-semantics entry, written as an s-expression in a small test-DSL
  (`case`/`input`/`output`/`error`/`trap`/`doc`) wrapping a program in the canonical representation,
  pairing an input with the exact result its execution must produce.
- **Reference interpreter** — an optional realization of the executable semantics as a runnable
  interpreter that a generation MAY provide as an independent oracle to cross-check compiled output;
  not required for the semantics to be defined or for a program to be judged, and never the runtime
  of a promoted generation.
- **Oracle** — the authority against which a compiled program's observable behavior is checked; the
  oracle is the executable semantics as recorded by the conformance corpus, not any one program that
  computes it.
- **Two-compiler differential** — the independence of the behavioral judgment supplied by two
  implementations of the compiler, the foreign-language seed compiler and the Cadenza-authored
  compiler, which must agree on the observable behavior of every program a generation realizes, so
  that no single implementation is both the definition of behavior and its own judge.
- **Observable behavior** — the defined projection of a program run compared against the oracle: its
  terminal condition, the value it produces on normal termination in canonical value form, and the
  ordered sequence of host calls it made with the arguments it passed; it excludes internal
  representation, timing, and diagnostics.
- **Host call** — an invocation a program makes of a WIT-typed host function its manifest enumerates,
  carrying the arguments it passed and returning the function's result; the ordered sequence of a run's
  host calls is a constituent of its observable behavior, and two host calls are the same exactly when
  they name the same function and their arguments are equal under the canonical byte form. Which host
  functions exist is the target's concern, not the language's (an *event* is one target's host call
  that returns unit). Distinct from an **effect**, which is the type-system label — a program's escaping
  effect row equals the host functions it imports.
- **Terminal condition** — the one way a program run ends: a normal result, a trap of a defined kind,
  or exhaustion of the resource measure.
- **Trap** — a defined-kind halt of a program at a defined point, raised by a partial operation or an
  overflow, distinct from normal termination and from resource-measure exhaustion; part of observable
  behavior.

## Verification layers

- **Verification layer** — an optional, meaning-preserving addition above the mandatory core
  that lets a program state and discharge stronger properties; adding one never changes what a
  program already means.
- **Contract** — a precondition, postcondition, or invariant a program states, checked
  dynamically or discharged statically without changing the emitted bytes.
- **Obligation** — a property a verification layer requires to hold; the compiler discharges it
  or rejects the program, never silently ignores it.
- **Discharge** — establishing that an obligation holds, either dynamically at runtime or
  statically before emission; a static discharge does not change the bytes emitted.
- **Certificate** — a recorded, reproducibly checkable witness that a statically discharged
  obligation holds, so that validation does not depend on a nondeterministic solver run.
- **Property test** — a check that a stated relationship holds across generated inputs;
  reproducible from a recorded seed, with generation constrained by refinements and shrinking
  that converges to a minimal failing input.
- **Dimensional analysis** — the optional, compile-time-only verification layer that checks the
  consistency of quantities carrying units of measure and then erases them.

## Self-hosting and bootstrap

- **Seed compiler** — the first Cadenza toolchain, authored in a foreign host language because
  nothing yet exists to compile Cadenza; a native reference compiler that lowers Cadenza's canonical
  representation to a runnable component and runs it, and the operator-synthesized origin
  of the staged path to self-hosting.
- **Self-hosting** — the state in which the Cadenza compiler is itself authored in Cadenza and
  derivable by the previous generation of the compiler.
- **Generation** — one produced version of the language or its compiler; each generation is
  derived by the generation before it and judged by the gates.
- **Flywheel** — the loop in which the running system reads the specification, synthesizes the
  next generation of the language as source, derives it, gates it, and activates it, so the
  system builds the system.
- **Ignition** — the demonstration that the seed toolchain performs a real, executed end-to-end
  derivation: a Cadenza source program derived to a content-addressed component whose imports mirror
  its manifest, actually run to produce its output, byte-identically re-derivable, and in agreement
  with the conformance corpus (the behavioral oracle); the appearance of a derivation without a
  component that was actually derived and run is not an ignition.
- **Ignition bar** — the bar an ignition must clear, fixed by the bootstrap specification; clearing it
  is the point at which the seed toolchain can produce the next generation.
- **Ignition subset** — the subset of requirements the seed toolchain must cite to clear the ignition
  bar — the constitution, the frozen contracts, the bootstrap specification, and the capability
  specifications the seed itself realizes — a strict subset of the full requirement set.

## Conformance and change

- **Requirement** — a single self-contained RFC-2119 sentence under a stable heading; the unit
  of normative obligation, identified by the tuple of its file, section, and quoted sentence.
- **Requirement gate** — the conformance gate that extracts every normative requirement and
  maps it to the implementation and test that satisfy it.
- **Behavior gate** — the gate under which every executable-semantics case must reproduce its
  recorded output; the execution check distinct from citation coverage.
- **Citation** — the pairing of a requirement with the code that implements it and the test
  that exercises it; a citation discharges its own requirement only if it annotates the code
  that performs the behavior and its test fails when that behavior is removed.
- **Enforcing line** — a specific line that detects a requirement's violation; every
  requirement binds to one, so that an unenforceable statement is not written as a requirement.
- **Load-bearing requirement** — a requirement that must be covered for a generation to be
  promoted; a requirement of an excluded optional capability is not load-bearing for that build.
- **Promotion** — accepting a regenerated compiler as the new generation; refused if any
  load-bearing requirement is uncovered or any behavior case fails.
- **Frozen contract** — a specification that pins a byte- or ABI-level form honored across every
  regeneration; changed only additively or by a coordinated version increment with a migration
  path.
- **Governance floor** — a minimum-process rule that no evolution policy may lower.
- **Decision** — an open point recorded as a directory under the declared-default location, whose
  README states the requirements a choice must satisfy and names the default choice.
- **Choice** — one candidate realization of a decision; a build adopts a decision's default choice,
  selects another listed choice, or authors a new choice for that one decision.
- **Declared default** — the choice a decision names as the one an autonomous build applies when
  nothing else resolves the decision, so two unattended builds resolve it identically.
- **Declared-default location** — the committed `options/` directory where each decision's choices
  and default are recorded, used because the standalone rule forbids naming an implementation choice
  in a requirement.
- **Open point** — a specification point a conforming generation could resolve in more than one
  way; each is a decision carrying a declared default.
- **Attended mode** — a build driven by an author who can resolve ambiguity: it halts at a
  specification ambiguity, has it resolved and folded into the specification as a requirement, and
  restarts from the corrected specification.
- **Autonomous mode** — a build driven by a user who cannot resolve internals: it never halts on a
  specification ambiguity and instead applies the point's declared default and records that it did.
- **Decision record** — the durable record of the mode, declared defaults, user-facing choices, and
  optional capabilities a build applied, so the assumptions behind the produced compiler are
  auditable.
- **Operator-gated point** — a specification point whose resolution the constitution reserves to an
  operator (the core symbol namespace, a frozen-contract byte-level pin); it must be resolved in the
  committed specification before an autonomous build can run, and an autonomous build never invents
  it.

---

*Informative references (never cited normatively): the design vocabulary of earlier Cadenza
generations and the surrounding literature — Hindley-Milner inference, linear types, algebraic
effects, SMT solving, the WebAssembly component model — inform these terms but are named only
here and in `options/` and `learnings/`, never in a requirement.*
