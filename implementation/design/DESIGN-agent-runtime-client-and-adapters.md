# Client shape + external adapters — the "what ships where" boundary (operator refinement, 2026-07-17)

**Owner:** v-agent-harness. **Amends:** `DESIGN-agent-runtime-minimal-kernel.md` + `-broad-primitives.md`.
Records the operator's refinement of the cdz-agent client/CLI shape and the external-adapter pattern. Folds
into the deploy-once-forever minimal-kernel model.

## The refinement (operator, verbatim)

> "Note that in cdz-agent I don't think we want to have the cli do much. It should just bootstrap the log and
> then start up the daemon. You would then need to deploy a separate slack connector that was able to post to
> the log on behalf of users as well as get notified by the main daemon that a user received a message in slack.
> Other than that, I think, the client doesn't need anything else."

## The three deployables (the "what ships where" boundary)

The system is **three separate things**, all coordinating through the log — reinforcing tiny-kernel + everything-
else-composable:

1. **The tiny kernel (daemon).** The deploy-once-forever Rust host: reads the log, runs the Cadenza `interpret`
   program, executes the broad host-ops (`exec`/`http`/`log`/`fs`/`now`). Understands no events. (Design so far.)
2. **The CLI — MINIMAL, exactly two jobs:**
   - **(a) bootstrap the log** — initialize the event log, AND (per the fork-5 ruling) **inject the genesis
     program** into it (the CLI seeds the first program; the kernel has no hardcoded genesis).
   - **(b) start the daemon** — kick off the kernel to read the log and run.
   - **Nothing else.** No client-side event logic, no commands — all behavior is Cadenza in the log.
3. **External adapters — separate deployables, one per external I/O surface.** Each translates an external
   world ↔ log events. The archetype the operator named is the **Slack connector**:
   - **inbound:** a user's Slack message → append an event to the log **on behalf of that user** (ties to the
     Cedar on-behalf-of / capability model — the connector posts as the user, and Cedar governs what that
     user's events may authorize).
   - **outbound:** the daemon **notifies** the connector when a user should receive a Slack message; the
     connector sends it to Slack (daemon → connector → Slack).
   - So the connector is a **bidirectional Slack ↔ log adapter**, deployed separately from the kernel + CLI.

This is the **log-native analogue of the current fleet `slack-bridge`** — worth designing so the eventual
fleet-on-cdz-agent convergence reuses this adapter pattern (every external surface — Slack, GitHub, the model
API — becomes such an adapter, not kernel code).

## How this composes with the earlier rulings

- The adapter's inbound "post on behalf of a user" is an `exec`/`http`-driven append governed by a **Cedar policy
  doc in the log** (the attenuation ruling): the user's on-behalf-of grant is a Cedar doc; the connector's append
  carries the user principal; Cedar de-escalates what that event may cause.
- The adapter is NOT special to the kernel — the kernel just sees `log_append`/`log_read` events. "The daemon
  notifies the connector" is itself a log event the connector subscribes to (an adapter is a log participant,
  like an agent). So adapters need no new kernel primitive — they use the broad ones + the log.

## Forks routed to the operator

1. **Connector ↔ daemon notification mechanism.** How does the daemon "notify" the connector that a user should
   get a Slack message? Options: (a) the daemon appends a `notify` event (kind chosen by the Cadenza program)
   and the connector **subscribes** to the log (polls/tails for events addressed to it) — pure log-native, no new
   channel; (b) a direct callback/webhook the daemon `http`-POSTs to the connector. Leaning (a) — the connector
   is just another log participant that tails for its events — but confirm (it decides whether adapters need any
   out-of-band channel or are purely log-coupled).
2. **On-behalf-of ↔ Cedar mapping for the connector.** When the connector appends "on behalf of user U", how is
   U's principal + its Cedar grant expressed on the event so the daemon's interpret evaluates the right policy?
   (Ties to the L0 Cedar on-behalf-of work — likely the event carries a principal attribute Cedar keys on.)

## Consequence for the build plan (updates the K-rungs)

No change to K0 (done) / K1 (kernel skeleton) / K2–K3 (msg/sub → Cadenza). ADDS two later rungs, AFTER the kernel
runs interpret e2e:
- **KC (CLI):** the thin `cdz-agent` CLI = bootstrap-log + inject-genesis + start-daemon. Small.
- **KA (adapter):** the Slack connector as the first external adapter (log-native slack-bridge analogue), pending
  fork-1's notify-mechanism ruling. Reuses the existing `slack-bridge` learnings.

These are deployment/edge concerns — the CORE remains kernel + interpret. Proceeding on K1 (kernel skeleton)
next; KC/KA come after the kernel drives interpret end-to-end.
