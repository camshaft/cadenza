# The byte gate absorbed the loop's hand-run WRONG-sweep as native classification — and a slower gate that runs the artifact is the honest one

*2026-07-07*

**What happened.** `component-check` (the byte-level self-hosting gate) landed a new classifier (ask-33): instead
of judging a disagreement by the emitted entry function's SYNTAX (the old "a decline is a bare `unreachable`
entry" proxy), it now RUNS both compiled programs and classifies by what they DO. When native and the
compiler-component both produce `Ok` bytes that differ, the gate runs each and sorts the outcome: component traps
where native yields a value ⇒ DECLINE (honest frontier); both yield EQUAL values ⇒ SOFT (byte-differ, same
behavior); values DIFFER, or the component produces a value where native traps ⇒ DISAGREE (a real miscompile).
The compiler.cdz component that read 65 agree / 124 disagree / 386 decline under the old proxy now reads 97 agree
/ 260 disagree / 25 soft / 195 decline — **and 0 "ran to a wrong value."** The 260 disagrees decompose cleanly:
190 `component=diagnostics` (ask-53's false-rejects) + 70 `native=rejected` comp=ok (ask-30's missing
type-checker), both compiler.cdz-side, no seed miscompile to chase.

**Why.** This is the third time this session a distinction the loop carried BY HAND became a first-class output of
the thing it was measuring — and the cleanest instance of the pattern. For ~15 cycles the loop ran a
"dangerous-bucket sweep" every time the byte gate showed disagreements: take each disagreement, run it through the
FULL oracle (value AND trap), and confirm none had "run to a value where a trap is required" — keeping a
hand-maintained WRONG=0. That sweep existed precisely because the gate's headline `disagree` count was a mixed
pile — byte-fidelity differences, honest declines that happened to trap, and genuine miscompiles all counted the
same — so the raw number was untrustworthy and the loop had to re-derive the only figure that mattered (are there
wrong VALUES?) by hand each cycle. ask-33 makes that re-derivation the gate's native behavior: `disagree` now
means "runs to an observably-wrong result," soft/decline are split out, and the WRONG=0 the loop computed by hand
is now the gate reporting `0` in the disagree-by-value bucket directly. The discipline moved from the observer
into the instrument.

The mechanism is the same one ask-48 named for the compiler's external diagnostics (expose a machine-branchable
rejection/decline/trap KIND) and ask-53 is landing for the compiler's internal check pass (carry the
decline-vs-reject kind as a distinct value): **a distinction repeatedly reconstructed from indirect evidence
should become a first-class output of the producer, after which the consumer reads it instead of re-deriving it.**
Here the "producer" is the gate itself and the "consumer" is the loop; the gate learned to say which kind of
disagreement each case is, so the loop stops running its sweep. Three layers — external diagnostics (ask-48),
internal check (ask-53), measurement gate (ask-33) — the same lesson at each.

The load-bearing implementation detail worth keeping: the honest classifier is SLOWER, because it RUNS the
artifact (both compiled programs, per disagreeing case) rather than inspecting its shape. The old proxy was fast
because reading an entry function's first opcode is cheap; it was also wrong, because the shape of the entry is a
proxy and proxies leak (the ask-33 predecessor missed 77 declines that trapped at runtime rather than emitting a
bare `unreachable`). This restates a rule the loop already knew — "a gate's discriminator is only as good as the
failure shape it models; run the artifact, entry-func shape is a proxy and proxies leak" — now with the cost
attached: the correct gate pays wall-clock to run every candidate to a value or a trap, and that cost is the price
of the classification being real rather than syntactic. A gate that is instant is probably classifying by shape.

**The requirement it drove.** No corpus case — this is the loop's measurement apparatus (the byte gate's
classifier), not a language-value behavior the `(output (: v T))` oracle expresses; the corpus is what the gate
runs, not what the gate IS. The output is this learning and the re-reading of the byte gate's numbers: under the
honest classifier the seed has 0 wrong-value miscompiles (the loop's long-standing WRONG=0, now native), and the
path to gate-green is exactly ask-53 (the check-pass decline/reject split) then ask-30 (the type-checks), each of
which the new gate will show landing as `disagree` drops with soft/decline held. General lesson: **a distinction a
measurement loop re-derives by hand every cycle is a missing output of the measured artifact — including when the
artifact is the GATE itself; move it into the instrument and the loop reads instead of re-derives. And the honest
version of such a classifier runs the artifact rather than inspecting its shape, so it is slower by construction —
an instant gate is a syntactic proxy, and proxies leak.**
