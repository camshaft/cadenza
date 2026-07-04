# The corpus is the differential surface for growing the compiler

*2026-07-03*

**What happened.** After the ignition bar was cleared with a single derived-and-run component, the
compiler had to grow from compiling one constant to compiling a real sublanguage. The tool that made
that growth tractable was a differential gate: it runs every executable-semantics case the compiler can
compile through BOTH the reference interpreter (the oracle) AND the compiler → component → run path, and
compares observable behavior — `agree` / `todo` (compiler declines) / `skip` (unrealized capability) /
`disagree` (compiled behavior contradicts the oracle, the one failing verdict). Each construct added to
the compiler flips cases from `todo` to `agree`; the invariant that the compiler declines what it cannot
compile (see the companion learning) keeps `disagree` at zero unless a genuine defect appears. This
turned oracle agreement from a one-component demonstration at ignition into a continuous, corpus-wide
measurement: the compiler grew 1 → 16 → 19 agreeing cases with the gate green throughout, and the gate
caught a real encoding defect (an invalid multi-function module) the instant it was introduced.

**Why.** Oracle agreement (constitution §XIV) is the whole safety story of the bootstrap, but a single
derived-and-run component only demonstrates it for one program. A compiler is grown across many small
increments, and each increment can silently break a construct that previously agreed. The
executable-semantics corpus already exists as the behavior oracle's witness set; running the *generated
path* over that same set — not just the interpreter — turns the corpus into a live regression surface
for the compiler. Nothing in the spec had said the generated path is exercised over the corpus (only
that it is exercised at all, on some component), so the practice of using the corpus differentially was
under-specified relative to how load-bearing it had become.

**The requirement it drove.** Added to `spec/capabilities/self-hosting-and-bootstrap.md` §"The Generated
Path Is Exercised Before It Is Trusted" a requirement that the generated path MUST be exercised against
the oracle over every executable-semantics case the generation's compiler compiles, so oracle agreement
is measured across the corpus as the compiler grows rather than on a single derived component. Builds on
conformance-gate.md §"The Behavior Gate" (which gates the interpreter against the corpus) by extending
the same corpus to gate the compiler differentially against the interpreter.
