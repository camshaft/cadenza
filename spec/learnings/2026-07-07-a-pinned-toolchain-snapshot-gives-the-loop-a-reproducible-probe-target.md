# A pinned toolchain snapshot gives the loop a reproducible probe target — and settles the churn readings

*2026-07-07*

**What happened.** A `implementation/stable/` snapshot appeared: a frozen, all-gates-green copy of the seed
toolchain — `cadenza-seed`, the value-heap runtime `cdz_runtime.wasm`, the cdz-rustc reference component, and
`SHA256SUMS` for pinning — published so the self-hosting work runs against a *fixed* seed instead of the
`implementation/seed/` binary that rebuilds mid-cycle. Verifying it: all three `SHA256SUMS` check OK, the stable
seed runs the behavior gate green (569), the byte-level self-hosting gate through it reads **65 agree / 124
disagree / 385 decline / 204 skip**, and the standing WRONG sweep is **0**. Those byte-gate numbers match the
last several cycles' readings on the churning seed — which retroactively confirms that the fluctuation I saw
earlier (183 disagree one cycle, 137 the next, back to 124) was transient in-flight churn, not real movement.

**Why this matters for the loop specifically.** Several recent cycles were muddied by the seed rebuilding *while
I probed*: a gate that timed out on a loaded box (a false regression I had to isolate), a byte-gate count that
swung because the ABI was half-migrated mid-measurement, an mtime that moved twice under a single cycle's
probes. Every one of those cost a reconciliation step to separate "the artifact changed" from "the artifact was
changing *as I measured it*." A pinned snapshot removes that whole class of confound: **probing a frozen
reference means a reading reflects the artifact, not the moment I caught it.** The loop's core discipline is
probe-don't-trust, and a probe is only as trustworthy as the target's stability — measuring a moving target is
probing something that no longer exists by the time you read the result. So the loop should, going forward, probe
against `implementation/stable/` (with `CADENZA_RUNTIME` set to its runtime, by ABSOLUTE path — a relative
runtime path resolves wrong and silently fails the component write), and treat `implementation/seed/` as the
live edge to *notice* (mtime/size deltas signal the agent is working) but not to *measure against*. Cross-check
the two only when a stable-vs-live divergence is itself the question (e.g. "did the agent's latest rebuild fix
ask-42?" — that's a deliberate live-seed probe, and the answer goes on the ask, dated to the live binary).

**The requirement it drove.** No corpus case, no ask — this is a loop-procedure improvement, not a spec or seed
finding. The durable output is this learning plus a standing-procedure update: **probe against
`implementation/stable/cadenza-seed` + `CADENZA_RUNTIME=<abs>/implementation/stable/cdz_runtime.wasm` for
reproducible byte-gate/WRONG-sweep readings; watch `implementation/seed/` mtimes as the activity signal; verify
`SHA256SUMS` when the snapshot refreshes.** The snapshot also documents the current frontier plainly: ask-42
(the deep-sum-match decline under Result-shaping) is known-open on it, and the durable fix is the kinded-artifact
interface (ask-41 / Amendment 0.8.0), under which the same logic compiles — so on this snapshot `compile` stays
bare-`Bytes` (trap on `KError`) and the diagnostics `Result`-wiring awaits the artifact interface. General
lesson: **a long-running measurement loop against a concurrently-changing artifact needs a pinned reference to
measure against — otherwise half its findings are indistinguishable from the artifact moving under the probe,
and the loop spends cycles reconciling churn instead of catching real change.**
