# A byte-string literal is one escape table, shared by the reader and three renderers

*2026-07-06*

**What happened.** The Bytes value form had one legible-to-a-machine spelling and an illegible display:
you wrote `(Bytes.of (list 137 80 78 71))` and it rendered back the same wall of decimals, hiding that
those four bytes are the PNG magic `\x89PNG`. We adopted the `bytes` crate's `Debug` form — `b"…"` with
printable-ASCII passthrough and the escape set `\n \r \t \\ \" \0 \xNN` — as *both* the reader spelling
and the canonical display, so a byte sequence reads and prints as a byte string and the two round-trip.
The change was small in each place but touched an unusually wide fan of surfaces, and the interesting
part was how many of them had to agree *byte-for-byte* on the same escape logic: the reader
(`b"…"` → `(Bytes.of (list …))`, pure sugar, no new node kind — the same move `a.b` → `(. a b)` and the
unrealized `#"…"` → `(Symbol.of …)`), the compile-time constant fold (`bytes_literal_text`), the
emitted-wasm type-directed renderer (a hand-emitted `if`-cascade over each byte), and the corpus oracle
(which now recognizes the `(Bytes.of (list …))` tree — however it was spelled — and renders it as `b"…"`
so the differential gate compares the same form on both sides). Four producers of the same text, plus
the reader that must invert it.

**Why.** The escape *order* is load-bearing and is exactly where independent reimplementations drift: `\`
and `"` have byte values inside the printable range `0x20..=0x7e`, so a renderer that tests the printable
passthrough first emits a raw `"` and produces `b"a"b"` — unreadable and non-round-tripping. Every one of
the four producers has to test the named escapes *before* the printable arm, and the reader has to accept
exactly the set they emit. The differential gate is what makes this safe rather than aspirational: it
compiles each corpus case to wasm, runs it, and compares the emitted-wasm renderer's output against the
oracle's — so a drift between the hand-written wasm cascade and the Rust `escape_byte` is a gate failure,
not a silent divergence discovered later by a human reading a byte dump. The two same-language renderers
(fold + oracle) were collapsed onto one shared `escape_byte` helper so they *cannot* drift; the wasm
renderer is a separate implementation by necessity (it emits opcodes, not calls a Rust fn), and the gate
is precisely the check that the separate implementation agrees. That is the same two-implementations-
of-one-thing discipline the whole project runs on (native vs. wasm compiler; oracle vs. compiled
program), applied at the granularity of one escape table.

The other quiet lesson was scope containment. Because `b"…"` is reader-and-printer sugar over the
existing value form, the wasm renderer could be rewritten *inline* — a per-byte `if`-cascade emitting
raw bytes or `\xNN`, using three scratch locals — with **no new fixed runtime helper**, so
`RT_FIXED_FUNCS` and `RT_FUNC_BASE` stayed at 3/35 and the frozen component envelope did not move (the
envelope re-derivation dance the rope and vector work needed was avoided entirely). The tell that this
was safe: the change lives wholly in the pure `cdz-compiler` core, compiled identically for the native
and wasm targets, so `component-check` agreeing on all 36 Bytes programs was expected, not lucky. It
also composes cleanly with the `(bin …)` binary form: `b"…"` is a whole-value literal (matches by
equality, splices into a `(bytes …)` segment — `(bin (bytes b"\x89PNG") …)` reads identically to the
explicit form), where `(bin …)` is a structured segment application — orthogonal surfaces that both
denote an ordinary Bytes, no grammar collision because the `b` sigil is a literal only directly before a
`"`.

**The requirement it drove.** No frozen contract changed. It pins a new isolated decision,
`options/byte-string-literal/` (default `b-string`), a sibling of `char-literal-syntax` and
`symbol-interning`'s `#"…"`: the reader spelling and canonical display of a Bytes value are a
reader-and-printer concern outside the compiler's trusted path (ast-encoding.md §"Parsing And Printing
Are Not In The Compiler's Trusted Path"), and the display form and reader form MUST be inverses so a
rendered byte sequence reads back to an equal value — the round-trip the constitution requires over the
canonical form (homoiconic-decoupled-display.md). `spec/semantics/10-bytes.sexp` now records `b"…"` as
the observable output form and adds round-trip cases pinning `b"…" == (Bytes.of (list …))` for printable,
escaped, and empty sequences, plus a build-render-read-back case. Recorded so the next reader-sugar /
display-form pair (the realized `#"…"` symbol literal, a `Char` literal, a future hex-blob form) starts
from one shared escape/inverse table checked by the differential gate, rather than N hand-copied cascades
that agree until the day an ordering edge case proves they don't. Composes with
[a Bytes rope defers materialization behind the same observable bytes](./2026-07-05-a-bytes-rope-defers-materialization-behind-the-same-observable-bytes.md)
(the runtime Bytes value this displays) and the homoiconic-decoupled-display choice (display is a
projection off the one canonical tree, which `b"…"` and `(Bytes.of …)` share).
