# A map operation does not accept a map passed as a function parameter

*2026-07-08*

**What happened.** A `Map.*` operation applied to a map that arrives through a FUNCTION PARAMETER is
declined "unsupported dotted-application", while the same operation on a map built inline works. `(def (f
mp) (Map.size mp))` applied to a map declines; `(Map.size (Map.insert Map.empty 1 10))` (inline) works.
The map is the ONLY heap collection with this limitation: `(def (f xs) (List.len xs))`, `(def (f b)
(Bytes.len b))`, and `(def (f s) (String.byte-len s))` all compile and run when the collection is a
parameter. The `maps` capability itself is realized — inline `Map.insert`/`Map.size`/`Map.lookup`/
`Map.swap`/`Map.take`/`Map.remove` all pass on the gate — so this is specifically the map-operation
dispatch not accepting a parameter (unknown-shape) map operand.

**Why it is a gap (an honest decline of a valid program).** A map is an ordinary heap value
(collections-and-text.md #A Map Is Built By Functional Construction; a map is stored and shared like a
list per memory-and-resource-model.md). Passing it through a function boundary and operating on it is
well-typed — every other heap collection does it. Declining is decline-don't-miscompile-safe (no wrong
value), but it blocks the ordinary idiom of threading a map — an environment, a symbol table, a
memo-table — through a function or a recursive accumulator. That idiom is exactly what a self-hosted
compiler is written in (its `List`-accumulator equivalent, `(def (go n acc) … (List.push acc n))`,
already works; the `Map`-accumulator equivalent `(def (go n acc) … (Map.insert acc n v))` declines).

**Root cause (likely) — the Map-operation lowering resolves its map operand's shape and only handles an
inline/known-shape map, not a parameter one.** The seed lowers `Map.op` for a map whose construction it
can see in the same expression (`(Map.insert Map.empty …)` — a known map), but a map arriving as a
parameter has an unknown shape at the operation site, and the `Map.*` dispatch declines it ("unsupported
dotted-application"). The List/Bytes/String operations already accept a parameter operand of their type
(they read the runtime handle without needing the construction site), so the fix is to lower a `Map.*`
operation against a parameter (runtime-handle) map operand the same way — the map operation reads the
runtime persistent-map handle, which a parameter carries as well as an inline value does.

**The lesson (the master pattern — sibling collection asymmetry).** A capability realized for one heap
collection's parameter operands (List, Bytes, String) is not carried to the sibling (Map). This is the
"a mechanism proven on one form is not carried to its sibling" master pattern across the COLLECTION-TYPE
dimension: three of four heap collections accept a parameter operand for their operations; Map does not.
The tell: `(def (f c) (C.len c))` compiles for List/Bytes/String but `(def (f mp) (Map.size mp))` does
not — the operation dispatch branches on the collection type, and the Map branch requires a known-shape
(inline) operand the others don't.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a map operation applies to a map passed
as a function parameter" — `(def (count mp) (Map.size mp))` applied to a two-entry map expects `2`. Gated
`(needs maps)`, which the seed realizes for inline maps, so the case runs; the seed currently DECLINES
(the map-operation dispatch won't take a parameter operand), so the case classifies `todo` (gate stays
GREEN apart from the unrelated c68 map-key FAIL). It will PASS when `Map.*` accepts a parameter map
operand. A generation that does not yet lower a `Map.*` operation on a parameter operand declines rather
than miscompiling.
