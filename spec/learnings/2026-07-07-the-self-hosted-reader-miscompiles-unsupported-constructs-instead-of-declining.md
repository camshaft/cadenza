# The self-hosted reader miscompiles unsupported constructs instead of declining — and correcting my own wrong call

*2026-07-07*

**What happened.** Running `compiler.cdz` over the whole corpus (via the interim byte-patching harness)
produced **0 mine-declines** — the compiler *never* cleanly declines an unsupported construct. It always
emits *some* valid component, which for anything outside its subset is a **silent miscompile** (a valid
component computing the wrong value). The root causes, verified:
- **CBOR float** (`0xfb`, major 7, info 27): `read-node`'s major-7 branch assumes a boolean
  (`0xF5`=true=arg 21, `0xF4`=false=arg 20), so a float head — same major 7, arg 27 ≠ 21 — decodes to
  `NBool 0`, and the program's `run()` returns **`false`**. A float literal `3.14` compiles to a
  component that yields `false`.
- **strings / records / tuples / bytes-ops / host calls**: the reader has no node shape for them, so
  they fall through to `NInt` / an `NPrim`-of-`"?"` and emit an i64 stub or a spurious value.
- **unbound name-reference** (a later facet): `read-node`'s tag branch decodes a name to `(NLocal
  (ienv-pos env idx 0))`, and `ienv-pos` returns **-1** for a name not in the environment — used
  directly as a local slot with no bounds check, so an unbound name decodes to `NLocal -1` → `KLocal
  -1` → an invalid `local.get` (uleb of -1 is a huge index), a miscompile rather than a decline.

This is a **reject-don't-miscompile violation inside the Cadenza-authored compiler itself**: the very
discipline the spec mandates for a generation ("a construct it does not yet cover MUST decline, not run
to a wrong value") is violated by the compiler's *reader*, which decodes an unrecognized atom kind to a
wrong-but-valid node rather than declining.

**This entry also corrects my own prior call, on the record.** Last cycle I looked at the same harness
output — ~147 disagree, 0 mine-declines, "mine" sizes clustered at 88–102 B — and concluded the
disagreements were a *harness artifact*: that the byte-patching wasn't reaching the compiler's decode
path, producing a degenerate stub the harness mis-scored. That was **wrong**. A direct probe this cycle
(feeding a CBOR float head through the reader's actual major-type dispatch) shows the bytes *do* reach
the decode path and are *miscompiled* — the ~88 B "stub" is a *wrong decode* (a float read as `false`),
not a non-decode. So the 147 disagree are (mostly) real miscompiles, and `0 mine-declines` is the true
alarming signal, not noise. I made exactly the error this spike keeps teaching against: I reasoned from
a plausible surface reading (size clustering ⇒ "not decoding") instead of *probing the actual behavior*
(what does the reader do with a float head?). The rule I've written four times over — **probe the real
thing, don't infer from a proxy** — I then failed to apply to a harness's summary table. The correction
is the lesson: a suspicious aggregate (0 declines) deserves a direct probe of one instance before it is
explained away.

**Why it matters.** `0 mine-declines` is not merely incomplete coverage — it is *unsafe* coverage. A
compiler that emits a wrong-but-valid component for an unsupported construct is worse than one that
declines: the decline is visible and honest (the corpus scores it todo), the miscompile is silent and
passes a naive "did it produce a component?" check. For a self-hosted compiler this is acute — when it
eventually compiles *itself*, a silently-miscompiled construct in its own source produces a subtly-wrong
compiler, not a clean failure. The fix is structural and matches what the reader *already does* for one
case: an **unrecognized operator head** correctly routes `PUnknown → KError → unreachable` (an honest
decline). The atom/literal decode must do the same — a major-7 value that is not a known boolean, and
any major/shape the reader has no node for, must produce `KError` (decline), not a defaulted `NBool`/
`NInt`. Decline-don't-miscompile is not only the seed's obligation; the Cadenza-authored compiler must
hold it too, and its reader is where it currently leaks.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"a CBOR simple value that is not
a known boolean is not decoded as a boolean"* — pins the specific miscompile at the seed level (which
the compiler's reader mirrors): decoding a CBOR major-7 head by checking *only* `arg == 21` (true) wrongly
classifies every other simple value (false's `20`, a float's `27`, `null`'s `22`) — so a bool decoder
must check the value is *actually* `0xF4`/`0xF5` (arg 20 or 21) and treat other major-7 heads as
not-a-boolean, not default them to false. It records the discriminating oracle (a non-bool major-7 arg
must not read as `0`/`false`). Because the reader's node set is `compiler.cdz`-internal (not a seed
behavior the corpus can drive directly), the broader "the reader declines rather than miscompiles
unsupported constructs" requirement is recorded as **SPEC-BACKLOG item 23** (route the reader's
unrecognized atom kinds to `KError`, mirroring the `PUnknown` head path), with the harness's `mine-declines`
count as its acceptance signal: it should rise from 0 to the count of unsupported constructs as the
reader learns to decline them. The corrected methodological note — *a suspicious aggregate deserves a
direct probe before an explanation* — is the durable rule this cycle adds to the "probe the real thing"
family.
