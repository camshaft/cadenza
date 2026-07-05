# The runtime is tag-free; the renderer walks a static shape, not a runtime tag

*2026-07-05*

**What happened.** M2 Phase C set out to widen the value-heap runtime from tuples to every compound
(records, lists, sums, bytes, strings, maps). The Phase-B runtime was already *name-free* (a record
was a positional product; field names lived in the compiler), and its renderer was a single FIXED
wasm body that dispatched on a per-object **type tag** the runtime stored (`tag-of` → INT/BOOL/
TUPLE/…). Extending that design to records and sums forced a question: the fixed renderer cannot
produce `(record (a 1))` or `(Some 42)` — it has no field or variant names — so records/sums *demand*
a type-directed, compiler-emitted renderer regardless. And once the renderer is type-directed it
already knows the exact shape at every step, so it never needs to ask the runtime "what is this?".
The operator put it directly: *"does the runtime really need tagging? the types are already known
statically. seems like the tagging is just runtime overhead."* Correct. Cadenza has **no type
erasure** — generics monomorphize — so the compiler knows the static type at every use site;
`get-int` is only ever emitted where the type says Int. The per-object type tag is fully redundant
with static knowledge.

So the tag was **removed**. `tag-of` left the interface; the shared `mod tag` constant was deleted
from both the compiler and the runtime. What the runtime still stores is not a type tag but two
pieces of *genuine runtime data*: an array's element **count** (`arr-len`, needed only for a
runtime-length list) and a sum's variant **discriminant** (`sum-disc` — *which* variant, a runtime
choice, a small per-sum index the compiler assigns and `switch`es on). One positional array shape
(`arr-alloc`/`arr-set`/`arr-get`/`arr-len`) now backs tuple **and** record **and** list — they
differ only in the static `Shape` the compiler-emitted renderer walks, which bakes `(tuple `,
`(list `, or `(record (k ` accordingly. A sum is a `(discriminant, payload)` (`sum-new`/`sum-disc`/
`sum-payload`): the compiler assigns each variant its index in declaration order as the
discriminant, `gen_runtime_sum` builds `sum-new(disc, box(payload))`, and the renderer switches on
the runtime `sum-disc` to write the correct variant name — the one runtime datum a sum carries
being *which* variant it is, never the sum's type. The compiler infers `main`'s result `Shape` (a recursive
structural type carrying field/variant names), emits one small render function per distinct
compound shape (each `(handle, cursor) -> cursor` walking the value through the accessors), and the
`run` export assembles the canonical text. Result: records, lists, nested and mixed compounds all
render correctly; the behavior gate rose from 326 to 369 passing with the pre-existing FAIL set
unchanged, IGNITION byte-identical, and the wasm compiler component agreeing with native on 412
programs.

**Why.** A type tag on every heap object is pure overhead when there is no erasure: a store word per
allocation and a load+branch per render/inspect step, encoding information the compiler already has
in hand. Worse, a *rendering* tag would force the runtime to grow a shadow copy of the type system —
it would need not just tags but field-name and variant-name tables to render `(record (a 1))` — which
is exactly the type-shaped knowledge that should stay with the compiler. Splitting it the tag-free
way is stable in both directions: the runtime's representation can change freely (size classes,
Perceus reference counting, CHAMP/RRB collections) with zero emitted-byte impact, because nothing
outside it observes a tag; and the naming/rendering vocabulary grows with the type system (new
records, new sums) without touching the runtime. The seam narrows to the accessor signatures plus
two conventions — arrays are 0-indexed positional, a sum discriminant is an opaque `u32` stored
verbatim — with no shared tag constant to keep in sync. This is the name-free insight
([the runtime is name-free](./2026-07-05-the-runtime-is-name-free-rendering-is-type-directed.md))
pushed one level deeper: that learning fixed *what names the runtime lacks*; this one removes the
*type identity* too, leaving a runtime that holds structure and data but no types at all.

**The requirement it drove.** `spec/contracts/component-abi.md` (FROZEN): the value-heap runtime is
not only name-free but **tag-free** — it stores no per-object type identity, because the program's
renderer walks a statically-known shape and never dispatches on a runtime type. The runtime's
observable contract is construction, positional/by-discriminant inspection, and reclamation of
values whose *type* is compile-time knowledge the runtime does not hold. This composes with
[the value-heap runtime is a shared component](./2026-07-05-the-value-heap-runtime-is-a-shared-component.md)
(where the runtime lives) and [the runtime is name-free](./2026-07-05-the-runtime-is-name-free-rendering-is-type-directed.md)
(what names it lacks). The emission technique that realizes it — a fixed component envelope around a
compiler-built core module, now with a type-directed renderer emitted per program instead of a fixed
render body — extends
[emitting a component with an import is a fixed envelope](./2026-07-05-emitting-a-component-with-an-import-is-a-fixed-envelope.md).
