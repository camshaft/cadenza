# SPEC RULING NEEDED (operator/spec-owner): leading-rest LET-BINDER traps, core-semantics §139 wants CDZ0210
STATUS: KNOWN (memory: leading-rest-list-binding-unsound-vs-spec), re-confirmed UNCHANGED by breaker 2026-07-16.
A too-short list through a LET-BINDER `(list a b .. r)` TRAPS (unreachable) at runtime; core-semantics §139
wants a COMPILE-TIME CDZ0210 for a refutable binding pattern. MEMORY-SAFE (halts, no wrong value) → NOT a
miscompile. BUT: (1) spec-conformance gap (trap vs CDZ0210), AND (2) conflicts with LANDED Inc-2 design —
the corpus has def-head/drop2 cases treating leading-rest bindings as IRREFUTABLE. So resolving it means
either (a) leading-rest binding IS refutable → CDZ0210 + revise the Inc-2 irrefutable cases, or (b) it's
irrefutable-by-design → the trap is wrong and it should bind (fill missing with...?) — a genuine design
fork. OPERATOR/SPEC-OWNER call, not a mid-tick fix. breaker pinned the sound MATCH-arm refutability as
coverage. ESCALATED to concierge 2026-07-16 (was memory-only, kept getting re-discovered).

## UPDATE 2026-07-16: concierge routed (a)/(b) to OPERATOR w/ (a) lean. HOLD impl + Inc-2 revision until ruled.
(a) → v-patterns adds CDZ0210 compile-reject + corpus-bugfix REVISES the Inc-2 def-head/drop2 irrefutable cases to refutable-reject.
(b) → v-patterns fixes the runtime trap to bind-whats-present + Inc-2 cases STAY + §139 carve-out.
Memory-safe (traps), no rush. Route impl to v-patterns + handle corpus side when operator picks.
