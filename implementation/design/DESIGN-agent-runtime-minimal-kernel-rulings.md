# Minimal-kernel — OPERATOR RULINGS on the forks (2026-07-17) + the revised critical path

**Owner:** v-agent-harness. **Amends:** `DESIGN-agent-runtime-minimal-kernel.md` (the 5 forks) and
`-minimal-kernel-plan.md` (the K-rungs). The operator ruled on forks **2** and **5**; this records those rulings,
**supersedes** the plan's mechanism-A recommendation, and re-derives the critical path. Forks 1/3/4 still pending
(the concierge reads 3 = extensible, 4 = oracle-then-delete, per the "everything modifiable" philosophy).

## Fork #2 (load-bearing) — RULED: WIDEN THE ABI; do NOT work around; BLOCK until fixed

Operator, verbatim: *"We need to widen the abi as much as needed to get the agent kernel working. This thing is
entirely why the cadenza language is what it is — to power a self modifying multi-agent orchestration system. So
whatever language features we need to accomplish that all the better. We don't want to work around any gaps.
BLOCK UNTIL THEY ARE FIXED."*

**This SUPERSEDES the plan's mechanism-A recommendation.** The plan proposed the Cadenza `interpret` program
*append its host-ops as `hostop` events* + return a scalar count (ABI-independent, no compiler change). The
operator explicitly rejects that class of workaround: the Cadenza program must return a **compound (a `List` of
host-op records) directly across the host boundary**, and the language/runtime must grow to allow it. The
agent-kernel port **blocks** on that ABI widening landing — this is deliberate: the whole point of Cadenza is to
power this system, so the language serves the kernel, not the reverse.

**Consequence for the shipped/held work:**
- **K1 (`kernel.rs`, held commit `81a600cb9`) is now built on a SUPERSEDED transport.** Its `HostOp` type +
  codec + `drain_and_execute` implement the *reject­ed* "drain `hostop` events" mechanism-A. It is NOT the v1
  design. Disposition: **do not send K1's mechanism-A drain path as the real kernel.** The `HostOp` *type* + its
  codec may survive as the payload the widened ABI carries (a `List HostOp` still needs each op encoded), but the
  "kernel drains hostop events it finds in the log" execution model is out. Re-scope K1 after the ABI lands.
- The new **critical path** is the host-ABI-widening itself — owned jointly by **v-rust-backend** (canonical-ABI
  lowering of a compound/list *result* across the boundary) + **v-effects** (the effect-result marshalling). I do
  not own that code; I've routed precise asks to both (locus: `rcdzc/src/backend/wasm/host.rs` — `abi_val_type`
  returns `None` for compound results; `HostOpDescriptor.result: Option<AbiValType>` is scalar-only per its
  E2h-2 scope). My kernel-port work waits on it and I coordinate + provide the target signature.

**The target the ABI must support:** `interpret : (tail: List Event, new_event: Event) -> (List HostOp)` — a
Cadenza host-callable that RETURNS a runtime-built list of host-op records (a resource-escaping compound result).

## Fork #5 (bootstrap/genesis) — RULED: genesis injected MANUALLY by the CLI

Operator, verbatim: *"The Genesis program would be injected manually by the cli. And then you can start the
daemon and get the ball rolling."*

**Even more minimal than the plan's proposal.** The plan floated "one hardcoded `program`-kind (genesis reads the
latest `program` event + calls it)". The operator removes even that: the **kernel needs NO bootstrap program and
NO hardcoded genesis event-kind**. The CLI manually injects the genesis program (as an event/program) into the
log; then the daemon starts and runs it. So:
- The Rust kernel does **not** name a `program` kind or embed any genesis. It is handed "run the program the log
  already contains" — the CLI put it there.
- This keeps the kernel maximally tiny (aligns with the mandate) and means the very first program is itself
  self-modifiable data in the log, injected by an operator/CLI action, not baked into Rust.

## Forks still pending (routed, no ruling yet)

- **#1 host-op floor (4 vs 3):** unchanged; awaiting ruling. (With the ABI widened to return a `List HostOp`, the
  set is whatever ops that list can name — `append`/`read_tail`/`invoke`/`schedule` remain the candidates.)
- **#3 capability registry extensible vs redeploy:** concierge reads EXTENSIBLE (per "everything modifiable"); not
  yet explicit.
- **#4 keep L2/L3 Rust as oracle vs delete now:** concierge reads keep-as-oracle-then-delete; not yet explicit.

## Revised plan — the K-rungs re-derived against the rulings

- **K0 (NEW, now the critical path, NOT mine to implement) — host-ABI compound-return.** Coordinate v-rust-backend
  + v-effects to widen the boundary so `interpret -> (List HostOp)` works. **Everything below blocks on K0.**
- **K1 (re-scoped) — the tiny kernel driver, once K0 lands.** Run the CLI-injected genesis program via `cdz_run`,
  passing the opaque tail; receive its returned `List HostOp` (via the widened ABI — NOT drained from the log);
  execute each op via the primitives. No hardcoded genesis, no event-kind knowledge. (The old K1 `HostOp`
  type/codec is salvageable as the list-element encoding; the drain-events execution model is dropped.)
- **K2/K3 — port msg.rs/sub.rs → Cadenza** against the Rust differential oracle (fork #4 = keep-oracle, pending).
- **K4 — slim fold.rs** (model-event record/replay → Cadenza). Still fork-independent of #1/#3/#4, but now also
  waits on K0 (the interpret program drives the model `invoke`).
- **K5 — delete the Rust event code** once the Cadenza versions pass.

## What I do while K0 (the ABI) is in flight

Per the operator ("block until fixed") I do NOT build the kernel-port on a workaround. Fork-independent,
K0-independent work remains: (a) coordinate/​unblock K0 with v-effects + v-rust-backend (done this tick —
asks sent with the locus); (b) author the `interpret.cdz` Cadenza program's LOGIC (it needs to exist regardless;
writing it surfaces the exact ABI shape K0 must deliver + any language gaps to REPORT); (c) keep the L2/L3 Rust
landing as the differential oracle. I will NOT send K1's mechanism-A drain path as the kernel.
