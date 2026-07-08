# Capability — Debug Information

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the source-level debug information the compiler may emit for a derived artifact so
> that an external debugging tool can relate a position in the running artifact back to the source it
> derives from. Requirements realize [Core Principle II](../../constitution.md),
> [Core Principle VI](../../constitution.md), and [Core Principle VII](../../constitution.md) and
> trace to [overview §7](../overview.md) and [overview §13](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading. The concrete debug-information format is pinned at
> the declared-default location.

## Purpose And Scope

This capability fixes that the compiler can, when a build asks it to, emit debug information for a
derived artifact — a mapping from positions in the executable form back to the source constructs they
derive from, together with the source-level names and types a debugger presents — and that doing so
never changes what the program means, what bytes it executes, what it imports, or whether its
derivation is reproducible. Debug information is metadata an external tool reads, not a value the
running component computes with; it is the compile-time counterpart of the replay-based runtime
observation fixed by [tooling-and-lsp.md](./tooling-and-lsp.md) (§Deterministic Replay Is The
Debugger), serving a stepping debugger where replay serves value inspection. This capability states
the obligations debug information carries; the concrete interchange format that realizes them is the
declared-default debug-information choice.

## Debug Information Is Inert

### Emitting Debug Information Does Not Change Observable Behavior

Emitting debug information for a program MUST NOT change the program's observable behavior.

The portion of an artifact that the runtime executes MUST be byte-identical whether or not debug information is emitted for it, so that debug information occupies a region of the artifact the runtime does not execute rather than altering the code that runs.

### A Running Component Cannot Observe Its Own Debug Information

A component MUST NOT be able to read its own debug information while it runs, so that debug information is metadata for an external tool rather than runtime type reflection the erasure guarantee removes (type-system.md §Types Are Erased From The Component).

Debug information MUST NOT add a host operation to a component's manifest, so that a component carrying debug information imports exactly the operations the same component without it imports and its capability manifest is unchanged.

## Debug Information Maps The Artifact To Source

### Debug Information Relates An Execution Position To Its Source

Debug information MUST relate a position in the executable artifact to the source construct of the canonical representation it derives from, so that an external tool can present an execution position as a location in the program's source.

### A Source Location Is A Span Over The Canonical Representation

A source location recorded in debug information MUST be a source span over the canonical representation, so that the location is stable under any textual rendering rather than tied to one textual syntax.

A source span recorded in debug information MUST be renderable to a textual source location by the printer, so that a textual debug view is a projection of the canonical form through the same printer any textual syntax uses rather than a second authority over where a construct is.

### A File Reference Is A Tree-Relative Module Path

A file reference recorded in debug information MUST be the tree-relative module path fixed by the source-tree-encoding contract rather than an absolute filesystem path, so that debug information names a source module the same way the canonical source tree does and carries no build-host path.

### Debug Information May Carry Source-Level Names And Types

Debug information MAY carry the source-level name of a definition or binding, so that an external tool can present a value under the name its source gives it.

Debug information MAY carry the source-level type of a binding as descriptive information, so that an external tool can present a value's type even though the executable form carries no runtime type.

A source-level name or type carried in debug information MUST NOT be reachable by the running component, so that carrying it for an external tool does not reintroduce the runtime type reflection erasure removes.

## Debug Information Is Reproducible

### Debug Information Is A Deterministic Function Of Source And Toolchain

The debug information the compiler emits MUST be a deterministic function of the canonical source and the pinned toolchain, so that two derivations of the same source with the same toolchain emit byte-identical debug information.

The order in which debug information records its entries MUST be a deterministic function of the source, independent of filesystem enumeration order or nondeterministic collection iteration.

### Debug Information Carries No Provenance

The compiler MUST NOT embed into debug information a wall-clock time, an absolute filesystem path, or a build-host identifier.

The compiler MUST NOT embed into debug information a producer or build-environment string that would otherwise vary between builds of the same source.

## Debug Information Is Separable

### Debug Information May Be Embedded Or Emitted As A Sidecar

Debug information MAY be emitted embedded in the artifact it describes, so that the debug information travels with the runnable artifact as a single self-describing file.

Debug information MAY instead be emitted as a separate artifact linked to the artifact it describes, so that a deployment can ship the runnable artifact lean and the debug information alongside it.

The compiler MUST emit a reference, reachable from the runnable artifact, that identifies the separately emitted debug artifact describing it, so that a tool holding the runnable artifact can locate the debug information for it.

### Stripping Debug Information Recovers The Undecorated Artifact

Removing the debug information from an artifact that carries it MUST yield the byte-identical artifact the same source derives when debug information is excluded, so that debug information is a strippable addition to the runnable form rather than a modification of it.

## Debug Information Is Consumable By Standard Tools

### Debug Information Uses An Interchange Format

Debug information MUST be emitted in an interchange debug-information format that an external debugging tool consumes, rather than a form only Cadenza's own tooling can read, so that an existing debugger relates the artifact to its source without bespoke Cadenza support.

The concrete debug-information format MUST be pinned at the declared-default location, so that two builds that emit debug information emit it in the same format.

## Emitting Debug Information Is A Build Choice

### Whether To Emit Debug Information Is A User-Facing Choice

Whether a derivation emits debug information MUST be treated as a user-facing build choice rather than a specification ambiguity, because it changes which artifact a deployer wants rather than what the program means.

Whether a derivation emits debug information MUST carry a declared default so that a non-interactive or autonomous build proceeds without asking.

A build MUST record in its decision record whether it emitted debug information, so that whether a produced artifact carries debug information is auditable.

## Optionality

### This Capability Is Optional

Debug-information emission MUST be an optional capability a build may include or exclude, in accordance with the build's declared defaults.
