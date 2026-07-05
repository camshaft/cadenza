# The runtime is a name-free value model; naming and rendering are type-directed

*2026-07-05*

**What happened.** Implementing M2 (a runtime value heap so a compound carrying a runtime element
can be produced), the first design gave the shared value-heap runtime a `render` operation: the
runtime would walk a value and return its canonical text, and the host would call it on the
program's result handle. A working prototype composed exactly that way and produced `(tuple 3 1)`.
But when the plan reached records and sums, the design broke on a simple observation: **at run time
the runtime holds no names.** A record is a product of values in slots; a sum is an integer tag with
a payload. The field name `a`, the variant name `Some` — those are source-level facts the runtime
never sees. A `render` living in the runtime therefore *cannot* produce `(record (a 1) (b 2))` or
`(Some 42)`; it has no `a`, no `Some`. Rendering is inherently type-directed, and the only party
that holds the type — with its field and variant names — is the compiler. So `render` was removed
from the runtime entirely. The runtime became a **name-free structural store**: constructors
(`box-int`, `tuple-alloc`, `tuple-set`) and **accessors** (`tag-of`, `arity-of`, `tuple-get`,
`get-int`, `get-bool`). The compiler now emits a **type-directed renderer into the program itself**
— a heap-walk that reads the value through those accessors and assembles the canonical text, baking
every keyword and name (`(tuple `, ` `, `)`, and later each field/variant name) as a static
constant. A compound result crosses the boundary as an ordinary string the program returns.

**Why.** Names are compile-time knowledge. A runtime that stores values efficiently wants the
opposite of names: uniform positional layout, integer tags, no per-type string tables. Pushing
rendering into the runtime forces the runtime to carry a shadow copy of the type system's naming —
field-name tables, variant-name tables, per-type formatting rules — which is exactly the growing,
type-shaped knowledge the compiler already has and the runtime should not. Splitting the concern the
other way is stable in both directions: the runtime's representation can change freely (a record and
a tuple can share one positional product form; Perceus reference counting, size-classed allocation,
and CHAMP/RRB collections can all land) without touching how anything is named, and the naming
vocabulary can grow with the type system (new record shapes, new sums) without touching the runtime.
The seam is the accessor interface plus the opaque handle: the compiler drives the walk, the runtime
answers structural questions, and neither needs the other's private knowledge.

**The requirement it drove.** `spec/contracts/component-abi.md` (FROZEN, version 3): the value-heap
runtime section was refined so the runtime interface *constructs and inspects* rather than
*constructs and renders*; a new §"The Runtime Does Not Name Or Render Values" states the runtime
holds no field or variant names and does not render; §"A Compound Result Is Rendered By
Compiler-Emitted Code" states the observable result is an ordinary string the program produces by
walking the value through the runtime's accessors, so the names a rendering requires stay with the
compiler. `spec/capabilities/capabilities-and-effects.md` and
`spec/contracts/reproducible-derivation.md` were reconciled (the runtime *constructs and inspects*;
its observable contribution is construction, storage, and reclamation, not rendering). This composes
with [the value-heap runtime is a shared component](./2026-07-05-the-value-heap-runtime-is-a-shared-component.md):
that learning fixes *where the runtime lives* (a shared component the host composes); this one fixes
*what it knows* (structure, not names).
