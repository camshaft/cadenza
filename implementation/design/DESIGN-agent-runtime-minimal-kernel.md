# Minimal-kernel re-charter — the Rust host understands NO events (operator mandate, 2026-07-17)

**Owner:** v-agent-harness. **Supersedes the build direction of:** the L2/L3 Rust rungs (see "What this changes").
**Anchors to:** `DESIGN-agent-runtime-vision.md` (self-modification is the whole point). **Status:** DESIGN — audit
+ proposed minimal host interface + the Rust→Cadenza migration + the forks routed to the operator.

## The mandate (operator, verbatim excerpt)

> "The agent kernel written in rust really needs to be more minimal than it already is… I don't think the rust
> side should really understand any events. There should be a cadenza program that does that and tells the host
> what to do. We want this kernel to be as absolutely minimal as possible so it doesn't ever need to be deployed
> again after its initial version. The more we hardcode there the less the fleet can self modify. The absolute
> goal is to make it as tiny as possible so _everything_ can be modified."

Restated as the invariant this design must satisfy: **the Rust kernel is deployed ONCE and never again.** Every
behavior that might ever change — event schemas, message kinds, the inbox/ack rules, subscriptions, dispatch,
routing, compaction, policy — must live in Cadenza (which the fleet can self-modify by appending a new program
to the log), NOT in Rust (which requires a redeploy to change). The Rust side is a dumb executor of a tiny,
fixed, generic set of primitive host-ops. It never parses an event payload; it never knows what "message" or
"subscribe" means.

## Audit — what is in the Rust kernel today, and which side of the line it belongs on

`implementation/seed/crates/cdz-kernel/` (2166 lines). Classified against the mandate:

| File | Lines | Understands events? | Verdict |
|---|---|---|---|
| `lib.rs` — `Log` trait, `Event {seq, kind:String, payload:Vec<u8>}` | 71 | **No** — payload is opaque bytes | **KEEP** (already generic) |
| `file_log.rs` — length-prefixed file-backed `Log` | 256 | No | **KEEP** (a log backend, event-agnostic) |
| `dynamo_log.rs` — event↔item marshalling + DynamoDB backend | 128 | No (marshals opaque `{seq,kind,payload}`) | **KEEP** (a log backend) |
| `fold.rs` — the fold owner; **binds `model-request`/`model-response`**, drives the agent turn | 371 | **Partly** — hardcodes the model-effect event kinds + the record/replay of them | **SPLIT** (keep the generic driver; move the event-kind knowledge out) |
| `msg.rs` — `Message`/`Ack`, `inbox_for`, `reply_then_ack`, the `"message"`/`"ack"` codec | 513 | **YES** — the entire semantics of messaging | **MOVE → Cadenza** |
| `sub.rs` — `Subscription`/`Predicate`, `active_subscriptions`, `dispatch`, `"subscribe"` codec | 539 | **YES** — the entire semantics of subscriptions/dispatch | **MOVE → Cadenza** |

**Finding:** ~1050 lines (msg.rs + sub.rs) and the event-aware half of fold.rs are exactly the "event-specific
Rust" the operator flags. The log layer (`lib.rs`/`file_log.rs`/`dynamo_log.rs`, ~455 lines) is already correct —
it treats every event as an opaque `{seq, kind, payload}` and never interprets it. That opaque `Event` is the
right seam; the kernel below it should stay, and everything that decodes a payload should move above it, into
Cadenza.

## The target architecture — a tiny host + one Cadenza interpreter

```
   ┌─────────────────────────────────────────────────────────────┐
   │  CADENZA program (self-modifiable — lives IN the log)          │
   │    interpret(log_tail, new_event) -> [HostOp]                 │
   │    ── knows every event kind: message/ack/subscribe/dispatch/  │
   │       model-request/… ; the inbox rules; the policy; routing.  │
   └───────────────▲───────────────────────────┬───────────────────┘
                    │ events (opaque bytes)     │ HostOp commands
   ┌────────────────┴───────────────────────────▼───────────────────┐
   │  RUST KERNEL (tiny, generic, deployed ONCE)                     │
   │    the fold loop: read tail → call interpret → execute HostOps  │
   │    + the primitive host-ops below. Understands NO event kinds.  │
   └─────────────────────────────────────────────────────────────────┘
```

The kernel's whole job becomes: **tail the log → hand the tail (opaque events) to the Cadenza `interpret`
program → receive a list of primitive HostOp commands → execute them → loop.** The kernel does not know why it is
appending a given event or invoking a given capability; the Cadenza program decided that.

## The MINIMAL host-op interface (proposed — the fork the operator should rule on)

The primitive set the Rust kernel must provide. The design target is **four** ops; the Cadenza program composes
everything else (messaging, subscriptions, dispatch, the agent loop) out of them:

1. **`append(kind, payload) -> seq`** — add one opaque event to the log. (The only write.)
2. **`read_tail(from_seq) -> [Event]`** — read the ordered opaque tail from a cursor. (The only read.)
3. **`invoke(capability, request_bytes) -> result_bytes`** — perform ONE external effect named by an opaque
   capability token (a model call, a clock read, a build, an HTTP call). The kernel does not know what the
   capability *does* — it looks the token up in a capability table and runs the bound effect; the Cadenza program
   chose the token and built the request bytes. This is where "capability = effect-type" (vision) is enforced:
   the kernel refuses a token the current lease doesn't grant.
