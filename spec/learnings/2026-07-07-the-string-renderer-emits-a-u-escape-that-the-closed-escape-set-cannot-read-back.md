# The string renderer emits a `\u{…}` escape the closed escape set cannot read back

*2026-07-07*

**What happened.** Adversarial probing of string rendering found a round-trip failure for any
string containing a non-printable Unicode scalar. A string holding U+0007 (BEL) renders as
`"a\u{7}b"`. But the reader's escape set is closed to exactly `\n \t \r \\ \"` — `\u` is not
among them — so reading `"a\u{7}b"` back does NOT yield the original: its byte-length comes back
6 (the literal characters `a \ u { 7 } b` minus whatever the unknown `\u` collapses to), where
the original BEL string is 3 bytes, and `(= "a\u{7}b" "a<BEL>b")` is `false`. The renderer
escapes every non-printable scalar this way — control chars (U+0007, U+001F, U+007F), the
zero-width space U+200B, and the maximal scalar U+10FFFF all render `\u{…}`. Printable scalars
(é, 😀, U+FFFD) render verbatim and DO round-trip.

**Why it is a break.** 13-strings.sexp §"a returned runtime string with a multi-byte scalar
renders the scalar verbatim" pins that the renderer's escaping must be such that "a rendered
string reads back to the same value." collections-and-text.md §"A String Literal's Escapes Are A
Closed Set" fixes the reader's escapes to five, none numeric. The two compose to a hard
constraint: the renderer may only emit escapes the reader recognizes. `\u{…}` is not one, so a
string with a control scalar renders to a form that is not a fixed point of read∘render — the
canonical-form round-trip the corpus requires is violated.

**Where the reference behavior misleads.** 13-strings.sexp:456 blesses the renderer as matching
the const path's Rust `{:?}`, "which prints printable Unicode literally." That is right for
*printable* scalars, but `{:?}` also escapes *non-printable* scalars as `\u{…}` — and that half
of `{:?}` is incompatible with Cadenza's closed escape set. The reference was pinned on the
printable case and silently carried its non-printable behavior, which the language cannot read
back. The fix: a non-printable scalar must render either verbatim (its raw UTF-8 bytes, as
printable scalars do — the only round-trippable option given no numeric escape) or via one of the
five recognized escapes where applicable (`\n \t \r`), never `\u{…}`.

**Why the gate does not catch it — a string round-trip blindspot.** The behavior gate compares
the compiled program's rendered output against the corpus's `(output (: <string> String))`,
where the expected side is produced by the SAME renderer. So a case whose input builds a
BEL-string and whose expected output is the raw-BEL string PASSES: both sides render `\u{7}` and
agree. This is the exact shape of the float-saturation blindspot
(2026-07-05-float-render-saturates-and-the-gate-cannot-see-it): expected and observed route
through one non-injective/non-round-trippable renderer. Floats got an independent guard —
`corpus.rs::float_output_round_trips` re-parses the rendered text and checks it equals the
recorded f64. Strings have no analogue, so a renderer that emits an unreadable escape passes
unseen. The durable gate fix is a string round-trip check: the rendered string text, fed back
through the reader, must equal the value that was rendered — computed by different code than the
renderer, so it catches a renderer whose output the reader cannot parse.

**The lesson.** A round-trip requirement ("renders back to the same value") is only enforced if
the check runs the *inverse* (read the rendered form), not if it re-renders the expected value.
A renderer and reader specified in different sections (one open to `\u{…}` via a borrowed
reference, the other closed to five escapes) can each look correct alone while failing to
compose; only exercising render-then-read exposes the seam. When a spec pins a round-trip, the
gate for it must apply the actual inverse, or the two halves drift.

**Status.** No corpus case added that FAILs — the value-oracle gate structurally cannot catch
this (same renderer both sides), so a `(output …)` case passes. Recorded as this learning; the
actionable fixes are (1) the renderer stops emitting `\u{…}` for non-printable scalars (render
verbatim), and (2) the gate gains a string round-trip check mirroring `float_output_round_trips`.
Native seed. Related: the closed-escape-set is already pinned (01-literals.sexp §"an unrecognized
string escape is rejected", `(needs strict-escapes)`); once the reader enforces it, `read("\u{7}")`
becomes a hard rejection rather than a mangled value — still not a round-trip.
