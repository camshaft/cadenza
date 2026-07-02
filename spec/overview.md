# Cadenza — Architecture Overview

> **What this document is.** The target architecture: a description of *what Cadenza is when it is
> built*. It is the intent arbiter — every normative requirement in the constitution, the frozen
> contracts, and the capability specifications traces back to a section here (see
> [traceability](./traceability.md)). When a specification and this document disagree about intent,
> this document is the north star and the specification is corrected to match.
>
> This document is descriptive, not normative: it carries no RFC-2119 requirements. The requirements
> that realize this architecture live in `constitution.md`, `spec/contracts/**`, and
> `spec/capabilities/**`. Section headings here are stable identifiers; they are cited by name from
> [traceability](./traceability.md), so they change only deliberately.

---

## 1. The one idea

Cadenza is a programming language whose purpose is to be **written and read by AI agents, read by
humans, verified for its properties, and compiled to sandboxed WebAssembly components** — and to be
itself such a component, so that the language can build the next version of itself.

Three consequences define the whole architecture:

- **The source is the truth; the component is a projection of it.** A program's meaning lives in its
  source; its runnable form is a content-addressed component that is a reproducible function of that
  source. A defect is fixed in the source or the specification and recompiled, never by editing a
  component.
- **The properties that make a program trustworthy are guaranteed by construction, not by
  convention.** Determinism, capability-safety, and bounded termination are not features a careful
  author opts into; they are the floor the language stands on, enforced by the environment a program
  compiles to.
- **The specification is the durable artifact; the compiler is disposable.** The compiler is a
  regenerable projection of these specifications. A wrong design is fixed by reworking the spec and
  regenerating, not by patching a live compiler — which is safe because the source of every program
  survives, and the compiler that derives it can be rebuilt.

## 2. Why Cadenza exists

Cadenza is the source language and derivation tool for a system in which **behavior itself is data**:
units of behavior are published as source plus a manifest of the capabilities they require, and the
runnable form of that behavior is a sandboxed, content-addressed component derived from the source.
The environment that runs such behavior can only load, verify, and run components — it contains no
compiler. Cadenza is the replaceable, capability-gated tool that turns source into a conformant
component, and is itself such a component. The constraints that environment places on the behavior
it runs — determinism, capability-safety, bounded termination, reproducible derivation, content
addressing — are the constraints this language is designed around. They are not a burden Cadenza
tolerates; they are the specification it fulfills.

## 3. Source, programs, and the canonical form

A program is a set of source modules that compile together into one component. The authoritative
form of a program is its source, and *the canonical stored form of that source is a stable binary
serialization of its abstract syntax tree* — not text. That binary AST is what a program is stored
as, hashed as, and handed to the compiler as, so "the same program" is a byte-exact,
third-party-checkable notion with no dependence on whitespace, line endings, or which syntax it was
typed in. Because the stored form is the tree, *text is only a projection*: a textual syntax is a
parser that converts text to the binary AST and a printer that renders the AST back, and there may be
many — a human-readable conventional syntax, a direct code-as-data syntax, or any a deployment adds —
none of them the program's identity. An agent generates and transforms a program by constructing the
tree directly; a human reads and writes it through whichever syntax they prefer; both converge on the
same binary AST. The tree carries everything a program means to keep, including comments and
documentation, so nothing is lost by storing the tree rather than a rendering.

## 4. Determinism and bounded execution

A compiled component produces byte-identical output given the same input and the same responses to
its declared capabilities, on every conforming runtime, and its execution is bounded by a
deterministic resource measure rather than by wall-clock time. Cadenza's discipline is not to forbid
nondeterminism but to make it **legible and never latent**: a program obtains a clock, randomness, or
any other outside influence only through a capability it declares, so its determinism is readable from
its manifest — a program that declares no such capability is deterministic, and a program that declares
one has said so where anyone can see it. The compiler's own contribution is to add **none of its own**:
it emits no operation whose result depends on uninitialized memory or thread scheduling, and what a
sandbox does not pin on its own — the result of a numeric operation, the order of emission — the
compiler pins, so that any residual variation is one the program explicitly asked for. Every value has
one canonical byte form, so "the output" and "the identity of a value" are exact. What a *particular
kind of program* is permitted to declare — for instance, that a program folding a shared log may hold
no nondeterministic capability at all — is a policy of the system that runs it, not of the language.

## 5. Types

Every expression in a well-formed program has a statically determined type before the program
becomes a component, and an ill-typed program is rejected rather than compiled with a deferred error.
Inference determines types without ceremony where it can, and an explicit annotation constrains
inference rather than contradicting it. Types are erased from the runnable form: the component
carries no runtime type reflection, and nothing it does depends on type information the compiler
could not remove. The type system is the mandatory floor beneath the optional verification layers.

## 6. Capabilities and no ambient authority

A program declares, up front, every capability it requires — every host operation it may call. The
component the compiler emits imports exactly those operations and no others, because the means to
reach anything undeclared is simply not present in the component. The manifest is therefore both a
description and an enforcement boundary. A program that reaches an operation it did not declare is
rejected at compile time; it is never compiled to a component with a latent import. Because the
manifest is exhaustive, the system that runs a component can decide from it alone what to allow — for
instance, that a program in a given role may hold no nondeterministic or outward-facing capability —
without the language having to bake that policy in.

## 7. Derivation and reproducibility

