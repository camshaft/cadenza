# run-sim generic return — STAGED (commit after a9d2cabb2 lands)

v-music (note 10012) needs `run-sim` generic in its return type (task returns List(MidiEvent), not
Instant). VERIFIED the idiomatic generic-boundary form compiles + returns the generic type + exports:

Change `implementation/des/src/sim.cdz`:
- `def run-sim(task: Unit -> Instant) = … task(unit)`
  → `def run-sim(t: Type, task: Unit -> t) = … task(unit)`
- update the 4 @test call sites: `run-sim(fn(_u) => …)` → `run-sim(Instant, fn(_u) => …)`
- export line unchanged (run-sim still exported; boundary now takes the Type param, verified clean).

WHY the Type param (not `Unit -> a`): Cadenza has NO forall-binder in an annotation — a lowercase type
var at a boundary is CDZ0101 ("no ∀-binder; take the type as an explicit Type parameter"). Unannotated
is polymorphic but can't EXPORT (CDZ0201 boundary-param-ambiguous). So `t: Type` is the idiomatic
exportable-generic form. VERIFIED: `run-sim(Int64, fn(_u) => (Sim.sleep(…); 42)) == 42` passes + exports.

DO NOT commit until a9d2cabb2 (the base module) is on trunk — don't stack on the unlanded commit.
When it lands: sync clean, apply the change, `cdz test implementation/des` (4 pass), fmt --check, MR.

ALSO for v-music (flagged in note): match rest-pattern `[e, .. t]` is NOT supported yet — their
play-task must use List.fold (or List.first/rest recursion) instead of a cons/rest match arm.
