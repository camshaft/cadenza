# Capability — Modules And Namespaces

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines modules, namespaces, imports, visibility, and dependency resolution. Requirements
> realize [Core Principle II](../../constitution.md), [Core Principle IV](../../constitution.md), and
> [Core Principle I](../../constitution.md) and trace to [overview §2](../overview.md) and
> [overview §7](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes how a program is composed from modules: how names are namespaced, how a name
from another module enters scope, what determines whether a definition is visible outside its module,
and how dependencies are resolved. It requires that imports be explicit and that dependency
resolution be reproducible, so that composing a program neither introduces hidden names nor makes
"the same program" depend on a version-range search that could resolve differently over time.

## Imports

### Imports Are Explicit

A name defined in another module MUST be brought into scope only by an explicit import.

An import MUST NOT introduce names into scope beyond those it explicitly names or the module it explicitly binds.

## Visibility

### Visibility Is Explicit

Whether a definition is visible outside its module MUST be determined by an explicit rule fixed by this specification, not by its position in the source.

A definition that is not made visible MUST NOT be importable by another module.

## Dependency Resolution

### Dependencies Resolve By Content Address

A dependency MUST be identified by a content hash so that resolving it yields the same source on every build.

Dependency resolution MUST NOT depend on a mutable version range whose resolution could differ between builds of the same program.

### Resolution Introduces No Authority

The set of capabilities a program requires MUST NOT be enlarged by dependency resolution beyond the union its modules declare.
