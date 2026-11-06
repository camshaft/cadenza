# A transport encoding may list one leaf value twice, since canonicalization merges equal leaves

*2026-11-06*

**What happened.** The compiler's value-form templates push each Int and Bool placeholder as a leaf
of its own, so that two equal placeholders keep a hole each, and the runtime fills the holes and
sends the bytes as built. Two holes filled with one value are one leaf value at two indexes of the
leaf pool. The transport section of [ast-binary-format.md](../contracts/ast-binary-format.md) required
a transport encoding to list each distinct leaf exactly once, and a program that received a pool that
lists one value more than once could not take a content address of the tree. Both rules came from the
reference implementation's canonicalization, which numbered leaves by their place in the pool and not
by value: the canonical re-encoding of such a pool kept both copies, so it was not the tree's
canonical byte sequence, which the encoder rule of the same contract says lists each distinct leaf
exactly once (leaves are deduplicated by value). The runtime's own output therefore broke the
transport rule, and a receiver that follows the contract refused to take its content address.

**Why.** The encoder rule already fixes the canonical byte sequence of every tree with each distinct
leaf listed once, so a leaf pool that lists a value twice and one that lists it once encode the same
tree. Once canonicalization merges equal leaves, as that rule says, the canonical re-encoding of a
received tree lists each distinct leaf once however the sender listed it, and a value's hash is the
same whichever way its bytes were encoded. The rule that a transport encoding lists each leaf once,
and the receiver rule that refused a pool with a repeat, then protect nothing, so the runtime's
templates, a builder that doesn't intern leaves, and any other transport sender may list a value at
two indexes. The receiver rules on Name text outside NFC and on magnitudes that aren't minimal stay,
since canonicalization copies each leaf value as received and doesn't repair them.

**The requirement it drove.**

- [ast-binary-format.md](../contracts/ast-binary-format.md), section "A Transport Encoding Is Any Valid
  Encoding Of The Tree": a transport encoding may list one leaf value at more than one index of its
  leaf pool, instead of the rule that it lists each distinct leaf exactly once; the rule that a
  program doesn't take a content address of a tree whose leaf pool lists one value more than once is
  removed; and the section's first rule no longer points to a rule for the leaf pool.
- The glossary's entry for a transport encoding.

The amendment doesn't change the canonical bytes of any tree: the encoder rule is unchanged, and the
reference implementation now meets it for an arena that holds one value at two leaf ids, where
`codec::encode` wrote the value twice before. `canon::canonicalize` interns leaves by value as well as
by first encounter, and `canon::is_canonical` answers false for such an arena, so `encode` rebuilds
it. The arenas that hold one value at two leaf ids are the compiler's value-form templates, which are
written with `codec::encode_as_built`, so their bytes keep each hole; the compiler's program in memory
after `prelude::push_atom` and the `try` desugaring, which no path writes with `codec::encode`; and a
decoded transport encoding. A search of the repository didn't find a stored program, content address,
golden byte vector or derived component that comes from `codec::encode` of such an arena, so none of
them changes, and the container version stays `cdzast\x00\x01`. `codec::check_addressable` no longer
refuses a repeat, and its `NotAddressable::RepeatedLeaf` variant is removed.
