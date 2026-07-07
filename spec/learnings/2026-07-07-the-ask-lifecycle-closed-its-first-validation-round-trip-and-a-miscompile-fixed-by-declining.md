# The ask lifecycle closed its first validation round-trip — and a miscompile fixed by declining is a valid resolution

*2026-07-07*

**What happened.** The operator's ask-lifecycle (`open → pending-validation → done`, filed the prior cycle) had
its first full round-trip this cycle, and it worked exactly as designed. The compiler agent, having adopted the
directories, **moved four asks into `pending-validation/`** — items it had implemented and wanted re-probed:
ask-19 (nested constructor pattern under `Some`), ask-25 (the `main`-named entry reorder, unblocked by the gap-3m
fix), ask-31 (checked arithmetic), and ask-34 (the first real miscompile). The loop's job on a `pending-validation`
item is to **re-probe the running artifact and confirm — or bounce it back** — never to trust the pending note.
All four validated against the live seed/`compiler.cdz`:

- **ask-19** — `(match (List.at xs 0) ((Some (E.Lit n)) n) …)` on a param list emits a valid component (was a
  decline); pinned as a fresh gate case → 5 (PASS).
- **ask-25** — a HELPER-FIRST module `(def (f x) …) (def (main) (f 41))`, previously a clean decline (`main`
  wasn't func 0, blocked on gap 3m), now compiles and runs to **42**: the entry is selected by NAME regardless of
  position. Byte gate confirmed the shift (the mutual-recursion cases moved decline → soft; total disagree
  153 → 141).
- **ask-31** — checked arithmetic ok/overflow arms verified (→ 42 / None).
- **ask-34** — the `(id true)` → `1` miscompile: re-probed, it now **traps** (decline), as do the two related
  invalid-emissions (`(pick true)` = `(if x true false)`, `(neg true)` = `(not x)`) — all clean valid trapping
  declines now.

All four moved `pending-validation → done` with the verifying evidence stamped in each file.

**Why.** Two lessons, one about process and one about the fix.

*The lifecycle proved its worth as a two-party protocol.* The prior cycle it was a static reorg; this cycle it
carried real traffic: the loop files (open) → the agent implements and moves (pending-validation) → the loop
re-probes and confirms (done). The critical property is that `done` is reached only by the loop re-probing the
live artifact, never by the implementer's claim — and this cycle that mattered concretely, because the artifact
moved *under the probe* (compiler.cdz changed mtime twice mid-cycle), and the honest numbers (byte gate 153 → 141
disagree, ask-34 `1` → trap) came only from re-running against the stabilized current binary, not from the
pending notes. A pending note is a claim; a `done` stamp is a measurement. The protocol enforces the loop's whole
discipline (probe, don't trust) at the granularity of a single ask.

*A miscompile fixed by declining is a valid resolution — the right one, first.* ask-34 was the first genuine
wrong-value miscompile (`(id true)` returning the integer `1`), and the agent resolved it not by the full fix
(specialize the polymorphic return kind to the argument's — which would reach byte-identity) but by the cheaper
**decline** (option 2 of the two I offered): trap rather than mis-widen a Bool to i64. That is exactly the
correct order of operations under reject-don't-miscompile — **a wrong value is the worst outcome; a decline is
honest; agreement is best but optional.** Turning the miscompile into a decline is not a half-fix, it is the
*first* fix: it removes the danger (a program silently computing the wrong value) immediately, and demotes the
remaining work (decline → agree) to a completeness item that can wait. The byte gate makes this legible: the case
moves `disagree → decline`, out of the "real miscompile" column and into the "honest frontier" column, and the
full-agreement fix becomes a separate low-priority ask (ask-35) rather than a blocker. The general principle:
**when a miscompile can't yet be compiled correctly, make it decline — restore reject-don't-miscompile now, chase
byte-identity later.** The dangerous state is the wrong value in between, and it should exist for the shortest
possible time.

**The requirement it drove.** One new corpus case (ask-19's shape, now that it compiles): *"a constructor pattern
nested under Some matches a runtime list element"* (→ 5, gate PASS) — a verified capability becomes a pinned case,
per the standing discipline. No corpus change for the others (ask-25/31/34 are `compiler.cdz`/seed behaviors the
byte gate and existing corpus already measure). One new follow-on ask (ask-35): the polymorphic-return-kind
specialization that would take ask-34's decline to `agree` — filed low-priority, since the miscompile is already
gone. And four asks confirmed `done` with re-probe evidence, closing the lifecycle's first full round-trip.
