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

The explicit visibility rule MUST govern only a definition's reachability from outside its module, not its reachability from within.

A definition MUST remain visible to the other definitions in its own module regardless of whether it is made visible outside, so that a module's members are mutually visible and a private helper stays reachable by its siblings.

### A Type's Handle And Its Constructors Are Independently Visible

A sum type's handle — the name that denotes the type itself — and its constructors MUST be independently exportable, so that a module can publish a type for other modules to name and hold values of without publishing the way to construct or take those values apart.

A module that makes a type's handle visible without making a constructor visible MUST render that constructor unreachable outside the module — a construction or a match through that constructor in another module MUST be a compile-time rejection carrying the machine-readable code for a withheld constructor — so that a value of such a type is built and deconstructed outside the module only through the functions the module exports, and an invariant the module's constructor establishes cannot be bypassed by another module fabricating a value directly.

A module MUST be able to make every constructor of a type visible in one act that also makes the type's handle visible, so that publishing a type together with its whole constructor set does not require enumerating the constructors one by one and does not drift as the constructor set changes.

## A Module's Role Bounds Its Effect Row

### A Module's Role Fixes Its Mandatory Effect Profile

A module MAY declare a role, and a declared role MUST fix the module's mandatory effect profile as a bound on its [escaping effect row](capabilities-and-effects.md) — for example, a fold role's mandatory profile is the empty effect row, purity — so that a module's role constrains which effects its body may reach.

### A Role Violation Is A Compile-Time Rejection

The compiler MUST reject a module whose body reaches an effect its declared role forbids, emitting a machine-readable diagnostic, so that a module reaching outside its role's effect profile fails to compile rather than reaching an effect at activation.

### A Module's Role Compliance Is Certified

The compiler MUST emit a machine-readable certificate that a module satisfies its declared role's effect profile, so that an activation review can trust the module's compliance — a fold role's purity, for instance — without re-deriving it from the module's body.

## Module Directives

### A Module Directive Is Drawn From A Fixed Set

A module MAY carry directives that instruct the compiler how to compile it, and every such directive's key MUST be drawn from a set fixed by this specification rather than invented per program, so that a directive has one fixed meaning across generations.

A module directive's arguments MUST match the shape the directive's key defines, and a directive whose arguments do not MUST be rejected with a machine-readable diagnostic.

### An Unrecognized Module Directive Is Rejected

A module directive whose key is not one the fixed set defines MUST be rejected at compile time with a machine-readable diagnostic, rather than ignored, so that a directive can neither silently change a program's meaning on a toolchain that understands it while being dropped by one that does not, nor silently fail to take effect.

### A Meaning-Changing Directive Is Part Of The Canonical Form

A module directive that changes the meaning of the module's definitions MUST be carried in the module's canonical form, so that the module's meaning is determined by its canonical form alone and does not depend on a compilation option outside it.

### A Module Directive Is Compile-Time Only

A module directive MUST be resolved at compile time and MUST NOT introduce any runtime representation of its own into the emitted component, so that a directive affects how the module is compiled without adding runtime cost or crossing the boundary.

### A Contract Module Declares Its Identity

A module MAY declare that it is a contract — a named input-to-output shape another component targets — by providing a self-describing `descriptor` the compiler evaluates, rather than through dedicated module directives, so that the contract's identity is derived from ordinary evaluated code and passed explicitly, not carried in a separate directive vocabulary. Compiling and evaluating the module's contract descriptor MUST yield the contract's identity — its name together with its input and output types — as a value, so a tool MAY derive the contract's content-addressed identity by executing the descriptor, without a registry outside the module and without a bespoke directive form. Because the identity is the value the descriptor produces from the module's own declarations, it is fixed by the module's code alone.

## Dependency Resolution

### Dependencies Resolve By Content Address

A dependency MUST be identified by a content hash so that resolving it yields the same source on every build.

Dependency resolution MUST NOT depend on a mutable version range whose resolution could differ between builds of the same program.

### Resolution Introduces No Authority

The set of capabilities a program requires MUST NOT be enlarged by dependency resolution beyond the union its entrypoints delegate to the host, so that pulling in a dependency that declares or performs an effect grants no authority unless an entrypoint delegates that effect (capabilities-and-effects.md §The Program Manifest Is The Union Of Its Entrypoints' Delegations).

## Composition

### Cyclic Module Dependencies Are Rejected

A set of modules whose import relationships form a cycle MUST be rejected at compile time.

### Initialization Order Is Deterministic

The order in which modules' top-level definitions are initialized MUST be a deterministic function of the source.

The initialization order of modules MUST follow their import dependencies, so that a module is initialized after the modules it imports.

### Colliding Imported Names Are Rejected

Importing two definitions under the same name into one scope MUST be a compile-time error rather than resolved by an implicit precedence.