Turning a program's source into its runnable, content-addressed component is derivation, and
derivation is a pure function of the canonical source and the pinned toolchain: the same inputs
always produce the same bytes. The compiler strips or normalizes anything that would otherwise vary
between builds — timestamps, build paths, producer strings — and emits its output in an order
determined by the source alone. A component is addressed by a hash over its own bytes and bound to
that hash, so anyone can re-derive it from source and confirm it matches, without trusting the party
that first produced it. This holds at two levels: the compiler derives programs reproducibly, and
the compiler itself is derived reproducibly.

## 8. The component boundary

A component interacts with its runtime and with other components only across a fixed boundary. The
mapping from Cadenza's types to the host interface's types, the calling convention, and the layout of
values that cross the boundary are pinned, so that a component derived by one generation of the
compiler interoperates with a component derived by another. Generics do not cross the boundary; the
compiler specializes exported signatures to concrete types first. The boundary is the byte-level
agreement that lets components composed from different sources and different compiler generations fit
together.

## 9. Cadenza as a replaceable build tool

Cadenza is invoked as a build tool: it consumes a canonical source tree and produces a component, its
manifest, and machine-readable diagnostics. It is itself a verified, reproducibly-derived component,
and it is not part of any minimal root whose only job is to load, verify, and run components — a new
build tool, for Cadenza or for another language, is introduced by providing the tool, not by changing
that root. Because a working component must exist before ahead-of-time compilation is complete,
Cadenza may derive a component by embedding a reference interpreter over the program's source; such a
component satisfies every guarantee a fully compiled one does and behaves identically.

## 10. One executable semantics

The behavior of every language construct is defined by exactly one executable semantics, which is the
single source of truth for what a program does. The compiler and every tool agree with that semantics
rather than each encoding their own, and every conformance test for a construct derives from it. This
is the structural answer to a language whose meaning had previously been scattered across an
interpreter, a separate document, a generated implementation, and a formal model that drifted apart:
there is one place a construct's meaning lives, and it is runnable.

## 11. The reference interpreter as oracle

The executable semantics is realized as a reference interpreter, and that interpreter is the
behavioral oracle: a compiled program's observable behavior must agree with the reference interpreter
over the same input. This makes "one executable semantics" a shippable artifact rather than only
prose, and it turns ahead-of-time compilation into an optimization that must match the oracle rather
than a second, independent definition of the language. It is also the seam through which the language
reaches self-hosting: the interpreter is authored in Cadenza and derived by the compiler that came
before it.

## 12. Progressive verification

A program compiles when only the core guarantees hold — static typing, determinism, and
capability-safety. Above that floor sit optional, ordered verification layers: contracts,
refinement types, and machine-checked proofs. Each layer is meaning-preserving — adding it never
changes what a program that already compiled does — and each obligation a layer states is either
discharged or the program is rejected, never silently ignored. Crucially, whether an obligation is
discharged statically does not change the bytes the compiler emits, so a nondeterministic solver
never enters the reproducible byte path; a static discharge is recorded as a reproducibly checkable
certificate. This lets a program grow from a quick sketch to a machine-proven artifact without ever
becoming two different languages.

## 13. Authored for agents

The affordances that make Cadenza easy for an agent to produce and transform are first-class: a
structural interface reads and rewrites a program's tree without textual patching and without
re-parsing unrelated code; a structural edit either yields a well-formed program or reports a
machine-readable rejection; the canonical formatter round-trips byte-for-byte, so a generated
program and its re-read form are identical; and every output of the compiler — its diagnostics, the
types it inferred, the manifest it produced — is machine-readable, each diagnostic carrying a stable
code and the rule it enforces so an agent can act on it programmatically.

## 14. Conformance by two gates

Two independent gates decide whether a regenerated compiler is real. The **requirement gate**
extracts every normative sentence and maps it to the implementation and test that satisfy it; a
generation in which any load-bearing requirement lacks both is not promoted. The **behavior gate**
runs every executable-semantics case and requires it to reproduce its recorded output. The two gates
guard against different failures: the requirement gate against unimplemented obligations, the
behavior gate against a compiler that passes its citations while its behavior is a stub. A
requirement that describes runtime behavior is discharged by executing that behavior, not by citing
code shaped like it, and a requirement that no mechanism can enforce is not written as a requirement
at all.

## 15. Self-regeneration: the flywheel

The purpose of a minimal environment that only runs components, plus a language whose compiler is a
regenerable projection of its specification, is that the running system can work on itself. An agent
reads the specification, synthesizes the next generation of the language as Cadenza source, derives
it to a component with the previous generation of the compiler, runs both gates against it, and
proposes it for activation — each step a reviewed, capability-gated event on the log. The flywheel
has not turned until a generation is actually derived, gated, and run and the system's behavior has
changed because a component it built is now executing; emitting the events that would accompany that
is not that. The language thereby extends and improves itself, with only the seed compiler ever
standing outside the loop.

## 16. What earlier generations taught

Cadenza has been attempted before, and the attempts are why this architecture is shaped as it is. A
compiler core was rebuilt several times, and each rebuild discarded accumulated intent, because the
compiler was treated as the artifact — hence the specification, not the compiler, is now the durable
artifact. A component-emitting backend was designed thoroughly but never actually produced a running
component, because the byte-level target was never pinned first — hence the component ABI and the
determinism forms are frozen contracts written before the capabilities that depend on them. The
meaning of the language lived in several places that drifted — hence one executable semantics.
Verification was designed as always-on and coupled a simple language to a heavy prover — hence
verification is layered. And there was never a concrete path from the language to the language
building itself — hence the reference interpreter is the oracle and the seam to self-hosting. These
lessons are recorded in [learnings](./learnings/).
