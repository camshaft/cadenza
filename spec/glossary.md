# Cadenza — Glossary

> **What this document is.** The controlled vocabulary for the whole specification. Every
> term used normatively in the constitution, the frozen contracts, and the capability
> specifications is defined here exactly once, so the specs use words consistently. This
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
- **Surface** — the human- and agent-readable textual form of a program: the concrete syntax
  a person or agent reads and writes.
- **Canonical representation** — the homoiconic, typed, code-as-data structure that is a program's
  durable form: the sole target of structural manipulation, hashing, the executable semantics, and
  verification. Every display is a projection of it.
- **Display** — a deterministic rendering of the canonical representation for reading or writing;
  more than one display may exist, and moving between displays never changes the program.
- **Canonical textual form** — the one display designated for the byte-for-byte round-trip;
  formatting is idempotent and parse-then-format reproduces it byte-for-byte.
- **Homoiconicity** — the property that a program is itself a value of the uniform data structure the
  language manipulates, so that display and representation decouple and code is data.
- **Structural interface** — the documented interface through which an agent reads and rewrites the
  canonical representation of a program without textual patching and without re-parsing unrelated
  code.

## Types and values

- **Value** — a datum a program computes with; every value has a type and a canonical byte
  form.
- **Type** — a static classification of expressions that the compiler determines before
  compilation; types are erased from the runnable form.
- **Inference** — the process by which the compiler determines an expression's type without an
  explicit annotation; where defined, it yields the most general type consistent with use.
- **Annotation** — an explicit type written in source; it constrains inference and is rejected
  on conflict, never silently overridden.
- **Erasure** — the property that no type information the compiler cannot remove survives into
  the runnable form, so the component carries no runtime type reflection.
- **Nominal type** — a type whose identity is its declared name, distinct from any
  structurally identical type of a different name.
- **Structural type** — a type whose identity is its shape, equal to any type of the same
  shape.
- **Sum type** — a type that is exactly one of several named variants, each optionally carrying
  data; the basis for typed results and error handling.
- **Refinement** — a predicate attached to a type that constrains which of its values are
  admissible; part of the verification layers, not the mandatory core.
- **Canonical byte form** — the single byte encoding of a value used wherever the value is
  hashed, compared across a boundary, or serialized; equal values encode to identical bytes.

## Determinism, capabilities, and resources

- **Determinism** — the property that the same input produces byte-identical output on every
  conforming runtime; achieved primarily by denying a program any source of nondeterminism.
- **Nondeterminism** — any dependence of a result on something other than declared inputs: a
  wall-clock time, a source of randomness, uninitialized memory, thread scheduling, or an
  unspecified numeric result.
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
- **Fold** — a pure, deterministic reduction of prior state and inputs into new state; the most
  constrained role, granted no source of nondeterminism at all.
- **Role** — the kind of behavior a component fills, which determines how constrained it is;
  the fold role is the most constrained.

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
- **Monomorphization** — replacing a generic definition with concrete specializations, done
  before emitting a component interface because generics do not cross the boundary.

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
- **Interpreted derivation** — deriving a component by emitting the reference interpreter over
  the program's canonical source; the initial derivation mode, satisfying every guarantee a
  compiled derivation must.
- **Compiled derivation** — deriving a component by ahead-of-time compilation of source to
  native component code; the maturation of the toolchain, which must agree with the oracle.
- **Phase** — a stage of the compiler with a defined input and output contract; each phase is a
  deterministic function of its input.
- **Diagnostic** — a machine-readable message the compiler emits about a program, carrying a
  stable code, a precise source span, and the rule it enforces.
- **Source span** — the precise region of source a diagnostic or structural element refers to.

## The executable semantics and the oracle

- **Executable semantics** — the single source of truth for what every surface construct does,
  expressed as runnable cases; the compiler and every tool agree with it rather than encoding
  their own behavior.
- **Semantics corpus** — the collected executable-semantics cases; each case is an input paired
  with its expected output, and a promoted compiler reproduces every recorded output.
- **Case** — one executable-semantics entry: a program input and the exact output its execution
  must produce.
- **Reference interpreter** — the realization of the executable semantics as a runnable
  interpreter that serves as the behavioral oracle for the language.
- **Oracle** — the authority against which a compiled program's observable behavior is checked;
  the reference interpreter is the oracle.
- **Observable behavior** — the input-to-output relation and emitted events of a program, as
  distinct from how it is represented internally; what must agree between a compiled program and
  the oracle.

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
  nothing yet exists to compile Cadenza; the operator-synthesized origin of the staged path to
  self-hosting.
- **Self-hosting** — the state in which the Cadenza compiler is itself authored in Cadenza and
  derivable by the previous generation of the compiler.
- **Generation** — one produced version of the language or its compiler; each generation is
  derived by the generation before it and judged by the gates.
- **Flywheel** — the loop in which the running system reads the specification, synthesizes the
  next generation of the language as source, derives it, gates it, and activates it, so the
  system builds the system.

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
- **Declared default** — the concrete choice recorded outside the normative prose that resolves
  an open point, so an autonomous build proceeds without halting.
- **Declared-default location** — the committed `defaults/` directory where a declared default's
  concrete, technology-named realization is pinned, used because the standalone rule forbids
  naming an implementation choice in a requirement.
- **Open point** — a specification point a conforming generation could resolve in more than one
  way; each carries a declared default.

---

*Informative references (never cited normatively): the design vocabulary of earlier Cadenza
generations and the surrounding literature — Hindley-Milner inference, linear types, algebraic
effects, SMT solving, the WebAssembly component model — inform these terms but are named only
here and in `defaults/` and `learnings/`, never in a requirement.*
