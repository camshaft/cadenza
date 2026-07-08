# A persistent collection fits the tagless heap with no new node field and no new RC machinery

*2026-07-05*

**What happened.** The value-heap runtime gained its first *persistent collection* — a persistent
vector (an immutable, structurally-shared, growable sequence), realized as a 32-way radix trie
(the Bagwell/Clojure representation). The notable result is how little the existing runtime had to
change to hold it: **no new `Node` field, no new discriminant, and not one line of new
reference-counting code.** A vector is built entirely from the same tagless node the rest of the
heap already uses — `Node { rc, handles, raw }` — where the trie's interior and leaf nodes carry
their children in `handles` and a small header node carries the element count and the root-level
radix shift in `raw`. Because the children live in `handles`, three things the runtime already does
apply to the trie unchanged:

1. **Structural sharing is just a refcount above one.** Path-copying an update allocates one new node
   per level along a single root-to-leaf path and shares every off-path subtree by `dup`-ing it into
   the new version. A shared subtree is a node whose `rc` is now greater than one — exactly the
   condition the heap was already built to represent. Two versions of a vector that share all but one
   path are two roots pointing into a common DAG of nodes.
2. **The existing iterative free cascade reclaims a whole trie.** The runtime's `drop` already drains
   a node's `handles` onto an explicit worklist and frees transitively, stack-safely. A multi-level
   trie is just a deeper such structure; dropping a version reclaims exactly the nodes no surviving
   version still references, and a shared subtree survives until its last owner drops. No new
   reclamation logic was written — the persistent vector's memory management fell out of the RC
   discipline that was already there.
3. **Rendering is unchanged.** A persistent vector renders exactly as a list does: the type-directed
   renderer reads its length and indexes it over the range. The renderer needs the element *shape*
   and nothing else — no runtime tag distinguishes a vector's nodes from a tuple's.

The ownership contract the vector's operations expose is the same one every other constructor and
accessor already follows: the constructors (`push`, `update`) **consume** their input vector and the
element and produce a new owned version — the old version is untouched, which is what makes the
structure persistent — while the indexed read **borrows**, returning an element the vector still owns.
A caller that keeps both an old and a new version `dup`s the input before the constructing call,
identical to the duplicate-binder rule for any other shared value.

**Why.** This is the tag-free design paying out exactly as it predicted it would. The earlier
learning [the runtime is tag-free](./2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md)
argued that removing per-object type identity leaves "a runtime that holds structure and data but no
types at all," and claimed as a direct consequence that "the runtime's representation can change
freely (size classes, Perceus reference counting, **CHAMP/RRB collections**) with zero emitted-byte
impact, because nothing outside it observes a tag." A persistent vector is the first test of that
claim, and it holds: a new collection is a new *arrangement* of the one tagless node, and the parts of
the runtime that don't care about arrangement — allocation, reference counting, reclamation — did not
have to learn about it. The reason a rope-of-shared-slices for byte buffers and a radix trie for
sequences can both be added the same cheap way is that both are the same move — overloading the
meaning of a node's children within a family of operations the compiler only ever calls on a value of
the matching static type. Tagless dispatch keeps the families from colliding: the compiler calls the
vector operations only on a value whose static type is a vector, so within those operations the node's
children mean "trie branches or elements," never anything else, with no runtime check.

The deeper reason this is cheap is the acyclicity invariant from
[an immutable heap is acyclic, so reference counting is complete](./2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete.md).
A persistent structure is the paradigm case that learning anticipated: many versions sharing a common
substructure, "copy the spine and share the rest," reclaimed precisely when the last version drops.
Path-copying adds only forward references from newer nodes to older, already-existing ones, so it
introduces no cycle; reference counting therefore stays both sound and complete over a heap of shared
persistent versions, needing no tracing or cycle collector. The persistent vector does not stress the
RC discipline — it is the workload the discipline was chosen for.

One thing the persistent case does *not* inherit from the byte-rope is flatten-on-read: a rope may
materialize a shared node in place on first full read because a flattened buffer is byte-identical, but
a persistent vector's whole contract is that shared versions are immutable, so no read ever mutates a
shared node. The shared discipline is the same; the in-place-mutation shortcut is specific to the
rope's collapse-to-a-leaf and does not generalize to a structure whose sharing is between live
versions.

**The requirement it drove.** None new — and that is the finding. The persistent vector is a
*realization* of requirements the memory model already states, and building it confirmed they were
specified at the right level of abstraction. `spec/capabilities/memory-and-resource-model.md`
#Sharing Is Not Observable already requires that "a value that shares another value's storage and a
value that copies it MUST be indistinguishable by every operation … including equality, length,
indexing, and the value's canonical byte form" — which is exactly the persistent vector's obligation:
two versions sharing all but one path must be indistinguishable from two independent copies. The same
file's #Retained Storage Is Accounted For What It Holds Live requirement — that shared storage stays
accounted against the resource measure rather than hidden, and that a value can be derived whose
storage is independent — is precisely the structural-sharing-plus-compaction behavior a persistent
collection embodies. And the runtime side stays within the frozen `spec/contracts/component-abi.md`
tag-free contract: a persistent vector is stored, inspected, and reclaimed as structure and data whose
*type* is compile-time knowledge the runtime does not hold, so the collection's arrival cost the ABI
nothing but an append of representation-agnostic operations. The lesson to carry to the next
collection (a CHAMP map/set, an RRB tree with relaxed nodes for logarithmic concatenation and split)
is that the design is stable in the predicted direction: the representation may grow arbitrarily
inside the runtime, and the seam to the emitted program grows only by appending operations whose
signatures name *what* a collection does and never *how* it is stored.
