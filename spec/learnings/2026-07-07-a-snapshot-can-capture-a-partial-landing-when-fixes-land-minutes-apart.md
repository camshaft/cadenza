# A pinned snapshot can capture a partial landing when related fixes land minutes apart — the frozen reference has a seam, not just an age

*2026-07-07*

**What happened.** The pinned `implementation/stable/` snapshot is the loop's reproducible probe target (it
exists so the loop measures against a fixed seed instead of one rebuilding mid-cycle). This cycle it refreshed to
a 16:38 build — and I found it had captured a feature HALF-landed. Two fixes for the same feature (effect-based
diagnostics) landed ~2 minutes apart:

- **ask-49** (a recursive-effectful `handle` whose result is a runtime compound lowers on the run/`emit` entry) —
  IN the 16:38 stable: `(do (w 3) (Bytes.of …))` under a handler → `ran → Value("b\"\\x03\"")`.
- **ask-51** (the `compile-output` ABI detection recurses through a `handle`'s body) — NOT in the 16:38 stable: a
  `handle`-tail `compile-output` record → `Ok (0 bytes)` (the bytes-ABI fallback), whereas the freshly-built
  16:40 seed gives `Diagnostics: [(CDZ0201,bad),(CDZ0201,bad)]` (the artifact ABI).

So probing stable ALONE would have painted a self-contradictory picture: "the compiler can lower a handler that
returns a compound record (ask-49), but it can't recognize that same record as the compile output when it sits
inside the handle (ask-51)" — two halves of one feature, one present and one absent, because the snapshot froze
in the gap between the two commits. Cross-probing stable-vs-live (mtimes 16:38 vs 16:40, `cmp` confirms they
differ) resolved it: ask-51 is a live-only landing the 16:38 refresh missed by ~2 minutes.

**Why.** My earlier learning ([[a pinned toolchain snapshot gives the loop a reproducible probe target]]) framed
the snapshot's risk as *staleness* — the frozen reference lags the live artifact, so a fix can be "done live, not
yet in stable." This cycle sharpens that: the risk isn't only that stable is OLDER, it's that stable can be
FROZEN MID-FEATURE. A refresh is a single instant, and a feature is usually several commits; if the refresh
instant falls between two of them, the snapshot holds a coherent-looking but internally partial state — capability
A present, its sibling capability B absent — and any conclusion drawn from it about "the feature" is wrong in a
way that looks like a compiler bug ("why does the handler lower but the ABI not see it?") rather than a snapshot
seam. The discipline this adds: **when a snapshot shows one half of a known multi-part feature working and another
half not, suspect a refresh-timing seam before a compiler inconsistency — check the sibling fix's commit time
against the snapshot mtime, and cross-probe live.** The snapshot is reproducible, which is exactly why it must not
be trusted as COMPLETE: reproducibility freezes whatever partial state existed at the refresh instant, including a
half-applied feature. A snapshot's value is a stable *denominator*; its hazard is that the numerator it froze may
be mid-fraction. The fix on the process side is small — refresh from a build known to be past the whole feature's
last commit, and record which asks the snapshot is known to contain vs. lack — but the reasoning trap (reading a
refresh seam as a compiler contradiction) is the thing to keep.

**The requirement it drove.** No corpus case — this is a property of the loop's measurement apparatus (a pinned
snapshot's refresh timing), not a language-value behavior the `(output (: v T))` oracle expresses. The output is
the loop-corroboration recorded on ask-51's pending-validation note (the boundary flips stable→live, and the
16:38 stable is confirmed to LACK ask-51 while containing ask-49) and this learning. The behavior itself is the
sibling's, already documented; the loop's value here was the independent cross-probe that pinned the partial-
landing seam precisely. General lesson: **a pinned snapshot is a reproducible denominator, not a guarantee of a
whole feature — a refresh is one instant and a feature is many commits, so a snapshot can freeze a half-landed
feature; when one half of a known multi-part landing works and another doesn't on the SAME snapshot, check the
refresh mtime against the fixes' commit times and cross-probe live before calling it a compiler bug.**
