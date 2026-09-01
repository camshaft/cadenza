# Vertical-ready brief: SESSION LIFECYCLES for the agent harness

**Design doc (landed on trunk):** `implementation/design/DESIGN-session-lifecycle.md`
(landed via PR #2372, trunk @ ff4476907; peer-converged with v-agent-harness + v-agent-harness-host +
design-session-directory).

**Subsystem:** `cdz-kernel` (durable events + fold guard) + `cdz-agent-host` (host executors).
Kernel slices are `v-agent-harness`'s zone; host slices are `v-agent-harness-host`'s zone — this is a
coordinate-not-fork feature spanning both, so the natural owner is **v-agent-harness leading, with
v-agent-harness-host owning I3–I6 host/Cedar seams** (mirrors how the cross-session-messaging E2E was
split). Alternatively a dedicated `v-session-lifecycle` vertical that coordinates both.

**What it is:** lifecycle-CONTROL of ANOTHER session as first-class Cedar-gated effect families
(`lifecycle/spawn|suspend|resume|terminate`), layered on the §6/§6a supervision roadmap (CloseOutcome
BUILT; spawn/child-completed planned) and the just-landed Emit/Inbound cross-session messaging. NOT a
duplicate of a session closing itself — this is controlling a *different* session's lifecycle, which
does not exist today.

**First increment (I1):** kernel — add `EventBody::Terminated{by, reason}` + a first-class
`FoldRefused` guard (a session whose log tail is `Terminated` refuses further folds — a kernel guard,
not a host convention, so a buggy host can't re-drive it). Gate: a fold-unit test (terminate → next
fold refused) + a replay test (recovered session stays terminated). This is the smallest durable
foundation the rest builds on; it is `v-agent-harness`'s zone.

**Full increment plan (see doc §"Increment plan"):**
- I1 kernel: `Terminated` marker + `FoldRefused` guard  ← START HERE
- I2 kernel: `Spawned{child_hash}` edge + genesis parent-provenance (= §6 slice-2)
- I3 host: `lifecycle/spawn` executor — INLINE registry mutation via `&mut AgentHost` (NOT
  route-and-await; on-loop effects self-deadlock — settled with v-agent-harness-host)
- I4 host: `lifecycle/suspend` + `lifecycle/resume` — per-session drive-eligible bool; QUEUE (not drop)
  inbound during suspension
- I5 host: `lifecycle/terminate` executor + a NEW PERMANENT Emit-bounce route (terminated target ≠
  RETRYABLE closed-inbox); also drives design-session-directory's group auto-evict via the Terminated
  signal
- I6 Cedar: new `ResourcePredicate::DescendantOf` — tree-derived descendant authority from the durable
  spawn-edge log (the spawn tree IS the supervision tree IS the grant; no bearer token)
- I7 prelude: userspace supervisor library (= §6a slice-4) — one-for-one restart + retry-with-backoff
  + restart-intensity ceiling, consuming ChildExited/FoldFailed

**Cross-design contract (LOCKED with design-session-directory):** terminate → my `Terminated` event
drives their group auto-evict (their I5); direct-name = tombstone; suspend = transparent to directory.

**Remaining external dependency:** SessionId = genesis-hash-hex (v-agent-harness pinning with operator).
Confirm the pin landed before I3 freezes the spawn effect-result type. All other host-mechanics
questions are resolved and recorded in the doc's "Coordination points" section.

**Operator context:** operator explicitly wants this designed AND implemented ("assign it to an owner
after everything is ready"). Priority: AFTER session naming, in parallel with the naming build.
