# op-takes-function — op takes a function argument, arm applies it to the state
## pyhof1 — apply(f) resumes (f s); body passes (*10) then (+100) closures. Model 1102/101. PASS x3, real-harness round-trip clean.
Higher-order op argument invoked inside the handler arm at the current state; two distinct closures across state-threaded dispatches. Promotable pass-witness.

## Side notes (CDZ0201 boundaries, NOT fold probes)
- An op RETURNING a function (op mk (-> (-> Int64 Int64))) is rejected: "resumes with (-> Int64 Int64) but result type is Int64" — the (-> (-> ..)) nullary-returns-fn form doesn't express a function result here (op-return-type limitation; corpus only ever has ops that TAKE fn args, never return them).
- Map.of/Map.get and Set.empty don't exist (Map.empty/insert/lookup; Set.of(list)).
