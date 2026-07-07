# A frozen interface contract supersedes the in-flight asks that assumed its old shape — and freezes a seed-migration gap open

*2026-07-07*

**What happened.** The diagnostics/output surface the loop had been tracking through two asks — ask-38 (the
operator's "decode returns a Result, not a trap," which I'd flagged as an open Option-vs-Result signature
question) and ask-40 (the `compile` diagnostics channel, assumed to be `result<list<u8>, list<diagnostic>>`) —
was **reshaped by a newly-frozen contract**, `spec/contracts/build-tool-interface.md` (constitution Amendment
0.8.0). The derivation entry is no longer a two-arm result; it is a **kinded-artifact interface**:

```
compile : list<artifact> -> compile-output
record compile-output { artifacts: list<artifact>, diagnostics: list<diagnostic> }
record artifact  { kind: string, bytes: list<u8> }
record diagnostic { severity, code, message }
```

Success = a component artifact present with no error-severity diagnostic; failure = no component artifact with
≥1 error diagnostic; a warning rides *alongside* a produced component. Diagnostics and byte-outputs are **distinct
channels**, not mutually-exclusive arms — which moots the Option-vs-Result question ask-38 raised (neither: it's
the artifacts+diagnostics record) and re-shapes ask-40's whole premise.

Probing the running seed showed the split the loop must record: **the contract is frozen in the spec, but the
seed's driver ABI has not migrated.** `cadenza-seed compile-run` / `component-check` still return a single
`list<u8>` (`compile → Ok (N bytes)`), and a type-rejection still emits the 88-byte bare-`unreachable` decline
stub — no `compile-output` record, no diagnostics surfaced. So the ~30 ask-30 type-rejections are still at
`decline`, not `agree`; the coded-diagnostic output the contract now mandates requires migrating both the seed's
compile-component ABI and the checker's expectation to the new record.

**Why.** This is the loop's reconciliation job, distinct from the design rationale (which the sibling's learning
covers). When a frozen contract lands over an area the loop has open asks in, three things must happen and only
the loop is positioned to notice all three: (1) the asks that assumed the *old* shape are superseded, not
completed — ask-40's "return a `result<_, diagnostic>`" is now the *wrong* target, and leaving it unreconciled
would send a fix at a shape the spec no longer blesses; (2) a resolved-elsewhere question (ask-38's Option-vs-
Result) becomes moot and should be marked so, not left as an open operator decision; (3) the freeze does not
itself change the implementation — it opens a *migration gap* (seed ABI + checker → the new record) that is now
the real remaining work, and a probe is what distinguishes "contract frozen" from "seed conforms." The general
lesson: **a frozen contract is a spec event, not an implementation event — the loop's value at a freeze is to
re-probe the seed against the new shape, re-target the asks that assumed the old one, and record the migration
gap the freeze opens, so a green gate (which still measures the old ABI) is not mistaken for conformance to the
new contract.** The byte gate stayed 65 agree / 124 disagree, WRONG=0 — unchanged, because it measures the seed's
current (old) ABI; the contract change is invisible to it, which is exactly why the reconciliation has to be done
by reading the contract and probing the seed, not by watching the gate.

**The requirement it drove.** No new corpus case — the build-tool interface is an ABI/interface contract (what
the tool consumes and produces), not a value-behavior the `(output (: v T))` oracle can express; it lives in
`spec/contracts/`, corpus-unpinnable by construction (same as the component-ABI and value-interchange contracts).
The outputs: a reconciliation note on ask-40 (the diagnostics channel is now the artifacts+diagnostics record,
not a two-arm Result; the Option-vs-Result flag on ask-38 is moot; the seed's driver ABI + checker still owe the
migration, and the ~30 type-rejections stay `decline → agree`-blocked on it), and this learning. General lesson
restated: **at a contract freeze, the loop re-probes and re-targets — the frozen spec says what the seed MUST
do, the probe says what it DOES, and the gap between them is the migration the freeze just made the real work.**
