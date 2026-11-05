# A message between programs needs a valid encoding of the tree, not its canonical byte form

*2026-11-05*

**What happened.** The binary AST became the data exchange format for messages between programs, as
well as the stored, hashed, and compiled form of a program, so that programs exchange typed values
without hand-written encoders. Until this amendment, the encoder rule of
[ast-binary-format.md](../contracts/ast-binary-format.md) ("Equal Trees Encode To Identical Bytes")
required every encoder to produce the one canonical byte sequence of a tree. Every message therefore
paid for the canonical form: in the reference implementation, `codec::encode` runs
`canon::canonicalize`, which walks the whole tree to check whether it is already in canonical order and
rebuilds it where it is not. At the operator's request, the canonical-form requirement was relaxed for
messages, where latency matters more than one byte form per tree.

**Why.** The canonical form exists so that equal trees produce identical bytes, which storage, content
addressing, and compilation depend on. A decoder never depended on it: the same contract already
required a decoder to accept the pools in the order the bytes present them, and to enforce only
referential integrity, tree-ness, and exact consumption. The encoder rule therefore bound more encoders
than the property it protects needs. A second container version for messages was considered and
refused: the binary AST is the one data exchange format, a dictionary transport version of this format
was removed once already, every reader would have to implement both versions, and a peer that reads
only one of them could neither store nor hash what the other sends.

**The requirement it drove.**

- [ast-binary-format.md](../contracts/ast-binary-format.md), section "Equal Trees Encode To Identical Bytes":
  the encoder rule now binds the stored, hashed, and compiled forms of a tree.
- [ast-binary-format.md](../contracts/ast-binary-format.md), section "A Transport Encoding Is Any Valid
  Encoding Of The Tree" (new): a transport encoding is any byte sequence a conforming decoder accepts
  and decodes to the tree, it lists each distinct leaf once and begins with the same container header,
  and a decoder decodes it by the same rules as the canonical byte sequence.
- [ast-encoding.md](../contracts/ast-encoding.md), section "The Encoding Is A Bijection With One
  Canonical Byte Form", gains two requirements: a transport encoding may differ from the canonical encoding, and a
  content address or hash is taken over the canonical re-encoding of the decoded tree, never over
  received bytes.

The amendment is additive with respect to every already-stored binary AST: the canonical bytes of every
tree, and so every stored program, content address, and derived component, stay as they are, and the
container version stays `cdzast\x00\x01`. It does not weaken any of the minimum-process rules the
constitution sets for a change to the specification. The reference implementation adds
`codec::encode_as_built`, which writes a tree's two pools as built, for this use. A transport
encoding lists each distinct leaf once because the reference implementation's canonicalization keeps
leaves apart by their place in the pool, not by value, so the re-encoding of a leaf pool that lists one value twice keeps both copies.

**Who keeps one leaf value at two leaf ids.** `Builder::leaf_unique` is the `Builder` call that
pushes a leaf without interning it. A search of the repository for it, and for tables keyed by a
leaf id, found three callers, and whether each depends on canonicalization keeping such leaves apart:

- The compiler's value-form templates (rcdzc, `lower/value_form.rs`): each Int and Bool placeholder
  is a `leaf_unique` leaf, so that equal placeholders keep a hole each, and each hole's offset is
  measured on the arena's leaf pool as built. They depended on it: they were written with
  `codec::encode`, which canonicalizes under `std`, so each offset held only while canonicalization
  left the pool as built, and twice a template built out of canonical order moved its holes. They
  are now written with `codec::encode_as_built`, by one function that writes the bytes and resolves
  the holes from the same pool, so the holes hold whatever canonicalization does. Without `std`,
  `encode` already wrote every template as built, and a template built in canonical order keeps its
  bytes. A runtime `Qty` template whose unit is a product, a power or a quotient is built out of
  canonical order, since the unit's heads are built after their operands: under `std` its bytes now
  list the unit's leaves and nodes as built, and decode to the same tree.
- `cadenza-ast`'s codec test of one value at two leaf ids: it compares two canonical re-encodings
  of one tree, which agree whether canonicalization keeps the two leaves apart or merges them, and
  it doesn't assert how many leaves the canonical bytes list. It doesn't depend on it.
- `cadenza-ast-serde`'s test of a received value that lists one leaf value at two ids: it reads a
  typed value from the bytes, which doesn't keep leaf ids, and doesn't canonicalize the decoded
  tree. It doesn't depend on it.

Two design notes name `leaf_unique`, and no other code calls it. A search for `LeafId` outside
`cadenza-ast` found one table keyed by a leaf id, the one that resolves the template holes, and it
reads the arena as built. Two compiler paths push a leaf without interning it and without
`leaf_unique`: `prelude::push_atom`, and the `try` desugaring, which pushes one name at two leaf
ids. A change that makes canonicalization merge equal leaves, as the encoder rule above says the
canonical byte sequence does, reads them first.
