# Capability — Cross-Component Interop

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This document
> defines **cross-component interop**: the surface by which a separately-derived Cadenza component binds
> to another Cadenza component's exported definitions and calls across a live component boundary, so that
> a program built from independently-compiled components composes without being merged into one compilation
> unit. Requirements realize [Core Principle IV](../../constitution.md) and
> [Core Principle VI](../../constitution.md) and trace to [overview §6](../overview.md) and
> [overview §8](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying exactly
> one obligation, under a stable heading. This capability states the surface's invariants; the concrete
> import surface, the exchanged-value transport, and the well-known runtime `value` type are pinned at the
> declared-default location and by the component-abi.md frozen contract.

## Purpose And Scope

Package linking (modules-and-namespaces.md) composes many source files into one component before the
pipeline runs; nothing there crosses a component boundary. Cross-component interop is the other axis: a
component that is already an emitted artifact exports definitions, and a second component — compiled with
no access to the first's source — imports those definitions and calls them across a live boundary at run
time. This capability fixes the invariants of that binding: that a cross-component import names a peer's
public export explicitly, that the exchanged signature is concrete, that a value crossing between composed
Cadenza components is the same value on both sides, and that the authority a component holds is not widened
by importing a peer. It does not restate the boundary calling convention or the exchanged-value transport,
which the component ABI governs (component-abi.md §Cross-Component Value Exchange), nor the canonical value
form, which the value-form contract governs; it is the *program-level binding surface*, distinct from both.

## A Cross-Component Import Names A Peer's Export

### A Component Reaches A Peer Only Through An Explicit Cross-Component Import

A definition another component exports MUST enter a component's scope only through an explicit
cross-component import that names it, so that a component's dependence on a peer is legible from its
imports and no peer definition is ambient (mirrors modules-and-namespaces.md §Imports Are Explicit).

A cross-component import MUST introduce no names beyond those it names, so that importing a peer's
interface binds exactly the definitions listed and not a peer's whole surface.

### An Import Binds Only A Definition The Peer Makes Public

A cross-component import of a name a peer does not export MUST be rejected at compile time, so that the
importing component can bind only the peer's declared public surface and never a definition the peer keeps
private (mirrors modules-and-namespaces.md §Visibility Is Explicit).

The public surface a peer offers for cross-component import MUST be the peer's exported definitions, so
that one visibility mechanism — the export list — governs both what a component publishes at its boundary
and what a peer may import.

### A Cross-Component Import Is Distinct From A Package Import

A cross-component import MUST bind a definition across a component boundary that persists at run time,
rather than merge the peer's source into the importing component's compilation unit, so that the peer
remains a separate artifact and the two components are linked as components rather than as source
(modules-and-namespaces.md; component-abi.md §Cross-Component Value Exchange).

## The Exchanged Interface Is Concrete

### An Exchanged Signature Is Monomorphic

A cross-component imported or exported signature MUST be monomorphic, so that the boundary names concrete
types and no generic definition crosses (component-abi.md §Generics Do Not Cross The Boundary).

A component that imports a peer's definition MUST bind it at a concrete instantiation the peer emitted as
an export, rather than request an instantiation the peer did not emit, so that the exchanged interface is
fixed at the peer's derivation and cross-component specialization is not required of the peer.

## A Value Crosses Between Composed Components As The Same Value

### An Exchanged Value Is Structurally The Same On Both Sides

A value that crosses between two composed Cadenza components MUST be, on the receiving side, structurally
equal to the value the sending side produced, so that crossing a component boundary preserves a value
exactly and interop introduces no lossy conversion (component-abi.md §Cross-Component Value Exchange).

The crossing of a value between composed Cadenza components MUST NOT be a re-serialization of the value
into a second encoding, so that a value exchanged by composed components is the runtime's own value rather
than a distinct interchange form (value-interchange.md §Serialized Bytes Are The Canonical Value Form —
interchange remains the surface a program invokes to turn a value into bytes, separate from a live
crossing).

### An Exchanged Value's Meaning Is Bounded By The Shared Runtime

A value a component receives from a peer MUST be meaningful only while the two components share the one
runtime instance the value belongs to, so that cross-component value exchange is a property of a
composition against a shared runtime and not a durable transport of a value between unrelated runs
(component-abi.md §A Cross-Component Handle Is Meaningful Only In The Shared Runtime Instance).

## Importing A Peer Does Not Widen Authority

### A Cross-Component Import Grants No Host Authority

Importing a peer's definition MUST NOT add to the importing component's manifest any host capability the
peer's definition reaches, so that a component's declared authority is its own and calling a peer does not
silently inherit the peer's authority (host-interface-binding.md §Imports Mirror The Manifest Exactly;
capabilities-and-effects.md).

A component that must itself reach a host operation a peer it calls performs MUST declare that capability
in its own manifest, so that every host authority a component exercises — directly or by delegating through
a peer it composes — is legible from that component's own manifest rather than acquired implicitly across
the import.

## Rejections

### A Cross-Component Import Of An Unknown Or Non-Exported Name Is Rejected

A cross-component import that names a peer the composition does not provide, or a definition the named peer
does not export, MUST be rejected at compile time rather than resolve to nothing, so that an unsatisfiable
cross-component dependency is a detected error and never a silent absence (reference-compiler.md §Outcomes
Are Ordered By Safety).

### A Not-Yet-Supported Cross-Component Shape Declines Rather Than Miscompiles

A cross-component import or export of a shape the compiler cannot yet emit as a well-formed boundary — a
generic signature, a value shape without a boundary representation, or a value the shared-runtime transport
does not yet carry — MUST decline to produce an artifact rather than emit a component whose boundary is
ill-formed, so that an unsupported interop shape is a refused compilation and never a miscompile
(reference-compiler.md §Outcomes Are Ordered By Safety; component-abi.md).

## The Realization Is A Declared-Default Decision

### The Cross-Component Import Surface Is Pinned At The Declared-Default Location

The concrete surface by which a component names a peer and imports its definitions MUST be pinned at the
declared-default location, so that two builds agree on the surface a program may rely on to bind a peer
component.

### The Exchanged-Value Transport Is Governed By The Component ABI

The representation by which a value crosses between composed Cadenza components MUST be the shared-runtime
handle transport the component ABI fixes (component-abi.md §Cross-Component Value Exchange), so that this
capability names the binding surface while the byte-level crossing stays the frozen contract's concern and
is not re-pinned here.

## Open Decisions This Capability Leaves To The Declared-Default Location

### The Points A Choice Must Resolve

A choice realizing this capability MUST resolve how a component names a peer in a cross-component import
(how a peer's identity is written and how the composition supplies the artifact that identity resolves to),
so that a cross-component import is directed by an explicit peer identity rather than by ambient discovery.

A choice realizing this capability MUST resolve how a composition of components against one shared runtime
is expressed and delivered to the host, so that which components share a runtime instance is a stated
property of the composition rather than an implicit default.

A choice realizing this capability MAY resolve cross-component interop with a non-Cadenza component, but
such interop MUST carry values across that boundary by the external marshaling representation the component
ABI fixes rather than by the shared-runtime handle transport, so that a shared-runtime handle stays an
internal Cadenza-to-Cadenza optimization and never escapes to a component that does not share the runtime.
