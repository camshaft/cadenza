# Host-primitive surface — the BROAD-REACH model (operator rulings on forks 1+3, 2026-07-17)

**Owner:** v-agent-harness. **Amends:** `DESIGN-agent-runtime-minimal-kernel.md` (the proposed 4-op interface is
SUPERSEDED) + `-rulings.md`. Records the operator's rulings on forks **1** (host-op set) and **3** (capability
extensibility) and the north-star principle. Only fork 4 (keep L2/L3 Rust as oracle) remains open.

## North star (make it the design's governing principle)

**The kernel is DEPLOY-ONCE-FOREVER.** Operator, verbatim: *"I never want to redeploy a new kernel unless we have
some security patches for our deps. Other than that, all kernel functionality needs to come from cadenza
programs. This likely means the host needs to expose very broad tools like 'execute this command on the host' and
'make this http call' for the broadest reach possible."* The ONLY sanctioned redeploy reason is a **dependency
security patch**. Every capability, behavior, and event-handler is a Cadenza program built on a small set of
BROAD host primitives.

## Fork 1 — RULED: broad generic primitives, NOT a narrow agent API

I had framed fork 1 as "how few narrow ops (append/read_tail/invoke/schedule)". The operator wants the **opposite**:
maximize **reach per primitive** so a new host op is *never* needed. The host exposes broad, generic, high-reach
primitives; Cadenza composes everything (messaging, subscriptions, dispatch, the agent loop, model calls, builds,
fleet coordination) out of them. Proposed broad-primitive surface:

| Primitive | Signature (shape) | Reach |
|---|---|---|
| `exec` | `exec(cmd: String, args: List String, stdin: Bytes) -> {code, stdout, stderr}` | run ANY host command — the universal escape hatch (git, cargo, the compiler, a model CLI, anything) |
| `http` | `http(method, url, headers, body: Bytes) -> {status, headers, body}` | ANY network call — Bedrock, other agents, external services |
| `log_append` | `log_append(kind: String, payload: Bytes) -> Seq` | the one write to the event log |
| `log_read` | `log_read(from: Seq) -> List Event` | read the ordered tail |
| `fs_read` / `fs_write` | `fs_read(path) -> Bytes` / `fs_write(path, Bytes)` | host filesystem (worktrees, artifacts, the store) |
| `now` | `now() -> Timestamp` | the one clock (recorded-effect determinism still applies — the result is logged) |

That is the whole host. Note `exec` + `http` alone give near-total reach (a model call is `http` to Bedrock or
`exec` of a CLI; a build is `exec cargo`; fleet send is `fs_write`/`exec`). `invoke(capability,…)` from the old
design collapses INTO this: a "capability" is no longer a kernel-registered effect — it's a Cadenza program that
composes `exec`/`http`/… under a policy. `schedule` collapses too: a wakeup is a Cadenza program that `log_read`s
and re-runs (or an `exec` of a timer) — no dedicated kernel op.

## Fork 3 — RULED: DECISIVELY EXTENSIBLE (a new capability NEVER needs a kernel redeploy)

The capability set is **fully extensible from Cadenza**: a new capability is a new Cadenza program composed out of
the broad primitives, appended to the log — never a Rust change. There is NO fixed syscall table of named
effects to extend. This is the direct consequence of deploy-once-forever + "all functionality from Cadenza".

## The security pairing (MANDATORY design element, flagged by the operator)

`exec("any command")` + `http("any url")` are **maximally powerful** — so the guardrail is NOT the kernel (it
grants broad reach) but the **Cedar/capability policy layer** in Cadenza. The model:
- The kernel grants the broad primitives unconditionally to the genesis program.
- A Cadenza **policy program** (Cedar-backed, from L0) ATTENUATES what each agent/sub-program may `exec`/`http`/
  write — on-behalf-of delegation narrows the broad grant to a least-privilege subset per actor.
- So "broadest reach" (kernel) + "attenuated per-actor" (Cadenza policy) = powerful but constrained. The
  attenuation is itself a self-modifiable Cadenza program (policies evolve without a kernel redeploy).

**Fork (routed to operator):** where is the attenuation ENFORCED so it can't be bypassed? If policy is "just
another Cadenza program", a buggy/malicious interpret could skip it. Options: (a) the kernel calls a fixed
policy-check entrypoint before every `exec`/`http` (one hardcoded indirection — tension with "no hardcoded
kinds"); (b) capability tokens are unforgeable (the broad primitive requires a token only the policy program
mints); (c) trust the single-owner fold + audit log (every `exec`/`http` is a logged event, reviewable). Leaning
(b) — unforgeable capability tokens gate the broad primitives — but this needs an operator call.

## Consequences for the K-rungs (updates `-rulings.md`)

- **K0 (host-ABI compound-return)** unchanged as the critical path, BUT its shape simplifies under fork-1: the
  broad primitives take/return SCALAR/STRING/BYTES + a small record — the one compound that must cross is the
  `(List HostOp)` interpret result, and (per my reply to v-effects) if its consumer is a Cadenza peer over the
  shared runtime it crosses as a handle via `extern_abi_val_type` (may already work). So K0 may be far smaller
  than a general host-compound-marshalling build.
- **K1 (kernel skeleton)** re-scoped again: the kernel is `exec`/`http`/`log`/`fs`/`now` + "run the CLI-injected
  genesis program via `cdz_run`, hand it the broad primitives, execute the `(List HostOp)` it returns via a thin
  Cadenza executor". The old K1 `HostOp` drain-events transport stays dropped; the `HostOp` SUM type survives as
  the list-element shape (now `Exec`/`Http`/`Append`/… broad variants, not the narrow 4).
- **K2/K3** (msg/sub → Cadenza) unchanged — they compose over `log_append`/`log_read`.

## Proceeding

Design updated to the broad-reach + deploy-once-forever model. Reported the revised host-primitive set + the
security/attenuation pairing (with the enforcement fork routed). Next: author the `interpret.cdz` + the broad-
primitive `HostOp` sum as the K0 repro for v-effects; continue coordinating K0.
