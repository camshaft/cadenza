# The compiler's byte-emitting spine needs a known-answer corpus case, not just verified primitives

*2026-07-06*

**What happened.** The compiler-in-Cadenza spike repeatedly reports its LEB128 encoders as
"verified byte-correct" — `uleb 624485 → E5 8E 26`, the canonical multibyte value from the LEB128
specification — and the seed does produce exactly that (`b"\xe5\x8e&"`). But that verification lived
only in the gitignored spike (`implementation/`): an ephemeral `emit` probe the agent ran by hand,
not a gate obligation. The corpus meanwhile pinned every *ingredient* of the encoder in isolation —
`(< n 128)`, `(& n 127)`, `(| byte 128)`, `(>> n 7)`, `Int.to-byte`, `Bytes.concat`, and a
recursive-by-count Bytes builder (`rep n → b"XXXX"`) — each in its own case, each green. What no case
did was pin the **composition**: the actual recursive unsigned-LEB128 encoder run to a known-answer
multibyte output. So the language's most load-bearing emit path — the recursion that produces every
section length, vector count, and operand in a wasm module — was witnessed only in a scratch buffer
that vanishes when the spike directory is cleaned.

**Why.** This is a specific instance of the corpus's structural blind spot
([[2026-07-06-authoring-the-compiler-surfaces-gaps-a-corpus-grown-from-a-floor-misses]]): a
floor-outward corpus is excellent at covering each primitive it decides to exercise and blind to the
*compositions* a real program builds from them. Verifying `&`, `|`, `>>`, and `Bytes.concat`
separately does not verify that they compose into the right bytes — a single-primitive slip (a wrong
mask, an off-by-one shift, a dropped continuation bit, a base/recursive-arm swap) is invisible to the
per-primitive cases yet corrupts the encoder, and the encoder's output is *bytes a wasm runtime must
accept*, so a slip is a miscompiled component, not a wrong number. The known-answer value is the
tighter check precisely because it is over-determined: `624485` was chosen by the LEB128 spec so that
all three 7-bit groups are non-trivial and the continuation bit matters on two of them, so any
primitive error changes the observed bytes. The deeper lesson, beyond LEB128: **when a spike reports
"verified byte-correct" via an ephemeral probe, that verification is not durable until it is a corpus
case** — the two-compilers gate only protects what the corpus pins, and a hand-run `emit` is exactly
the kind of parallel, drifting verification the one-executable-semantics discipline exists to prevent
([[2026-07-02-parallel-semantics-drifted]]).

**The requirement it drove.** Two conformance cases in `10-bytes.sexp`, tagged `bytes` (so the seed
runs them): *"an unsigned LEB128 encoder emits the known-answer multibyte encoding"* — the recursive
encoder composing all six primitives, `(uleb 624485) → b"\xe5\x8e&"` (bytes `E5 8E 26`) — and its
base-case companion *"an unsigned LEB128 encoder emits a single byte below the continuation
threshold"*, `(uleb 100) → b"d"`, which exercises the `(< n 128)` terminator arm in isolation so a
regression in either arm is localized. Both PASS today (the seed already emits these bytes), turning
an ephemeral spike claim into a permanent gate obligation on the compiler's emit spine. The
methodological rule stands on its own and applies to every future "verified byte-correct" claim the
spike makes (section framing, the signed-LEB128 encoder, the component envelope): **pin the
composition to a known answer in the corpus, do not leave it in a probe.**
