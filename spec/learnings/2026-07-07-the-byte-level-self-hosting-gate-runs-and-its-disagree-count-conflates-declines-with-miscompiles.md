# The byte-level self-hosting gate runs — and its "disagree" count conflates honest declines with real miscompiles

*2026-07-07*

**What happened.** The last wiring step landed: the seed gained `compile-run <compiler.cdz> --emit-component
<path>`, which persists the Cadenza-authored `cadenza:compiler/compile` component to disk (SPEC-BACKLOG #28, the
step the loop scoped the prior cycle). So for the first time the real byte-level self-hosting gate is runnable:
persist `compiler.cdz` → a 27 KB compile-component, then `cadenza-seed component-check <it> spec/semantics`
byte-diffs its output against native `cdz-rustc` over the whole corpus. The first run:

```
component-check: 58 agree, 496 disagree, 204 skip
```

496 "disagree" looks alarming against the interim `emit`-harness's rosy "0 hard, mostly clean declines." But
probing the disagreements directly shows the count is **misleading in exactly the way the interim harness's
trap-oracle bucket was** ([[a-decline-that-lands-on-a-trap-oracle-is-coincidental-agreement-not-a-semantic-trap]]),
now at the byte level: the vast majority of "disagrees" are **honest declines**, not miscompiles.

- **158 disagreements emit the byte-identical 88-byte component**, and it disassembles to `func 0 →
  unreachable` — a `KError` decline stub. Two structurally different unhandled programs (`(record (x 1))` and
  `(tuple 1 2)`) produce the *same* 88 bytes, and the component *traps* when run. So `compiler.cdz` correctly
  DECLINES a construct it can't read (records, strings, floats, effects), emitting a valid trapping component —
  exactly reject-don't-miscompile.
- `component-check` byte-compares that trapping decline stub against native's real output, sees different bytes,
  and scores `disagree`. It has no notion of "the component declined"; a decline and a genuine wrong-bytes
  miscompile are the same to it.
- Spot-checking for a *real* disagreement (a component that RUNS to a wrong result rather than trapping): `(effect
  E (op)) (def (main) 5)` compiles to `i64.const 5` (the effect declaration is dropped) — a non-decline
  disagreement worth a look, but it is NOT among the 158 + 70 + 45 constant-stub declines that dominate the count.

So the true self-hosting frontier, once declines are excluded, is roughly **58 agree plus the `soft`
fold-vs-overflow-helper set, and the rest is the reader not yet decoding records/strings/floats/effects** —
expected, not a regression.

**Why.** This is the same measurement principle the loop keeps rediscovering on each new gate: **a differential
that compares only the observable a decline shares with a real answer cannot separate the two.** On a value
oracle, a decline traps where a value is wanted — visibly distinct (the interim harness scores it `decline`). On
a trap oracle, a decline and a semantic trap share `unreachable` — indistinguishable without a trap-cause check
(#26). On the *byte-level* gate, a decline (a trapping stub) and a wrong-but-valid miscompile both produce "bytes
that differ from native" — indistinguishable without checking whether the component's entry is a bare
`unreachable`. Each gate needs the *same* discriminator, re-derived in its own terms: **is this component a
decline, or did it actually compute (and get it wrong)?** The byte-level gate is the strongest evidence of
correctness where it says `agree` (byte-identical is unforgeable), but its `disagree` count is nearly worthless
until declines are subtracted — and the danger is symmetric with the trap-oracle finding: a raw "496 disagree"
reads as catastrophe when it mostly means "the reader doesn't cover records yet," while a *real* miscompile
(a component that runs to a wrong value) is hidden in the noise of 496 rather than standing out among a handful.

**The requirement it drove.** No corpus case — the corpus cases are correct; the gap is in the gate's
classification, and it is seed/tooling work, not spec. Two outputs handed to the compiler agent (via the
`📡 FROM THE CONFORMANCE LOOP` channel) and the operator (SPEC-BACKLOG): (1) **`component-check` must gain a
decline discriminator** — if the emitted component's entry core func is a bare `unreachable` (no computational op
before the trap), classify the case `decline`, not `disagree`, so the `disagree` count means genuine miscompiles
only (a component that RUNS to wrong bytes). This is the byte-level twin of the trap-cause discriminator the
interim harness already got (#26), and the cheapest form is the same entry-func disassembly check. (2) Until it
lands, **read `component-check`'s `disagree` as `decline + disagree` combined** — the `agree` count (58) is the
real signal, and the true miscompile set is only the non-`unreachable` disagreements (e.g. the dropped-effect
`(effect …) (main) 5` case), which must be enumerated separately before the number means anything. General
lesson, now proven across three gates (value / trap / byte): **every new differential inherits the
decline-vs-result blind spot and must be given the discriminator explicitly; a gate's headline count is
trustworthy only in the direction where the shared observable cannot be counterfeited — `agree`/byte-identical
up, never `disagree` down.**
