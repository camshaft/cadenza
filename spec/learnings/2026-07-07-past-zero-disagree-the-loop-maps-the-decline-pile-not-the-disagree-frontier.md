# Past zero disagree, the loop maps the decline pile, not the disagree frontier — soundness is a certified negative, coverage is the positive frontier

*2026-07-07*

**What happened.** The byte gate reached 0 disagree (Run 114): the self-hosted compiler is sound on everything it
handles. The next cycle was quiet — compiler.cdz barely changed (+8 bytes), gates held (byte gate PASS, native
574/0). With no disagree to chase, I turned the same mapping technique that characterized the disagree frontier
(Run 111) onto the DECLINE pile: a per-file byte-gate breakdown of the 434 declines. The result is a coverage
roadmap — 05-compound-types 139 (runtime records/tuples/lists/maps as results and operands — the M2
runtime-compound-output gap, ~1/3 of all declines), 02-binding-and-control 56, 10-bytes 49, 13-strings 45,
09-functions 34 (closures/HOF), 12-metaprogramming 26 (ask-39), 14-effects 22, then a long tail. The dominant
target is runtime-compound VALUES, which also underlies chunks of the string/bytes/list/equality files (an
operation that returns a compound hits the same wall). I filed it as ask-57.

**Why.** The zero-disagree milestone changes what the loop is FOR, and the change is worth stating precisely.
Before the milestone, the gate's headline number (disagree count) WAS the frontier — every disagree was a
soundness defect to drive to zero, and the loop's job was to find, characterize, and verify each one down. After
the milestone, disagree is a certified NEGATIVE: "the compiler never produces a wrong answer on what it handles,"
and the differential gate keeps certifying it continuously and for free (any new emit path either matches native
or the gate catches it as a fresh disagree). So the loop no longer has a disagree frontier to hunt — soundness is
done and self-maintaining. What remains is the POSITIVE frontier: coverage, the set of features the compiler
declines rather than emits. And coverage is not visible in the pass/fail of the gate (the gate PASSES with 434
declines); it is visible only in the decline pile's SHAPE. So the loop's measurement pivots from "drive the
disagree count to zero" to "map the decline pile so the coverage push is ordered by leverage" — the same
map-the-frontier move as Run 111, now aimed at declines instead of disagrees.

The leverage principle carries over intact and is the reason the map is worth more than the count: **cluster the
declines by the SHARED MISSING CAPABILITY, not by corpus file, because one capability underlies many files.**
139 declines are in 05-compound-types, but runtime-compound VALUES also gate a string operation that returns a
string, a bytes concat that returns bytes, a list builder that returns a list, a structural-equality on two
records — so the true size of the "runtime-compound emission" gap is larger than any one file, and landing that
one capability (the value-heap alloc + type-directed renderer the native seed already has) cascades across
domains. A file-by-file count understates the leverage; the capability-clustered read is what says "build
runtime-compound emission first, and ~a third of the decline pile plus pieces of four other files fall together."
This is the coverage-phase analogue of Run 111's "a rejection family that looks like N checks is often one check
at N positions" — here, "a decline pile that looks like N feature files is often a few capabilities each gating
many files."

One honesty note the loop must keep making: a PASSING gate with 434 declines is not "almost done." The pass
certifies soundness, and the 434 is the distance to completeness — a large, real distance. The value of reporting
both numbers separately (0 disagree AND 434 decline) rather than a single "percent" is exactly that it prevents
reading a green gate as a finished compiler; the map turns the 434 from an intimidating lump into an ordered,
leverage-ranked backlog, which is the honest and useful thing to hand the operator at a milestone.

**The requirement it drove.** No corpus case — the map is measured over the existing corpus (every decline is an
already-pinned case the compiler doesn't yet emit), and each cluster converts decline → agree when its capability
lands, with the gate guaranteeing soundness through the change. The output is ask-57 (the coverage frontier map:
434 declines by feature domain, with the runtime-compound-emission cluster identified as the highest-leverage
next push) and this learning. General lesson: **once a differential gate reaches zero disagree, soundness becomes
a certified, self-maintaining negative and the loop's measurement pivots to the positive frontier — map the
decline pile clustered by shared missing CAPABILITY (not by file, since one capability gates many files), so the
coverage push is leverage-ordered; and keep reporting disagree and decline as separate numbers, because a passing
gate with a large decline count is sound, not complete.**
