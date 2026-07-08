# A keyed collection needs no serialization seam — tag-free structural hashing and comparison suffice

*2026-07-06*

**What happened.** Designing the value-heap runtime's persistent map (a CHAMP — a Compressed
Hash-Array Mapped Prefix tree) and its sibling set forced the first hard question the earlier
collections did not raise: a map must **hash** a key (to index the hash trie) and **compare** keys (to
deduplicate, to detect "already present," and to resolve hash collisions), but the runtime is
deliberately **tag-free** — it stores structure and bytes and holds no type identity, so on its face it
cannot hash or compare a value. Three seam designs were on the table: (1) the compiler serializes each
key to its canonical byte form and ships that `Bytes` blob across the boundary with every operation;
(2) the compiler passes a hash *and* an equality function pointer the runtime calls back on collision;
(3) the runtime hashes and compares keys itself by a **direct structural walk of the node graph** —
`raw` bytes plus recursively-compared child `handles` — with keys crossing the seam as plain handles,
exactly as they already do to the map stub.

The design took (3). Serialization (1) allocates a canonical-bytes buffer on every insert and lookup —
the very O(n) allocation a hash map exists to avoid — and forces the compiler to emit a
canonical-serializer it does not otherwise need. The equality upcall (2) inverts the import direction
(the runtime calling into program code), reintroducing reentrancy across the component boundary and
tangling reference-count and fuel accounting. The structural walk (3) allocates nothing, emits nothing
new on the compiler side, and needs no upcall: the runtime derives a key's hash by walking
`(raw, handles)` and resolves a collision by structurally comparing two node graphs.

**Why.** The reason a *tag-free* structural comparison is nonetheless a correct *value* comparison — the
crux of this learning — rests on two facts that hold together:

1. **Keys within one map are homogeneous.** The compiler guarantees a single key type per map; a
   cross-type key comparison is a compile-time rejection. So any two keys the runtime ever compares are
   the same static type. The node-level ambiguity that would otherwise sink a tag-free compare — a boxed
   integer of eight bytes, an eight-character string, and an eight-byte leaf are all
   `handles = [], raw = <8 bytes>` — is therefore **harmless**: whichever type the bytes really are,
   *both* operands are that same type, so byte-for-byte node equality is value equality. The tag the
   runtime lacks is precisely the tag it never needs, because the compiler already established it before
   the key crossed the seam. This is the tag-free principle
   ([the runtime is tag-free, rendering walks a static shape](./2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md))
   applied to equality and hashing themselves: like rendering, they look only at structure and bytes,
   and the compiler's static knowledge supplies everything else.

2. **Every value form has a canonical node representation — with exactly one exception.** Scalars are
   byte-canonical; strings are their bytes; tuples, records, lists, and sums are structural with no
   slack; the persistent vector's radix trie is canonical for a given length and contents; and a CHAMP
   is canonical *by construction* — a given set of entries has one and only one node layout regardless
   of insertion order (that canonicality is the "C" that distinguishes a CHAMP from a plain hash-array
   mapped trie). The single exception is the Bytes rope
   ([a Bytes rope defers materialization](./2026-07-05-a-bytes-rope-defers-materialization-behind-the-same-observable-bytes.md)),
   which has many node shapes for one logical byte string. Because canonical representations are unique,
   two structurally-equal values have identical node graphs, so a structural walk is automatically
   consistent with the frozen rule that equal values have identical canonical bytes — and automatically
   **order-independent** for maps and sets used as keys, with no special commutative combine. The one
   non-canonical form carries a one-line obligation instead: a rope key is compacted to a flat leaf
   before it is used as a key, which the runtime already has an operation for.

The deeper finding is that **structural equality and hashing do not need a type system at run time,
because a canonical representation already encodes the identity a tag would carry**. The same
immutability-makes-representation-invisible argument that licensed the rope's deferred materialization
here licenses tag-free key comparison: if the representation is canonical, its bytes *are* the value's
identity, and comparing bytes is comparing values. This collapses "hash the key" and "compare the keys"
into one mechanism the runtime already has the material for — no serialization, no upcall, no tag — and
it is why a keyed collection turns out to cost the seam nothing beyond the operations that name what a
map does.

It also names a **tripwire** the whole keyed-collection story now depends on: *every value form must be
canonical, modulo a compaction obligation for the non-canonical ones.* A future RRB-tree vector
(a relaxed radix balancing that trades canonicality for O(log n) concat/split) would be the first
representation to violate this, and if an RRB vector were ever used as a map key its structural compare
would wrongly distinguish two equal vectors of different internal balance. The invariant is therefore
not "the CHAMP is correct" but "no key type has a non-canonical representation without a normalize-on-use
rule" — a constraint any later collection must be checked against.

**The requirement it drove.** None new — and, as with the persistent vector and the rope, that is the
finding: the design is a *realization* of requirements already written, at the altitude that let them
absorb it. `spec/contracts/deterministic-value-form.md` §"A Value Has One Canonical Byte Form" (equal
values have identical canonical bytes; unequal values have distinct ones) is exactly what makes a
structural walk a sound equality; §"Ordering Of Aggregate Members Is Fixed" is satisfied because a
canonical trie has one member order derived from the members; `spec/capabilities/collections-and-text.md`
map and set semantics (order-independent equality, total membership) fall out of canonicality; and
`spec/contracts/component-abi.md`'s tag-free contract holds because a map is hashed, compared, and
reclaimed as structure and bytes whose type is compile-time knowledge the runtime does not hold. The one
thing the design *adds to the record* is the canonicality tripwire above, which is where a later
non-canonical representation would have to be reconciled — the same place the RRB relaxation the
persistent-vector learning anticipated would first bite.
