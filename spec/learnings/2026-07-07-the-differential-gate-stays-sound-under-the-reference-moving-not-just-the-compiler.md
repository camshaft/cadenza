# The differential gate stays sound under the REFERENCE moving, not just the compiler — a co-evolving pair only ever grows the decline pile, never a false disagree

*2026-07-07*

**What happened.** This cycle the native reference seed advanced: its behavior gate went 574 → 577 passed (todo
6 → 4, skip −1), so three corpus cases the native compiler previously couldn't compile now compile natively
(HOF/effects work landing native-side). Meanwhile the self-hosted byte gate stayed **0 disagree** (134 agree / 0
disagree / 26 soft / 421 decline). I confirmed the mechanism: the three newly-native-compiling cases did NOT
become byte-gate disagreements — they are byte-gate DECLINES (the self-hosted compiler doesn't yet emit those
features, so it honestly refuses them). The reference gained capability; the differential gate absorbed it as
more declines, not as a single false disagree.

**Why.** The earlier zero-disagree learnings framed soundness-self-maintenance from the COMPILER's side: as
compiler.cdz gains an emit path, that path either matches native (agree/soft) or the gate catches it as a fresh
disagree — so the compiler can't silently regress. This cycle exposes the OTHER side, which matters because both
halves of the differential pair are under active development simultaneously (native and compiler.cdz each rebuild
most cycles). When the REFERENCE moves — native learns to compile something new — the differential is
`native=value, compiler=?`, and the self-hosted compiler's answer is one of: it also handles the feature
(agree/soft), or it doesn't (decline). It CANNOT produce a disagree from native advancing alone, because a
disagree requires the compiler to produce a WRONG value, and a compiler that lacks a feature declines (emits a
stub) rather than guessing. So a reference that races ahead only ever GROWS THE DECLINE PILE; it never manufactures
a false disagreement. The gate is sound under both sides moving: compiler-ahead is caught (disagree if wrong),
reference-ahead is absorbed (decline if unhandled), and the intersection — what both handle — is where agree/soft
live and where correctness is actually asserted.

This is the property that lets the loop trust 0-disagree in a co-evolving system where the denominator and both
compilers change every cycle. It would be reasonable to worry that "the reference improved" could break the gate —
in most test setups, the oracle moving IS a source of churn and false failures. Here it structurally cannot,
because the gate's failure condition (disagree) is a wrong VALUE, and neither "compiler lacks a feature the
reference gained" (→ decline) nor "compiler gained a feature the reference lacks" (→ the reference is the oracle,
so this shows as the compiler being ahead, which the gate reads against the reference — a disagree ONLY if the
compiler's value is wrong, which for a real feature it isn't) produces a wrong value. The decline bucket is the
shock absorber: every capability mismatch in EITHER direction lands there or in agree, never in disagree. That is
why the coverage phase is safe to run while native is also evolving — the reference can lead, the compiler can
lag, and the only visible effect is the decline count breathing up and down, with disagree pinned at zero.

The honest caveat the loop must keep: this soundness is about VALUES, and it assumes the reference itself is
correct. The differential gate certifies "compiler agrees with native where both run"; if native itself were
wrong, the gate would happily certify the compiler matching a wrong reference. That risk is carried by the OTHER
gate — the corpus's own `(output …)` oracle on the native behavior gate (577/0 here) — which is why both gates
matter: the behavior gate checks native against the hand-written oracle, and the differential gate checks the
self-hosted compiler against native. Neither alone is sufficient; together they pin correctness from two sides.

**The requirement it drove.** No corpus case — the observation is about the gate's behavior as the reference
moves, measured over the existing (growing) corpus. The output is this learning and the confirmed mechanism
(native 574→577, byte gate held 0 disagree, the 3 new-native cases became declines not disagrees). General
lesson: **a differential gate between two co-evolving compilers stays sound under EITHER side moving — a compiler
gaining a feature is caught (disagree if wrong), a reference gaining a feature is absorbed (decline if the
compiler lacks it), and neither produces a false disagree because the failure condition is a wrong VALUE and a
missing feature declines rather than guesses; the decline pile is the shock absorber for all capability mismatch
in both directions, which is what lets 0-disagree be trustworthy while both sides rebuild every cycle — with the
standing caveat that this certifies agreement-with-the-reference, so a separate oracle on the reference itself is
still required.**
