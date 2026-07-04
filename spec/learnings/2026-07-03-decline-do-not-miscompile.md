# A compiler grown incrementally declines what it cannot compile — it never miscompiles

*2026-07-03*

**What happened.** The Cadenza-authored compiler is being grown one construct at a time, measured
by a differential gate that runs every executable-semantics case through both the reference
interpreter (the oracle) and the compiler → component → run path, comparing observable behavior. The
gate is only meaningful if "the compiler cannot do this yet" and "the compiler does this wrong" are
kept strictly apart. The discipline that made the climb safe: every emitter path that meets a construct
or a value it does not yet handle — a non-integer operand where it expects an integer, a constant
outside the single-byte LEB128 range, an unbound name, a computed callee, a float `=` whose IEEE
semantics differ from the language's canonical-byte equality — forces the derivation to TRAP (decline)
rather than emit bytes. A declined derivation is classified `todo` (the honest backlog); only a
component that ran and disagreed is a `disagree` (the one failing verdict). With this invariant the gate
ran green at each step (1 → 16 → 19 agreeing cases, 0 disagree) even though the compiler compiled only a
fraction of the language, and it still caught a real defect the moment one appeared: a multi-function
core module that omitted the `\0asm` preamble produced invalid bytes, surfaced immediately as `disagree`.

**Why.** A compiler under construction compiles a strict sublanguage of the language it is written in.
Without a rule, an unhandled construct has two tempting wrong outcomes: emit plausible-but-wrong bytes
(a silent divergence from the oracle), or silently skip it (masking a divergence as an absence). Either
destroys the gate's meaning — a green gate would no longer imply "every compiled program agrees." The
resolution makes the compiler's partiality *observable and safe*: it declines exactly the programs it
cannot yet compile, and everything it does compile it compiles in agreement. Growing the compiler is
then monotone — flipping `todo` to `agree` — and can never regress correctness silently.

**The requirement it drove.** Added `spec/capabilities/self-hosting-and-bootstrap.md` §"An Unsupported
Construct Is Declined, Not Miscompiled" (two requirements: a generation whose compiler does not yet
compile a construct MUST decline to derive rather than emit a divergent component; and the declined set
MUST be observably distinct from the divergent set, so incremental growth is measured by agreement, not
by masking). This sharpens the existing oracle-agreement obligation (§"A Compiled Program Agrees With
The Oracle", constitution §XIV) for a compiler built incrementally, and is what lets the differential
gate serve as the promotion ratchet.