4. **`schedule(delay_or_predicate, program_ref)`** — ask to be re-entered later (a timer, or "when an event
   matching X lands"). This is the one op that lets subscriptions/wakeups exist without the kernel understanding
   them. *(Fork: is this even needed, or can scheduling be expressed as an event the interpret-loop re-reads? —
   see forks.)*

Everything else is Cadenza: `interpret(tail, new_event) -> [HostOp]` is a pure Cadenza function; the kernel calls
it. A message send is `append("message", <encoded by Cadenza>)`. The inbox is a Cadenza fold over `read_tail`.
Dispatch is Cadenza matching the new event against Cadenza-decoded subscriptions. A model call is
`invoke("cadenza:model/api", <prompt>)` with the record-as-event handled by Cadenza emitting an `append` after.

## What moves Rust → Cadenza (the migration)

- **msg.rs (all of it)** → a Cadenza module: the `Message`/`Ack` types, the codec, `inbox_for`, `reply_then_ack`.
  Cadenza already has sum types, records, and (via the metaprogramming vertical) enough to encode/decode. The
  kernel just `append`s/`read_tail`s opaque bytes.
- **sub.rs (all of it)** → a Cadenza module: `Subscription`/`Predicate`, `active_subscriptions`, `dispatch`,
  `matches`. Predicate matching becomes a Cadenza function over the opaque event; the kernel never matches.
- **fold.rs event-awareness** → Cadenza: the `model-request`/`model-response` record/replay becomes the Cadenza
  program emitting `append`s around an `invoke`. The kernel keeps only the generic "call interpret, run the ops"
  loop + the `invoke` capability plumbing (which reuses `cdz_run`'s host-op driver + `RunOpts::host_responses`
  for replay determinism — that part is already generic).
- **KEEP in Rust:** `lib.rs` (Log/Event), `file_log.rs`, `dynamo_log.rs`, and a slimmed `fold.rs` = the
  interpret-loop + the 4 host-ops. Target: the whole kernel well under ~500 lines, none of it event-aware.

## What this changes about the L2/L3 work already in flight

L2 (msg.rs) and L3 (sub.rs) were built as **Rust**. Under this mandate they are the wrong layer. BUT they are
gated-green and encode the exact semantics the Cadenza versions must reproduce — so the proposal is to **keep them
transiently as the executable SPEC / reference oracle** for the Cadenza port (a differential target: the Cadenza
`inbox_for` must match the Rust `inbox_for` on the same log), and **delete them once the Cadenza versions pass**.
L3c is queued at pr-sync and L3d is held; letting them land costs nothing and preserves the reference. *(Fork: keep-as-reference-then-delete vs. reject-and-delete-now — routed below.)*

## The real forks for the operator (routing these up per the assign)

1. **The exact host-op set.** Is the 4-op interface (`append`, `read_tail`, `invoke`, `schedule`) right? Candidates
   to cut: `schedule` (could be "the interpret-loop re-reads on every append + a Cadenza-held timer-event"), making
   it **3 ops**. Candidate to add: a `spawn` for concurrent fold-owners (or is that also just events?). **Fewer is
   better per the mandate — where's the floor?**
2. **How does Cadenza return HostOps to the kernel?** The host-op boundary today can't return a compound/List from
   Cadenza (known ABI gap — `host-op-cannot-return-string-or-compound-result`). Options: (a) the interpret program
   returns ops one-at-a-time via repeated calls (kernel calls `interpret` in a loop until it says "done"); (b)
   widen the host ABI so a Cadenza fn can return a List of ops; (c) Cadenza writes ops to a scratch log the kernel
   drains. **This is the load-bearing fork** — it decides whether the "everything in Cadenza" vision is reachable
   with today's ABI or needs an ABI change first (ties to v-effects/v-rust-backend).
3. **Where exactly is the Rust/Cadenza line for `invoke`?** The capability effects (model/clock/http) are
   inherently host-side (they touch the outside world). Does the kernel hold a *fixed* table of capability
   implementations (so adding a new effect-type IS a redeploy — violating "deployed once"), or a *dynamic*
   registry the fleet extends? A fixed table means new capabilities need Rust; that may be an acceptable floor
   (the OS analogy: syscalls are fixed, everything above is user space) — **operator to rule on whether new
   capability *types* may require a kernel bump.**
4. **Keep L2/L3 Rust as reference-then-delete, or reject/delete now?** (See prior section.)
5. **Bootstrapping:** the `interpret` Cadenza program itself lives in the log (so it's self-modifiable). What runs
   before the first program is appended? Proposal: a tiny hardcoded genesis that only knows "read the
   latest `program` event and call it" — the one irreducible bit of Rust that names an event kind. **Is one
   hardcoded `program`-kind acceptable, or must even that be indirected?**

## Proposed next steps (pending the operator's ruling on the forks)

1. This design + audit (here) → route forks 1–5 to the operator (concierge ask, this tick).
2. On ruling: write `DESIGN-agent-runtime-minimal-kernel-plan.md` decomposing the migration into gated sub-rungs
   (K1 the interpret-loop + host-op trait; K2 port msg.rs→Cadenza behind the differential oracle; K3 port
   sub.rs→Cadenza; K4 slim fold.rs; K5 delete the Rust event code).
3. Build the tiny kernel; port event logic to Cadenza rung by rung, each gated against the Rust reference until
   the Rust reference is deleted.
