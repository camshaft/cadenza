# A Bytes rope defers materialization behind the same observable bytes — sharing and deferral are one mechanism

*2026-07-05*

**What happened.** The value-heap runtime's byte-buffer type gained a *rope* representation: a Bytes
value is now a tree of **concat** and **slice** nodes bottoming out in shared byte **leaves**, so
concatenating two buffers or taking a sub-range allocates one node and copies **no bytes** — the copy
is deferred until something reads the buffer out. The motivation is concrete and load-bearing for
self-hosting: the Cadenza-authored compiler assembles a WebAssembly module by concatenating encoded
sections, and with a flat byte buffer that is an O(n²) copy cascade as the module grows (each concat
recopies everything so far). A rope makes concat and slice O(1) and turns the whole build-then-emit
into O(n). This was identified as the single highest-leverage runtime change for the self-hosting
compiler's build time.

Like the persistent vector before it
([a persistent collection fits the tagless heap with no new machinery](./2026-07-05-persistent-collections-fit-the-tagless-heap-with-no-new-machinery.md)),
the rope needed **no new node field**: the three shapes are distinguished by the count of a node's
children, which the tagless node already carries — a leaf has none (its bytes are the raw payload,
exactly the old representation), a slice has one child (its parent buffer) plus a stored offset and
length, and a concat has two children (its left and right) plus a stored total length. The existing
byte accessors are byte-unchanged on the leaf case, so every pre-existing byte-buffer behavior stayed
green through the leaf path; the reference-counted free cascade reclaims a rope with no new code
because its children live where every other compound's children live; and a byte leaf shared between
two ropes survives until its last owner drops, which is just a refcount above one.

The one genuinely new mechanism is **flatten-on-access**, and it is where the interesting lesson is. A
naive tree-walking read of the logical byte at an index is O(tree depth); the compiler's emit step
reads the whole buffer in a loop, so on a section-by-section (deeply right-leaning) concat chain that
loop is O(n²) again — the exact cost the rope was meant to kill. The fix is that the first full read of
a rope node **materializes it into a leaf in place, once**: it fills a buffer with the logical bytes by
an iterative walk, installs them as the node's payload, and releases the children the node no longer
references. Every subsequent read is then O(1) per byte. The single flatten is O(n); the emit loop is
O(n) total.

**Why.** Flattening a *shared* node in place — mutating a value that other references can see — sounds
like exactly the aliasing hazard the memory model forbids, and the reason it is instead *licensed* is
the crux of this learning: **the flattened buffer is byte-identical to the rope it replaces**, so no
operation the language defines — length, indexed read, structural equality, canonical byte form — can
tell the difference before and after. It is not a mutation of a *value*; it is a change of
*representation* underneath an unchanged value. The memory model already anticipated this precise move.
Its #Sharing Is Not Observable requirement says a value that shares another's storage and a value that
copies it must be indistinguishable by every operation, and — the sentence that turns out to be doing
the real work — it explicitly permits that "a value the compiler derives by combining or narrowing
existing values MAY defer the work of materializing its contents until an operation observes them,
provided the deferral is not observable and is a deterministic function of the source." Concat is
combining; slice is narrowing; flatten-on-first-read is the deferred materialization; and because the
result bytes are fixed by the source, the deferral is deterministic. The rope is that clause made real.

So the deeper finding is that **structural sharing and deferred materialization are the same
mechanism seen from two sides**, both authorized by the same "not observable" test. A persistent
collection shares storage between versions and never materializes a copy; a rope shares storage between
a buffer and its slices/concatenations and materializes lazily on read. Both are safe for the identical
reason — immutability makes representation invisible to meaning — and both are, again, the tag-free
design paying out: the runtime holds structure and data but no type identity, so a byte buffer's
internal arrangement can become a rope with zero change to what the emitted program observes across the
seam. This is the second collection added the same cheap way, which is the evidence that the pattern
generalizes rather than being a one-off.

One retention consequence is worth stating because it is a feature that looks like a bug: a small slice
of a large buffer keeps the **whole** large buffer alive, because the slice references it. That is
correct — the memory model's #Retained Storage Is Accounted For What It Holds Live requires that shared
storage stay accounted for what it actually holds live, not hidden — and the release valve is a
compaction operation that materializes a slice's own bytes into independent storage and drops the
parent, which the flatten path already implements for free. A resource measure over ropes must
therefore count *retained* storage (the pinned parent), not merely the logical length of the slice.

**The requirement it drove.** None new — and, as with the persistent vector, that is the finding: the
rope is a *realization* of requirements the memory model already states, and building it confirmed they
were written at the right altitude. `spec/capabilities/memory-and-resource-model.md` #Sharing Is Not
Observable (including its deferred-materialization clause) is precisely the license for
flatten-on-access; #Retained Storage Is Accounted For What It Holds Live is precisely the
slice-pins-its-parent accounting and the compaction escape hatch; and the runtime stays within the
frozen `spec/contracts/component-abi.md` tag-free contract, since a rope is stored, read, and reclaimed
as structure and data whose type is compile-time knowledge the runtime does not hold. The rope's
arrival cost the ABI only an append of three representation-agnostic operations whose signatures name
*what* they do to Bytes and never *how* the bytes are stored — so a later balancing or tail change,
like the persistent vector's future RRB relaxation, is invisible across the seam.
