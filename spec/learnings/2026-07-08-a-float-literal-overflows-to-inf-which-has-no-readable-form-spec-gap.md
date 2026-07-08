# A float literal overflows to inf, which has no readable form (spec gap)

*2026-07-08*

**What happened (probe, not pinned).** Adversarial probing of the numeric-literal lexer found that a
float literal whose magnitude exceeds Float64's range silently produces an infinity: `1e400` → `inf`,
`-1e400` → `-inf`, `1e309` → `inf`. The renderer emits the text `inf` / `-inf` for these values, but the
Cadenza reader cannot read any of them back: `inf` → "unbound name: inf", `-inf` → "unbound name: -inf",
`Infinity` → "unsupported bare form/constructor". So an overflowing float literal produces a value whose
canonical rendered form does not round-trip through the language's own reader.

**Why it is left UNPINNED (a spec gap, not a compiler break with a clear oracle).** The numeric-model
spec defines *integer* overflow exhaustively (#Overflow Is Defined — a value or a trap, never
undefined), and the deterministic-value-form contract defines the canonical form for finite floats,
negative zero, and NaN (all-NaN → one canonical form). But NOTHING in the spec addresses:
- whether a float LITERAL whose magnitude exceeds Float64 is accepted (→ inf) or rejected;
- whether infinity is a value the language admits at all;
- if it is, what its canonical readable form is.
Three defensible resolutions each contradict the others, and each would be a distinct oracle:
1. **Reject the literal** — an out-of-range float literal is a malformed literal (CDZ0001/0201), parallel
   to the out-of-range integer literal `9223372036854775808` which IS rejected "integer literal out of
   the Int64 range". This is the most consistent with the integer-literal rule.
2. **Admit inf with a readable form** — define a literal spelling (`inf`/`-inf`/`Infinity`) so the render
   round-trips, and specify inf's structural equality and canonical byte form.
3. **Admit inf but only as a computed value, never a literal** — then `1e400` must still be rejected as a
   literal (it is the literal surface, not a computation).
Picking any one here would invent a spec position. Per the standing rule "probe UNSPECIFIED → learning,
do NOT invent an oracle," this is recorded as a spec gap rather than a corpus case.

**The concrete inconsistency worth resolving.** The deterministic-value-form contract (line 65) requires
"exactly one canonical byte form of a value" and the value-oracle gate independently re-reads a rendered
float (the `float_output_round_trips` oracle, see [[float-render-saturates-and-gate-blindspot]]). Rust's
`"inf".parse::<f64>()` DOES yield `f64::INFINITY`, so that oracle would accept `inf` at the f64 level —
but the Cadenza READER rejects `inf`. So the render is round-trippable by the host parser but not by the
language's own reader: the rendered form is not a program the language accepts. Whatever resolution the
spec takes, the reader and renderer must agree on infinity's form (or agree it cannot arise).

**Recommendation.** Resolve in the numeric-model / literals spec: most likely (1) — reject an
out-of-range float literal as malformed, exactly as an out-of-range integer literal is rejected — since
the language provides no way to write `inf` and treating a literal as silently saturating to a
non-writable value is the float analogue of the integer-saturation blindspot already closed. If infinity
is instead intended to be a value, its literal spelling and canonical form must be specified so the
renderer round-trips through the reader. Until the spec takes a position, no corpus case is added.

**Related:** [[float-render-saturates-and-gate-blindspot]] (the whole-float saturation defect, fixed; the
gate's independent float round-trip oracle came from it), the deterministic-value-form contract's
canonical-form requirement, and the integer-literal-out-of-range rejection (`9223372036854775808` →
"integer literal out of the Int64 range") which is the finite-integer analogue this float case lacks.
