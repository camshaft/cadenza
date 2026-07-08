# A degraded representation forces the diagnostic to reconstruct intent from surface cues — an out-of-range literal arrives as a name, not a number

*2026-07-07*

**What happened.** The self-hosted compiler learned to reject an out-of-range integer literal
(`9223372036854775808`, `0xFFFFFFFFFFFFFFFF`) with CDZ0201 (malformed literal), matching native — agree 98 → 100.
The mechanism is the interesting part. The out-of-range literal does NOT reach the compiler as an integer that
overflows a range check: the AST encoder couldn't fit the value in an i64, so it fell back to the generic
`Node::Name` tag — the SAME representation a genuine identifier gets. So at the reader, `9223372036854775808` and
an unbound name `y` are indistinguishable by node TYPE; both are `Node::Name`. The compiler distinguishes them
only by a surface cue — the first character: a digit-led name is a malformed numeric literal (CDZ0201), a
letter-led name is a genuine unbound name (CDZ0101). The fix mirrors the seed's `looks_like_numeric_literal` at
the unbound-name arm. I verified by isolation: the out-of-range case now agrees (CDZ0201), and a genuine unbound
name `y` STILL declines as native's CDZ0101 — the reclassification does not over-fire.

**Why.** This is a distinct shape from "detect a rejection provable from the type" (last cycle's coded-diagnostics
lesson). Here the rejection's PREMISE is present, but the input's INTENT has been erased by the representation.
**When an encoder degrades an ill-formed input to a generic fallback representation, it discards the information
that says what the input was trying to be — so the diagnostic must reconstruct that intent from surface cues that
survived the degradation.** `9223372036854775808` was trying to be a number; the encoder, unable to represent it
as one, stored it as a name, and the fact "this was meant to be a literal" survives ONLY in the token's
digit-led shape. The compiler that wants to emit the RIGHT diagnostic (malformed literal, not unbound name) has to
recover the lost intent by re-reading that surface cue. A compiler that takes the degraded representation at face
value emits the wrong diagnostic — "unbound name: 9223372036854775808" — which is technically a rejection but a
misleading one, blaming the wrong thing.

The load-bearing discipline is the discriminator, and it must be tested BOTH ways — the same cross-product rule as
exhaustiveness. Reclassifying "digit-led name → malformed literal" is only correct if "letter-led name → unbound
name" still holds; a reclassification that swallowed genuine unbound names would trade one wrong diagnostic for
another. The corpus pins both sides — out-of-range literal → CDZ0201 (01-literals) and a real unbound name →
CDZ0101 (02-binding-and-control) — so the byte gate proves the surface-cue split cuts exactly where intended. The
general rule: whenever a diagnostic is chosen by reconstructing intent from a surface cue over a degraded
representation, both the reconstructed case (cue present → recovered diagnostic) and its complement (cue absent →
the representation's face-value diagnostic) must be pinned, or the reconstruction can silently mis-slice.

There is a deeper note for the whole self-hosting effort: the reason the out-of-range literal degrades to a name
at all is that the AST ENCODER is lossy for values it can't represent — an i64-typed literal slot can't hold
2^63, so the encoder spills to a name. That is a property of the interchange representation, not the compiler, and
it means the compiler inherits the reader-boundary reclassification whether it wants to or not: the same
`Node::Name` arm must serve two error families. A representation that could carry an "oversized literal" marker
distinctly would not force this, but given the one it has, mirroring the seed's digit-led heuristic is the honest
fix — and the corpus already classes these with the `_`-prefixed and radix-boundary cases as one "reader-boundary"
family, which is exactly the right grouping.

**The requirement it drove.** No new corpus case — both sides are already pinned (out-of-range → CDZ0201 in
01-literals.sexp:60, genuine unbound name → CDZ0101 in 02-binding-and-control.sexp:121), which is why the byte
gate could show the out-of-range cases flipping to agree while the unbound-name case correctly stays a disagree
(ask-30's remaining CDZ0101 frontier). The output is this learning and the verified accounting (agree 98→100,
WRONG=0, 0 false-rejects; the digit-led reclassification confirmed not to over-fire on `y`). General lesson: **a
lossy encoder degrades an ill-formed input to a generic fallback, erasing what it was trying to be; the diagnostic
must reconstruct that intent from surviving surface cues (a digit-led prefix), and the reconstruction is a
discriminator that must be pinned BOTH ways — cue-present recovers the true diagnostic, cue-absent keeps the
representation's face-value one — or it silently mis-slices one rejection family into another.**
