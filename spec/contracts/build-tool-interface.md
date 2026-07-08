# Frozen Contract — Build Tool Interface

> **FROZEN CONTRACT.** This document pins the interface Cadenza-the-compiler presents when it is
> invoked to derive a component: what it consumes, what it produces, and the properties it
> guarantees, so that a host can invoke it as a replaceable build tool and swap it for another
> language's tool without changing what loads and runs the result. It is versioned and changed only
> by the coordinated act described in the constitution's Governance Floors. Its requirements realize
> [Core Principle I](../../constitution.md), [Core Principle VI](../../constitution.md), and
> [Core Principle XIV](../../constitution.md) and trace to [overview §7](../overview.md) and
> [overview §9](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract names the tool's interface, not a concrete host; the concrete
> invocation surface is pinned at the declared-default location.

## Purpose And Scope

Cadenza is a build tool: something invokes it with a program's source and receives a runnable
component. This contract fixes that interface — the canonical source tree in, the component and its
manifest and diagnostics out — and the properties that make Cadenza a *replaceable* tool: it is
itself a verified, reproducibly-derived component, and it is not part of any minimal root that only
loads, verifies, and runs components. It also fixes that the build tool derives a component by
lowering the source to a component. It does not fix the concrete invocation surface,
which is a declared default.

## What The Tool Consumes And Produces

### The Tool Consumes A Canonical Source Tree

The build tool MUST accept its input as a canonically encoded source tree as fixed by the source-tree-encoding contract.

The build tool MUST reject an input that is not a well-formed canonical source tree with a machine-readable diagnostic rather than an opaque failure.

### The Tool's Inputs Are A Kinded Artifact List

The build tool's derivation entry MUST take its inputs as a list of kinded artifacts, each a named kind paired with its bytes, so that the canonical source tree is one artifact among an open set and the input channel admits further inputs — additional source units of a multi-unit program, a build cache, or a previously derived dependency — without changing the entry's arity.

The kind of an artifact MUST identify how its bytes are interpreted, so that a consumer selects an input by kind rather than by position, and an input kind the tool does not recognize is reported as a diagnostic rather than silently ignored.

An input artifact list that omits the source tree the tool requires to derive a component MUST be reported as a diagnostic rather than producing an empty or arbitrary output.

### The Tool Produces A Component, A Manifest, And Diagnostics

The build tool MUST produce, on success, a content-addressed component together with the capability manifest against which its imports are bound.

The build tool MUST produce, on failure, machine-readable diagnostics rather than an opaque error.

The build tool's derivation entry MUST return a record pairing a list of kinded output artifacts with a list of diagnostics, so that the byte outputs and the diagnostics are distinct channels rather than mutually exclusive arms of one result.

The derived component MUST be one artifact in the output artifact list, identified by its kind, so that a byte output that is not the component — a debug-information sidecar, a source map, the capability manifest — is another artifact of the same shape rather than a second return type, and the set of output kinds is open to additive extension.

The tool MUST signal a successful derivation by the presence of a component artifact in the output together with the absence of any error-severity diagnostic, and a failed derivation by the absence of a component artifact together with at least one error-severity diagnostic, so that success and failure are read from the produced artifacts and diagnostics rather than from an in-band sentinel such as an empty byte sequence.

The tool MUST be able to return diagnostics alongside a produced component, so that a derivation that succeeds while reporting non-error diagnostics — a warning — carries both the component and those diagnostics rather than having to discard one.

The component the build tool produces MUST have imports that mirror the manifest it produces, as fixed by the host-interface-binding contract.

### A Diagnostic Carries A Severity, A Code, And A Message

A diagnostic the build tool produces MUST carry a severity that distinguishes an error — one that denies a component artifact — from a non-error such as a warning that accompanies a produced component, so that a consumer decides from the diagnostic itself whether the derivation failed.

A diagnostic MUST carry the machine-readable code and message fixed by the diagnostics-schema, so that a diagnostic in this interface is the same machine-actionable record the rest of the specification uses.

## The Tool Is Replaceable

### The Tool Is Itself A Verified Component

The build tool MUST be a content-addressed component.

The build tool MUST itself be reproducibly derivable from its own source.

The build tool MUST NOT be part of any minimal root whose only responsibilities are to load, verify, and run components.

### A New Source Language Is A New Tool

A minimal root whose only responsibilities are to load, verify, and run components MUST NOT contain a compiler for any source language.

A minimal load-verify-run root MUST be unchanged by the introduction of a build tool for a different source language.

## Derivation By Compilation

### The Tool Lowers Source To A Component

The build tool MUST derive a component by lowering the program's canonical source to a component.

A derived component MUST satisfy the determinism guarantee.

A derived component MUST satisfy the capability-binding guarantee.

A derived component MUST satisfy the reproducibility guarantee.

A derived component MUST exhibit the observable behavior the executable-semantics corpus records for the program.

### A Derived Component Computes Its Behavior When It Runs

A derived component MUST compute the program's observable behavior when it runs rather than replay an observable behavior recorded before it ran.

A build tool MUST NOT emit a component that reproduces only a pre-recorded transcript of the program's observable behavior.

A derived component's behavior MUST be a function of the compiled program rather than of behavior-specific code the derivation emitted.

### The Component And The Host Are Distinct Artifacts

A derived component MUST be an artifact distinct from the host that provides the capability operations it imports.

The host that provides a component's imported capability operations MUST provide only the operations the component's manifest enumerates.

## The Compiler Exposes Reader, Printer, And Display As Exports

### The Compiler's Interface Exports Derivation, Reading, Printing, And Display

The compiler component's interface MUST export, alongside its derivation entry, a reader that converts program text to the canonical source tree, a printer that renders the canonical source tree as re-readable text, and a display conversion that renders a typed result as its canonical text.

The reader, printer, and display conversion MUST be exports of the compiler's own interface rather than operations any host performs, so that the knowledge of a value's textual form lives in the compiler and a host stays value-agnostic (host-interface-binding.md §The Host Formats Nothing).

### The Compiler Imports No Host Function

The compiler component MUST reach no host function to derive a program, read text, print the canonical source tree, or render a result, so that its host-import world is empty and its derivation is a pure function of its input.

The compiler MAY import the single, well-known value-heap runtime interface to construct and render the compound values it forms (ASTs, diagnostics, results), because that import is not a host function and not a capability (capabilities-and-effects.md §The Value-Heap Runtime Is The One Import That Is Not A Capability), so the compiler's manifest stays empty and its runtime import is the same one every program carries.

The empty host-import world of the compiler MUST be the same "purity is the empty row" property every program with an empty manifest has, so that the compiler is not a special case of the capability model but an instance of it.

### A Typed Result Crosses The Boundary As Its Proper Type

A compiled program's entry MUST export its result as the result's proper component type rather than collapse it to an untyped string at the boundary, so that the boundary is strictly, statically typed.

The display conversion the compiler exports MUST be the path by which a harness obtains a result's canonical text, so that rendering a value is a compiler-exported operation over the typed result rather than a formatting rule the harness carries.

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-derived components and already-invoked tool interfaces, or else carry an explicit version increment.

A change to this contract that is not additive with respect to already-derived components and already-invoked tool interfaces MUST carry a stated migration path.
