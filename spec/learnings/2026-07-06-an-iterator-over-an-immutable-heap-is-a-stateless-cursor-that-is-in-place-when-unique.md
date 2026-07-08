# An iterator over an immutable heap is a stateless cursor — in-place when unique, forkable when shared

*2026-07-06*

**What happened.** The persistent map's iteration surface raised a question the array and vector had
sidestepped. A flat array and a radix vector both offer cheap indexed access (`len` + `get`), so
"iterate" is just a loop over an index — no iterator object at all. A hash trie has no cheap i-th
element, so a map genuinely needs an iterator, and the first sketch had it return a materialized array
of entries. That array is an O(n) allocation on every traversal, and it defeats the reason to iterate
lazily in the first place: a chain of transforms (`map` then `filter` then `fold`) over a materialized
array is O(n·m) in the intermediate structures, where a fused pull-iterator is O(n) with nothing
allocated in between. The design replaced the entries array with a **cursor**: a bounded descent-stack
value that yields one entry at a time.

Two design decisions fell out, and both are the interesting part. The first is that the cursor is
**stateless** — `iter-next(cursor)` returns a *new* cursor rather than mutating the one passed in — even
though iteration is the textbook example of stateful computation and the whole heap is immutable. The
second is that "stateless" is expressed as **two projections rather than a pair-returning step**:
`iter-key`/`iter-val` read the current entry (the head), and `iter-next` returns the advanced cursor
(the tail), so no operation has to allocate a `(value, next-cursor)` pair on every step. This is exactly
the shape of a lazy cons-stream — OCaml's `Seq.t = unit -> Nil | Cons of 'a * 'a Seq.t`, the ML form of
the stream-fusion source that Rust's `Iterator` is the imperative face of — split into its head and
tail projections so it fits a runtime where every operation returns a single opaque handle.

**Why.** A stateless cursor over an immutable structure sounds strictly more expensive than a mutable
one — a fresh cursor per step instead of an advancing pointer — and the resolution is the crux: **at a
reference count of one, a stateless cursor and a mutable cursor are physically the same object.** The
in-place-reuse discipline the runtime already ships (Perceus reset/reuse — see
[an immutable heap is acyclic, so reference counting is complete](./2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete.md))
makes `iter-next` on a *unique* cursor refit that cursor's own cells in place: physically a mutation,
semantically a new value, zero steady-state allocation. The ordinary iteration loop, where one owner
threads the cursor through, is therefore exactly as cheap as a mutable pointer — the statelessness costs
nothing.

And when the cursor is *not* unique, statelessness pays for a capability a mutable iterator cannot offer
at all. If a caller duplicates the cursor (raising its count above one) and then advances one copy,
`iter-next` cannot reuse the shared cells, so it path-copies — leaving the other copy frozen at its
position. That is a forkable iterator — peekable, tee, backtracking — and it arrives for free, by the
*identical* rule that gives every persistent structure its sharing: unique means reuse in place, shared
means copy. A mutable iterator would need a bespoke clone operation and could never share the tail. So
the two behaviors an iterator wants — a cheap linear walk and a cheap fork — are not two mechanisms but
one, and it is the same `count == 1 ? reuse : copy` predicate the whole memory model already turns on.

The reconciliation of "stateful iteration" with "immutable heap" is thus that **a cursor is not a
value** in the sense the heap is built around. It is linear, ephemeral, and borrowing: it is never
persisted, never rendered, never compared, never sent across the boundary; it holds *borrowed* views
into a structure another owner keeps alive, and an entry that escapes the loop is duplicated at that
point exactly as an array element read is. Because it is ephemeral iteration state and not a
participating value, mutating it in place when unique is not a violation of immutability — it is the
frame-limited-reuse identity applied to the one kind of object for which "the old version is dead the
instant the new one exists" is true by construction. The stateless contract is what *lets* the runtime
make that optimization safely and hands the fork case over for free; a mutable contract would forfeit
the fork and gain nothing, since the unique case is already in-place either way.

A corollary shaped the operation surface: because the compiler knows every collection's static type, it
dispatches iteration **statically** — it emits the map's advance operation for a map cursor and the
set's for a set cursor directly, with no runtime branch on a cursor kind. This is the `impl Iterator`
(monomorphized, no vtable) rather than `dyn Iterator` (tagged, indirect) choice, and it is the right
default for the same reason tag-free rendering is: the static type already decided, so paying for a
runtime discriminant on every step would be paying twice. A tagged cursor becomes worth its cost only
for genuinely heterogeneous iteration behind an existential, which can be added later as its own
operation without disturbing the static path. The door-open property — that the map's internal
representation can change with no recompile — comes not from sharing one advance operation across
collections but from the operation's *signature* being frozen while its *body* is private, which holds
whether the operation is shared or per-collection.

**The requirement it drove.** None new; the cursor is a realization within existing contracts. It
stays within `spec/contracts/component-abi.md`'s tag-free, name-free seam (a cursor is structure the
compiler emits typed operations against, statically dispatched); it is licensed by
`spec/capabilities/memory-and-resource-model.md` #Sharing Is Not Observable and the frame-limited-reuse
discipline (the in-place advance is unobservable because a unique cursor's old state is dead the instant
the new state exists, and a shared cursor is copied); and it satisfies
`spec/capabilities/collections-and-text.md`'s deterministic-iteration semantics without materializing
an intermediate collection. The finding worth carrying forward is the general one: **an iterator over
an immutable, reference-counted structure should be a stateless cursor read through head/tail
projections, because the reuse-when-unique rule makes the functional form free in the loop and the
copy-when-shared rule makes it forkable — the same rule, not two.** This is the uniform pull protocol
every future collection's iteration should take, and the point at which stream fusion becomes possible
in the language: the runtime supplies a non-allocating source, and the compiler fuses the combinators
above it.
