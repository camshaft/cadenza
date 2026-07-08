# Capability — Compiler Pipeline

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the compiler's phases and the obligations each carries, and names the two gates a
> generation must pass. Requirements realize [Core Principle II](../../constitution.md),
> [Core Principle IX](../../constitution.md), and [Core Principle XII](../../constitution.md) and
> trace to [overview §7](../overview.md), [overview §10](../overview.md), and
> [overview §14](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes that the compiler proceeds through well-defined phases each of which is a
deterministic function of its input, that diagnostics from one phase do not abort the rest, and that
a generation is judged by two gates: the requirement gate (every load-bearing requirement cited by an
implementation and a test) and the behavior gate (every executable-semantics case reproduces its
output). It states the obligations phases carry; it does not prescribe the phase decomposition beyond
requiring that one exist and be respected.

## Representation

### The Compiler Operates On AST Values

The compiler MUST receive the program as an AST value obtained via quote or decode from the binary form.

The compiler MUST represent the instructions it emits as values of a typed sum type — the AST sum or a dedicated instruction sum — deconstructible by pattern matching, not as string-tagged pseudo-structures, so that an instruction is inspected like any other Cadenza value rather than by matching on a string tag.

The compiler MUST serialize instruction values to bytes through a recursive function that pattern-matches the instruction sum type exhaustively over its variants, so that an instruction variant the serializer does not handle is a compile-time error rather than a silent fall-through.

### The Compiler Resolves Names Before It Selects Instructions

The compiler MUST lower the AST to an intermediate representation in which every name reference is resolved to the binding it denotes before it selects the instructions to emit, so that instruction selection reads a resolved binding rather than searching a scope.

The compiler MUST determine the handler that discharges each performed effect operation from the structure of the resolved intermediate representation, so that the discharging handler of an operation is fixed before instruction selection rather than by state accumulated while instructions are emitted.

### Emission Serializes A Lowered Representation

The compiler MUST perform name resolution, type checking, and each transformation it applies to a program as a transformation of its intermediate representation rather than as an effect of emitting instruction bytes.

The step that emits instruction bytes MUST consume an already-lowered representation and MUST NOT itself resolve a name, decide a type, or choose an effect's handler, so that emission is the serialization of decisions already made.

### The Compiler Constructs AST Values Via Quasiquote

The compiler MUST use quasiquote to construct the AST values it builds programmatically — in its frontend and its macro layer, where the values it constructs are themselves program syntax — so that AST-construction code is readable and maintainable rather than a wall of manual AST constructor calls, while a dedicated instruction sum is built by ordinary constructors and pattern-matched to bytes.

## Phases

### The Pipeline Has Defined Phases

The compiler MUST proceed through phases each of which has a defined input and a defined output.

Each phase MUST produce output that is a deterministic function of its input.

### Phases Recover From Errors

A phase that encounters an error in one part of a program MUST record a diagnostic for that error.

A phase that encounters an error in one part of a program MUST continue processing the well-formed remainder rather than abort the whole compilation.

The compiler MUST report all diagnostics it can produce for a program rather than stop at the first.

## The Behavior Gate

### The Corpus Is A Gate

A build MUST fail if any executable-semantics case whose required capabilities the generation realizes does not reproduce its recorded output.

A behavior requirement MUST be discharged by executing the behavior and observing its output rather than by inspecting the shape of the code that implements it.

## The Requirement Gate

### Coverage Is A Gate

A generation in which any load-bearing requirement lacks both an implementation citation and a test citation MUST NOT be promoted.

The requirement gate MUST emit a machine-readable report from which the covered fraction of load-bearing requirements can be computed.

## The Two Gates Together

### Both Gates Must Pass

A generation MUST pass both the requirement gate and the behavior gate before it is promoted.

A generation that passes the requirement gate while failing the behavior gate MUST NOT be promoted.
