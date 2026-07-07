# "Disagree" rising can be progress — cases moving off the decline floor into the soft/heap middle ground

*2026-07-07*

**What happened.** The byte gate moved 61 agree / 137 disagree / 377 decline → 61 agree / **183 disagree** /
**330 decline** — declines dropped ~47, disagrees rose ~46. On its face that reads as a regression: honest
declines becoming disagreements. But the standing full-oracle dangerous-bucket sweep stayed **WRONG = 0**, so no
case moved to a wrong value. Probing the ~46 that moved showed the opposite of regression: the +3.3 KB
`compiler.cdz` change **expanded coverage** — many `let`, `match`, and pattern constructs that previously
declined (emitted a bare-`unreachable` stub) now COMPILE, to value-correct or heap-compound results. They left
the decline floor and landed in the soft/heap middle ground, which `component-check` (agree/disagree/decline)
scores as `disagree` because it has no `soft` bucket and no scalar oracle for heap results.

Classifying the 151 `native=ok` disagreements: **29 soft** (value-correct, byte-different — e.g. a nested-let
chain → 2048, but 464 B where native is 230 B because `compiler.cdz` inlines the checked-arithmetic guards per
op while native shares a helper), **37 still decline-stub** (match-on-integer-literal etc. — genuinely still
declined), **85 "other"** (the newly-compiling `let`/pattern cases that build heap/compound values my scalar
sweep can't compare — WRONG=0 confirms none is a miscompile). The net: the compiler attempts and correctly
handles far more of the language than a cycle ago; the raw disagree count rose precisely *because* fewer cases
sit on the decline floor.

**Why.** This is the third distinct way `component-check`'s three-bucket score misleads, and the mirror of the
earlier ones. Before: `disagree` over-counted because it lumped honest declines with real miscompiles (fixed by
the decline discriminator, ask-29/33). Now: `disagree` *rising* over-signals regression because a case leaving
`decline` for `soft`/heap is progress but registers identically to a case that got worse. The through-line: **a
gate with fewer buckets than the phenomenon has states cannot express direction — "disagree went up" is not a
sign until you split it into soft (good), still-declined (neutral), and wrong-value (bad).** The only bucket
whose movement is unambiguous is `agree` (up is always good) and `WRONG` from the loop's own sweep (up is always
bad); every raw `disagree`/`decline` delta needs the sweep to interpret. The reassuring invariant that made this
cheap to read correctly: WRONG=0 held, so whatever moved, none of it became a miscompile — the decline→disagree
shift was safe by construction, and only its *value* (progress vs. churn) needed probing.

**The requirement it drove.** No new corpus case — every one of the newly-compiling cases is already pinned;
the coverage growth is measured directly (fewer declines, WRONG=0). The durable output is this learning and the
measurement caveat carried forward: **read `component-check` deltas through the WRONG sweep, not off the raw
disagree/decline counts — a rising disagree with WRONG=0 is coverage moving off the decline floor (good), not a
regression.** (Separately confirmed this cycle: ask-40, the diagnostics channel, has NOT landed — type-rejections
like `(+ 1 true)` / `(if true 1 false)` still emit the 88-byte bare-`unreachable` decline stub, not a coded
`Diagnostics` result; so the ~30 native-rejected disagreements are still decline-blocked, awaiting the coded-
diagnostic ABI to reach `agree`.) General lesson: **when a gate's headline number moves the "wrong" way, check
the invariant that would make it a real regression (here WRONG) before reading the number as bad — a coarse
gate's count can move against the grain of actual progress.**
