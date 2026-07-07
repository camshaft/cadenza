# The reader decodes CBOR as the input dual of the output spine — built on the byte primitives that already work

*2026-07-07*

**What happened.** The compiler-in-Cadenza spike started its **reader** — the last major piece before
self-hosting — and it took the shape the byte-emitting work predicted: an *input dual* of the LEB128
output spine ([[2026-07-06-the-compilers-byte-emitting-spine-needs-a-known-answer-corpus-case]]). The
compiler's input is the canonical binary AST as deterministic CBOR (`[version, prelude, root]`, where an
atom is a CBOR scalar and an application is a CBOR array `[head-index, …children]`). The new primitives
decode a CBOR item's head: `cbor-major` (top 3 bits of the initial byte, `(>> byte 5)`), `cbor-info`
(low 5 bits, `(& byte 31)`), `cbor-arg` (the info directly when < 24, else a following 1/2/4/8-byte
**big-endian** argument assembled most-significant-byte-first via `be-bytes`), and `cbor-head-len` (how
many bytes the head occupies). All are built on `byte-at` = `(match (Bytes.at b i) ((Some x) x) (None 0))`
— the runtime `Bytes.at`-plus-Option-match idiom that landed a cycle earlier — plus the bit ops (`>>`,
`&`) and arithmetic already proven for the output side. Verified against the real bytes of a CBOR head:
`major` of `0x83` is 4 (array), `cbor-arg` of `18 2A` is 42 (one-byte argument), and a 2-byte big-endian
assembly of `01 2C` is 300.

Notably, the reader is being authored **around** the still-open reader-gate facets, not blocked on them.
The built-in `Option`-across-a-boundary decline for `String.from-bytes` and a bare `(Some 42)` is still
open ([[2026-07-07-the-reader-gate-is-being-closed-accessor-by-accessor]], SPEC-BACKLOG item 12), so the
reader decodes **raw bytes** with `Bytes.at` + bit ops (which do cross boundaries now) rather than
routing head/length integers through `String.from-bytes`. The symbol *table* — where a head index
becomes a name string — is where `String.from-bytes` becomes unavoidable, so that facet is the reader's
next real dependency; the head/structure decode does not need it.

**Why.** Two durable points. First, the reader falling out as the *dual* of the writer is the
resolved-IR architecture paying off symmetrically: the same byte primitives (`Bytes.at`, `Bytes.concat`,
`>>`, `&`, `*`, `+`) that compose upward into the LEB128 encoder compose downward into the CBOR decoder,
so the input and output halves of a self-hosted compiler are built from one small, proven vocabulary —
there is no separate "reader runtime." That is why the reader could start the moment `Bytes.at`-across-a-
boundary landed: everything else it needs was already there. Second, the reader's *decode step* is a
composition, and — exactly as on the output side — verifying each primitive (`Bytes.at`, `>>`, `&`)
individually does not verify they compose to the right decoded number. A single slip (wrong shift for
the major type, wrong mask for the info, wrong place value in the big-endian assembly) yields a
plausible-but-wrong integer that mis-indexes the symbol table or mis-sizes a child array — a miscompile
of the *input*, silently. So the composition needs a known-answer corpus case, the input mirror of the
LEB128 known-answer case, not just the byte primitives it is built from.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"a CBOR head decodes its major
type and big-endian argument from the input bytes"* — pins the reader's head-decode spine to a known
answer: against `19 01 2C` (CBOR uint, info 25 = a 2-byte argument), `major` is 0 and `arg` is
`0x012C = 300`, returned as `(tuple 0 300)` so both halves — the major-type shift and the big-endian
multi-byte argument assembly — are checked in one case. It composes `Bytes.at`+match, `>>`, `&`, `*`, `+`
into the decode step exactly as the LEB128 case composes them into the encode step, and it PASSES (the
reader primitives run on the real bytes today). This is the input dual of the byte-emitting-spine case
and closes the same gap on the input side: the reader's decode is now a durable gate obligation, not an
ephemeral probe. The reader's remaining dependency is the symbol-table decode (`String.from-bytes`
through a boundary — SPEC-BACKLOG item 12), which its raw-byte head decode deliberately does not touch;
until that lands the reader can decode structure but not yet resolve a head index to a name string.
