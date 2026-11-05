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
  and decodes to the tree, it begins with the same container header, and a decoder decodes it by the
  same rules as the canonical byte sequence.
- [ast-encoding.md](../contracts/ast-encoding.md), section "The Encoding Is A Bijection With One
  Canonical Byte Form", gains two requirements: a transport encoding may differ from the canonical encoding, and a
  content address or hash is taken over the canonical re-encoding of the decoded tree, never over
  received bytes.

The amendment is additive with respect to every already-stored binary AST: the canonical bytes of every
tree, and so every stored program, content address, and derived component, stay as they are, and the
container version stays `cdzast\x00\x01`. It does not weaken any of the minimum-process rules the
constitution sets for a change to the specification. The reference implementation adds
`codec::encode_as_built`, which writes a tree's two pools as built, for this use.
