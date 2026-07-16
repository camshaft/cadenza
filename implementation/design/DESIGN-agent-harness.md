# Design — a native Cadenza agent harness / runtime (Bedrock-direct, self-modifying, Cedar-authorized)

**Author:** v-agent-harness (vertical owner). **Audience:** the implementer picking up any increment,
the concierge/operator (the design forks below need operator rulings), and the sibling verticals this
leans on (v-peer-linking, v-effects, v-metaprogramming, v-runtime, v-verification).
**Status:** 🟡 **Increment 0 — DESIGN (this doc).** No harness code yet. This is the end-state
architecture, the hivemind alignment, the Bedrock-binding **gap analysis** (what Cadenza's host/peer
surface can do TODAY vs what's missing), the Cedar principal/action/resource model for
acting-on-behalf-of, and the increment sequence. Written 2026-07-16 against `trunk` @`6e40be0a9`.

**Operator directive (2026-07-16, verbatim intent):** "get another vertical to own building an agent
harness using Cadenza and getting away from using Claude Code and calling Bedrock directly. And then
the whole agent runtime would be self-modifying and have evolvable toolchains. I want to use Cedar for
permissions where we can grant agents to work on behalf of users and stuff. A lot of the ideas I've
captured in here: https://github.com/camshaft/hivemind"

---

## 0. What this replaces, and why it's the ultimate dogfood

The fleet today (and hivemind's own worker) runs its agent loop by shelling out to the **headless
`claude` CLI** (`claude -p --input-format stream-json --output-format stream-json`), authenticated to
Bedrock via `CLAUDE_CODE_USE_BEDROCK=1` and the task role. Verified in the hivemind reference repo:
`crates/hivemind-daemon/src/worker.rs` drives exactly that persistent process (`backend: "claude"`, the
only backend implemented), and `crates/hivemind-daemon/Dockerfile` bakes in Node + the `claude` binary
solely to host it. The enrichment Lambda (`crates/lambda-enrich/src/main.rs`) already calls Bedrock's
`invoke_model` directly — that is the pattern we reproduce, but in Cadenza and for the *agent's own
reasoning loop*, not just enrichment.

**The end state:** the agent loop (read-inbox → build-context → call-model → parse-tool-calls →
execute-tools → append-events → repeat) is authored **in Cadenza**, calls **Amazon Bedrock directly**
(Converse / InvokeModel over SigV4), authorizes every tool invocation and resource access through a
**Cedar** decision, supports **on-behalf-of delegation** (a user grants an agent scoped authority), and
can **modify its own code and grow/replace its tools at runtime** (leaning on Cadenza's
quote/eval/`Ast` metaprogramming + load-time expansion).

This is the flagship dogfood: the fleet building its own harness in the language it compiles. Like the
compiler-ml port, **language gaps found here are the point — REPORT/FIX them (file a `.sexp` into the
queue for the breaker/PM path), don't paper over them.**

---

## 1. Where this sits in hivemind (alignment)

Digest of `VISION.md` / `ARCHITECTURE.md` / `DECISIONS.md` / `BUILD_SPEC.md` (reference clone at
`/tmp/hivemind-ref`):

Hivemind is a **substrate, not a runtime.** Its spine is CQRS/event-sourcing: one immutable append-only
**event log** is the source of truth; everything else (a memory's current state, a task's status, an
agent's local view) is a deterministic **fold** (projection) over that log. A long-lived **daemon**
(one per agent identity) tails the log into a local DuckDB-over-Parquet view, serves reads over a Unix
socket with zero round-trip, and forwards writes to the central SigV4-authenticated ingest API. CLI /
MCP / Slack are thin adapters over that daemon.

Crucially, **hivemind deliberately does NOT specify the agent's reasoning loop** — it is
"framework-agnostic, Claude Code first-class." The worker-loop contract lives in `BUILD_SPEC.md §3f`
(`hivemind work`): a cheap supervisor loop, blocked on the daemon's fresh inbox projection, that DRIVES
a persistent agent process (today `claude`) over stdin — reply-then-ack per message so a crash never
drops a reply.

**So our harness plugs in as the agent runtime the daemon drives** — a client of the daemon's local
socket (it reads the inbox / task lifecycle by **pull**, with the presence hub as an optional wake, and
writes results back as **immutable events** through the daemon). We are the piece that replaces the
`claude` subprocess with a Cadenza-authored loop that calls Bedrock itself.

**Three things are net-new relative to the hivemind docs (ours to define):** (a) the agent's own
model-call path — the docs scope Bedrock strictly to enrichment + embeddings, never the agent's
reasoning; (b) **Cedar** authorization — the term appears NOWHERE in hivemind's docs (its authz is
IAM/SigV4 + per-request body-fetch authorization + approval-gate events); (c) self-modification /
evolvable toolchains as code-level concepts (hivemind's analogue is the org-level self-reinforcing
scheduler, not an agent rewriting its own toolchain). These three are exactly this vertical's mandate,
and they are additive to hivemind rather than in tension with it.

**Boundary we will respect:** the harness talks to the hive through the daemon's IPC, not directly
against AWS DynamoDB/S3 — except the ONE outbound call hivemind also makes directly: the model API
(Bedrock). We do not re-implement the event log; we consume and emit its events.

---

## 2. The Bedrock-binding gap analysis (this is the load-bearing Increment-0 finding)

**The first question the charter poses:** can Cadenza today make an authenticated HTTPS/AWS-SigV4 call
to Bedrock, and what's missing? Answer, from reading the host/peer ABI surface
(`implementation/seed/crates/rcdzc/src/backend/wasm/host.rs`, `.../cdz-run/src/lib.rs`):

### 2.1 What EXISTS today

- **A host-effect boundary.** An effect an entrypoint delegates to the host (`(host (E) …)`) lowers to
  a `Core::HostCall`, collected into a deterministic import set and emitted as an imported boundary
  function (`collect_host_imports`). The runner (`cdz-run`) binds each host import and feeds it a
  recorded response in call order (`bind_host_imports` + `RunOpts::host_responses`). This is the E2h
  path v-effects shipped.
- **A peer boundary, unified with effects (v-peer-linking's U1–U4).** `(effect Math …)` + `(bind Math
  "cadenza:pkg/iface")` routes an escaping effect to a *separately-compiled Cadenza peer* over a shared
  runtime. Precedence: in-source default < compile-request `--bind` override < in-program `(handle …)`.
- **A peer op's String RESULT crosses by handle.** `extern_abi_val_type` gives a runtime-owned compound
  (String/List/Record/tuple/sum/Map/Set/Bytes/BigInt/Rational) an opaque `u32` heap handle into the
  shared runtime — a peer op can **return** a String today (probed e2e — see §2.1a).
- **Scalar host params/results + a CONST String argument.** `abi_val_type` maps every aliased scalar
  (Bool/Char/f32/f64/s8…u64). A host op can take a *constant* `String` argument (`HostParam::Str`, the
  (ptr,len) lift, const-fold path). The runner can coerce a `String` *response* (`bind_host_imports`
  handles `Val::String`) — but the compiler-side host RESULT ABI (§2.2) doesn't emit one.

### 2.1a ⚠ CORRECTION (probed e2e 2026-07-16) — the String-crossing matrix is ASYMMETRIC

An earlier draft of §2.1 claimed "a peer op CAN take and return a String today." **That is wrong — only
the RESULT direction works.** Built `cdz`/`cdz-run` and probed every crossing; results
(`issues/string-crossing-matrix-blocks-model-call-shape.md` has the full table + repros):
`✅ peer RESULT=String` (ran to a value); `🔴 peer ARG=String` (CDZ0201, inbound rope handle not emitted);
`🔴 entrypoint RESULT escapes String` (resource-escape lacks the peer import); `🔴 host ARG=String
non-const`; `🔴 host RESULT=String` (§2.2); `🔴 entrypoint PARAM=String`. **A `String` crosses NO boundary
in a runnable `(String -> String)` model-call shape today except a peer RESULT** — so BOTH routes below
need ABI work, and Route B's real critical-path unblock is the **peer String-ARGUMENT** cell, not the
host-result widening. §2.2–§2.3 below are kept for the host-result cell but read them through this matrix.

### 2.2 What's MISSING — the gate to Bedrock-direct

**A genuine HOST op cannot RETURN a String or compound today.** `abi_val_type` (host.rs:59) matches
only scalars and falls through to `_ => None` for `Ty::String` and every compound; `first_unrepresentable_host_op`
(host.rs:~614) explicitly declines a host op with a `String`/`list<u8>`/compound result because it "needs
the memory + list-lifting envelope" that the peer path has but the host path does not. The in-code
comment says it outright: *"a NON-scalar non-Unit result (a `String`, a compound) is NOT [representable]."*

This is the exact "STILL OPEN" constraint the earlier CodeAct spike flagged
(`[[cadenza-agent-harness-codeact-spike]]`): *"a host op RETURNING String/List isn't expressible."*

**Why it gates Bedrock:** a model call is fundamentally `(String prompt / JSON request) -> (String
completion / JSON response)`. A Bedrock Converse/InvokeModel binding is a boundary op that must **return
a String** (the completion text, or a JSON body to parse). Today that op declines at compile time.

### 2.3 The two routes to close it (a real design fork — see §6, ask #1)

- **Route A — widen the HOST-result ABI to String/compound (compiler change in `rcdzc`).** Teach the
  host boundary the memory + list-lifting envelope the peer/closure-`Bytes` path already has, so
  `abi_val_type` (or a host-specific successor) admits a `String`/`Bytes` result. This is the "one
  compiler change between the spike and tools-compose-as-host-capabilities" the spike named. It is the
  clean, general fix and unblocks every future host op that returns text — but it is a non-trivial
  ABI widening in a frozen-ish surface, and needs v-peer-linking / v-effects coordination.
- **Route B — model Bedrock as a Cadenza PEER, not a host op.** A `cadenza:bedrock/api` peer that
  exposes `converse`, `(bind Bedrock "cadenza:bedrock/api")`, and a shim doing SigV4 + HTTPS. A peer op's
  String RESULT works today (§2.1a), so the **completion** comes back fine — but per the matrix the
  **prompt ARGUMENT** (`String` in) declines (CDZ0201, cell #2), so a naive `(converse (-> String
  String))` does NOT compile yet. Route B's critical-path unblock is the **peer String-argument** emit
  (the mirror of the working result path) — a smaller lift than the host-result widening. Interim
  workaround until #2 lands: pass the prompt as a tuple/record of scalars or via a side channel, take the
  completion back as the peer String RESULT, and consume it IN-PROGRAM (so the entrypoint returns a
  scalar, sidestepping cell #3).

**Recommendation to the operator (revised post-probe):** Route B is still the right bring-up, but it is
NOT zero-compiler-change as first stated — it needs the **peer String-argument** cell built (route to
v-peer-linking; it's the mirror of the working result path, a focused lift). Route A (host String result)
is the parallel durable fix. Neither alone yields `String -> String` today. Sequence: (1) v-peer-linking
builds the peer String-arg emit; (2) Route B ships a real Bedrock call; (3) Route A follows so the SigV4
edge can migrate into Cadenza. All three tracked in the matrix issue.

### 2.4 The SigV4 / TLS reality regardless of route

Neither route makes Cadenza speak TLS or compute an AWS SigV4 signature *itself* yet — there is no WASI
`http`/`sockets` binding in this tree (`grep` found none). The signing + socket lives in a host/shim
either way. BUILD_SPEC §3a confirms hivemind itself uses the `aws-sigv4` crate over `reqwest` (~15
lines/request), explicitly NOT Smithy codegen for v1. So the harness's Bedrock edge is: **Cadenza loop
→ boundary op → (host runner OR peer shim) that holds the `aws-sigv4` signer + `reqwest`/hyper client.**
A future increment can explore a Cadenza-native WASI-http capability (v-runtime), but that is not on the
critical path and should be its own design.

---

## 3. The agent loop, in Cadenza

The loop is a small state machine. Modeled on `BUILD_SPEC §3f` but authored in Cadenza:

```
loop:
  msgs      = inbox.peek(limit)         # a daemon IPC read (pull); empty ⇒ sleep on the wake signal
  for msg in msgs:
    ctx     = build_context(msg, memory, tools)   # system preamble + tool schemas + forward type-env
    resp    = Bedrock.converse(ctx)               # THE boundary op (§2) — returns String/JSON
    calls   = parse_tool_calls(resp)              # resp → a list of (tool, args) Cadenza values
    for (tool, args) in calls:
      decision = Cedar.authorize(principal, action=tool, resource=args, context)   # §4
      result   = if decision.permit then dispatch(tool, args) else deny(decision)
      ctx      = append(ctx, tool_result(result))  # host-authored fact, never a forged model turn
    inbox.send(reply); inbox.ack(msg)             # reply-then-ack (crash never drops a reply)
```

**Tool dispatch is an effect.** Each tool is an effect operation; the harness `(handle Tools …)` around
the loop routes a dispatch to its implementation (a host op, a peer, or a hive-federated `tools/call`).
This reuses v-effects wholesale and makes the tool set a first-class, swappable handler stack — which is
exactly the seam self-modification (§5) grows.

**Tool calls are Cadenza values.** A model's tool-call block parses into an `Ast`/sum value (v-meta's
`Ast` sum), so the loop pattern-matches structured tool calls rather than string-munging. `parse_tool_calls`
is where the model's JSON/XML tool-use surface becomes typed Cadenza data.

**Context hygiene (durable lessons from the spike, `[[cadenza-agent-harness-codeact-spike]]`):** author
facts into the HOST channel (a `tool_result`), never forge the model's action turns; prune failed
attempts; carry a forward "type-environment" block PAST the cache breakpoint; persist harvested
resolve-type facts so a re-derive degrades to a cache miss, not a wrong answer.

---

## 4. Cedar — the on-behalf-of authorization model

**Cedar is already vendored in this tree** (`cedar-policy = "4"` in `cadenza-syntax/Cargo.toml`), and —
importantly — Cadenza already has a **Cedar SURFACE**: `cadenza-syntax/src/cedar.rs` parses a `.cedar`
policy into the canonical arena and prints it back (`(cedar-policyset …)` node vocabulary,
arena-idempotent round-trip). But that surface is *syntax only* — "we do NOT evaluate policies, and
Cadenza bakes in no authorization engine." So an agent can *construct and rewrite* Cedar policies with
the same structural-editing tools it uses on any arena, but **evaluating** an authorization request is
net-new.

### 4.1 The principal / action / resource / context model

- **Principal:** the acting identity. Two shapes:
  - a plain agent: `Agent::"agent:v-cad"`.
  - an agent **acting for a user** (delegation): the principal is the agent, but the `context` carries
    the delegation (`context.on_behalf_of == User::"user:cameron"`) and the request is authorized
    against BOTH the agent's own policies AND the user's delegation grant (intersection — the agent can
    do only what it is *and* what the user delegated). Cedar's `is`/`in` + a `context` condition
    expresses this cleanly without a bespoke engine.
- **Action:** the tool being invoked (`Action::"tool:write-file"`, `Action::"tool:bedrock-converse"`,
  `Action::"resource:read-memory"`). Every tool dispatch AND every hive resource access is an action.
- **Resource:** what the action touches — a file path, a memory scope, a task, a peer component. Modeled
  as a typed entity (`File::"/repo/src/foo.rs"`, `MemoryScope::"private"`, `Task::"task:123"`).
- **Context:** the delegation (`on_behalf_of`), the deployment/team, time, and any request attributes a
  `when`/`unless` clause conditions on.

### 4.2 Delegation ("grant an agent to work on behalf of a user")

A user issues a **delegation grant** = a scoped Cedar policy set (permit action-set A on resource-set R
when `context.on_behalf_of == that user` and the principal is the granted agent), plus an expiry. The
grant is itself an immutable hive event (a memory), so it is auditable and revocable by superseding
event. At authorize-time the harness evaluates: `agent's-own-policies ∩ (user-delegation where
on_behalf_of == user)`. This is the safe reading of "on behalf of": an agent acting for a user is
bounded by the *narrower* of the two.

### 4.3 Where the evaluator lives (a design fork — see §6, ask #2)

Cadenza has the Cedar *syntax* but no *evaluator*. Options:
- **4.3-A — host op.** `Cedar.authorize(request) -> Decision` is a boundary op backed by the
  `cedar-policy` crate's real evaluator (host-side). Fast to ship, correct engine, but authorization is
  non-Cadenza. (Blocked on the same String/compound-result ABI gap as §2 if the request/decision cross
  as compounds — another forcing case for Route A.)
- **4.3-B — evaluate Cedar IN Cadenza.** Author a Cedar evaluator over the `(cedar-policyset …)` arena
  in Cadenza itself (the arena is already the representation; this is "just" a fold + a small expression
  evaluator). Maximally dogfood, provable (ties to v-verification), but a real build. Likely a later
  increment after the host-op version proves the model.

**Recommendation:** ship **4.3-A** (host op over `cedar-policy`) for correctness + speed, and treat
**4.3-B** (Cadenza-native evaluator) as a flagship self-verification target with v-verification — a
self-modifying agent that can PROVE its next Cedar state still denies what it must is the safe form of
self-modification.

---

## 5. Self-modification + evolvable toolchains (design LAST)

Most speculative; sequence it after the loop + Bedrock + Cedar basics work. The seams:

- **Tools are handler stacks (§3).** "Evolve the toolchain" = install/replace a `(handle Tools …)` arm
  at runtime. Because a tool is an effect op and the dispatch is a handler, adding a tool is adding a
  handler arm — no core change. A new tool's *implementation* can be authored as Cadenza source, quoted
  to an `Ast`, compiled (`compile_component`), and composed as a peer (the package-linking path).
- **Self-modifying code via metaprogramming.** v-meta's quote/quasiquote/`eval`/`Ast` + the load-time
  expansion machinery let the harness construct a new version of its own loop or a tool as data, compile
  it, and swap it in. The compile-repair loop the spike proved (a real `CDZ0203`/`CDZ0101` diagnostic
  fed back → converges) is the success engine: the model needn't be one-shot correct, just correctable.
- **Cedar-gated self-modification (the safety story).** Every self-modification is itself an action
  (`Action::"self:replace-tool"`, `Action::"self:rewrite-loop"`) authorized by Cedar, and — the strong
  form — accompanied by a v-verification proof that the new state preserves a stated invariant (e.g. "the
  new Cedar policy still forbids `tool:delete-prod`"). This is where v-verification is a natural partner.

Self-modification leans on v-meta guarantees; **note v-metaprogramming** before building it (the JSX
eval-core recursion-guard is a known open there — see `[[metaprogramming-vertical-log]]`).

---

## 6. Design forks for the operator (route to concierge as `ask`s)

These are the real forks this design can't unilaterally resolve; each has concrete options so the
concierge can route a one-line decision:

> **RESOLVED (2026-07-16, all 4 concierge-confirmed):** (1) Bedrock — shipped as **option (c)** the
> embedder (a third route: a naive peer/host String op couldn't answer the handle-crossing boundary; see
> §7), with Route A the durable end-state. (2) Cedar — **(A)** host op over `cedar-policy` (shipped);
> (B) the Cadenza-native evaluator is a later v-verification flagship. (3) Crate home — **in-tree**
> (`implementation/seed/crates/cdz-agent`, workspace-excluded). (4) Scope — **subprocess-only** first
> (replace the `claude` subprocess; keep hivemind's daemon/log/tools). Q1 (start Inc-4 now) + Q3
> (proven-self-mod in the first cut) remain with the operator.

> **DESIGN SESSION (2026-07-16, operator live via concierge) — fleet-convergence fork LOCKED:**
> cdz-agent is the fleet's own **execution substrate end-state** — fleet agents will *eventually become*
> cdz-agent instances (operator: "eventually we replace ALL agents with cdz-agents, but we have to build
> it first"). Explicitly **build-first, migrate-later**: converge the FORMAT + capability NOW (the
> inbox-driver binary already reads the fleet inbox format — keep it there), but do NOT migrate the fleet
> yet; the substrate cutover is a later, deliberate, proven migration. **Inc-4 is designed toward "a
> cdz-agent can eventually run a fleet role"** (drive an inbox loop, author/load tools, be Cedar-gated).
> Operator-confirmed leans to proceed on: author-**new-tools** MVP before rewrite-own-loop;
> **content-addressed** Cadenza tool-modules; **gate-first, prove-later**; the Rust embedder as the
> approved stopgap. STILL OPEN (escalated to operator): the **self-mod SCOPE** fork — (A) author/load new
> peer TOOLS with a fixed loop [MVP lean] vs (B) the agent rewrites its OWN loop/handlers. No Inc-4 code
> until that scope call + a greenlight.

1. **Bedrock binding route (§2.3):** (A) widen the host-result ABI to String/compound in `rcdzc` —
   clean/general/durable, non-trivial ABI change, needs v-peer-linking; (B) model Bedrock as a Cadenza
   peer via a tiny non-Cadenza SigV4 shim — zero compiler change, ships now, leaves a non-Cadenza edge;
   (Recommend: B for bring-up, A in parallel as the end state.)
2. **Cedar evaluator (§4.3):** (A) host op over the vendored `cedar-policy` crate — correct/fast,
   non-Cadenza authz; (B) author a Cedar evaluator in Cadenza over the existing arena — max dogfood +
   provable, a real build. (Recommend: A now, B as a v-verification flagship.)
3. **Crate home:** does the harness live in-tree as a new `implementation/` crate (like the CodeAct
   spike's `cdz-agent-harness`), or in the hivemind repo as the `backend: "cadenza"` worker alongside
   `backend: "claude"`? (Recommend: prototype in-tree against committed `trunk` in a worktree — the spike
   learned building against a moving main hits pre-existing WIP; then upstream to hivemind's daemon as a
   new backend.)
4. **Scope of "get away from Claude Code":** does the harness replace the `claude` *subprocess* only
   (Cadenza loop calling Bedrock, keeping hivemind's daemon/log/tools), or also aim to replace the daemon
   itself over time? (Recommend: replace the subprocess first; the daemon is hivemind's substrate and is
   out of this vertical's scope.)

---

## 7. Increment sequence

> **STATUS (2026-07-16): Inc 0–3 SHIPPED + hardened; Inc 4 in DESIGN-ALIGNMENT (operator live).** The
> fleet-substrate direction is locked (§6); the self-mod SCOPE fork is the last open gate before Inc-4
> design/build. The Bedrock binding
> shipped as a THIRD route the fork below didn't list — **option (c): a custom cdz-run EMBEDDER** — not
> the peer shim (Route B) originally planned here. Why: a Cadenza String peer op crosses as a `u32`
> runtime HANDLE and the provider imports the value-heap runtime, so a naive WASI `converse(string)->string`
> peer can't answer it; instead the embedder binds `cadenza:model/api`.converse to a Rust HOST CLOSURE
> over the shared runtime (reads the prompt rope with `str-get`, calls the model, mints the completion
> with `str-new`). This keeps the agent loop pure Cadenza with the model call as the only non-Cadenza
> edge; when the host-String-result ABI (Route A) lands it can collapse to a cleaner host binding.

- **Inc 0 — this design doc.** ✅ Landed. Forks §6 routed + all 4 concierge-confirmed (see §6 status).
- **Inc 1 / 1b — Bedrock-direct.** ✅ Landed as **option (c)**: `cdz_run::run_agent` (the embedder
  runner, binds `converse` to a host closure over the shared runtime) + the `implementation/seed/crates/
  cdz-agent` crate (`mock_converse` for tests; `bedrock_converse` behind `--features bedrock`, real
  `aws-sdk-bedrockruntime` InvokeModel). rcdzc `u7`/`u8` pin the String peer boundary; cdz-agent's CI
  job runs the embedder e2e. Route A (host-String-result ABI) remains the durable end-state (v-peer-
  linking's task), which would let (c) collapse to a host binding.
- **Inc 2 — the loop.** ✅ Landed: `implementation/agent-harness/` (a Cadenza package — `loop.cdz`, the
  `Model`/`Tools`-effect recursive loop) + the `cdz-agent` DRIVER binary that reads a real fleet inbox
  and drives each message through the loop (reading the body via an `Inbox.next` effect, reporting the
  model's actual completion). Generic `run_agent_hosted` binds N host ops (inbox + model + cedar).
- **Inc 3 — Cedar authorize (4.3-A).** ✅ Landed: `cdz-agent/src/cedar.rs` (the real `cedar-policy`
  evaluator) + `authz-loop.cdz` (every tool dispatch performs `Cedar.authorize` and dispatches only on
  allow) + `cedar_authorizer`/`cedar_delegated_authorizer` (fail-closed; on-behalf-of = agent ∩ user
  delegation intersection). Pinned e2e: permit, forbid, on-behalf-of both directions, malformed→deny.
- **Inc 4 — self-modification / evolvable toolchains (§5).** ⏸️ DESIGN-ALIGNMENT IN PROGRESS (operator
  live via concierge, 2026-07-16). Direction now partly locked (§6 DESIGN SESSION): cdz-agent is the
  fleet's execution substrate end-state, build-first/migrate-later, and Inc-4 is designed toward "a
  cdz-agent can eventually run a fleet role." Confirmed leans: author-**new-tools** MVP, content-addressed
  Cadenza tool-modules, gate-first-prove-later, Rust embedder stopgap. THE remaining gate is the **self-mod
  SCOPE** call (escalated to the operator): (A) author/load new peer TOOLS with a fixed loop [MVP lean] vs
  (B) the agent rewrites its OWN loop/handlers. **No Inc-4 code until that scope call + a greenlight.** When
  it starts: add `rcdzc` as a `cdz-agent` dep (concierge pre-cleared) for runtime compile-a-new-tool + the
  compile-repair loop; Cedar-gate every self-mod (`Action::"self:add-tool"` …); v-verification for the
  proven form (§5).

Each increment reports language gaps it hits (REPORT/FIX, not work-around) and lands with gate coverage
that pins its invariant. (This vertical's probes drove several v-effects + v-peer-linking fixes: the
peer String argument/result-escape emit, multi-peer fused resource envelope, and the effectful-helper-
in-a-recursive-self-call specialization family.)

---

## 8. Coordination (notes to send as increments approach)

- **v-peer-linking / cross-component-interop** — Route A (host-result ABI widening) and the Bedrock peer
  shim's interface shape. THE most important early partner (§2). `[[cross-component-interop-workstream]]`,
  `[[peer-linking-vertical-log]]`.
- **v-effects** — tool dispatch as effect; the loop's `(handle Tools …)`; host-vs-peer routing.
  `[[index-effects-capabilities]]`.
- **v-metaprogramming** — self-modification via quote/eval/`Ast`; the compile-repair loop.
  `[[metaprogramming-vertical-log]]`.
- **v-runtime** — a future Cadenza-native WASI-http capability (off critical path); host capabilities.
- **v-verification** — provably-safe self-modification; a Cadenza-native Cedar evaluator with an invariant
  proof (§4.3-B / §5).
- **v-guide** — an eventual showcase surface once the loop runs.

## 9. References

- Charter: `.claude/fleet/queue/seed-agent-harness-vertical.md`.
- Hivemind (reference clone `/tmp/hivemind-ref`): `VISION.md`, `ARCHITECTURE.md`, `DECISIONS.md`,
  `BUILD_SPEC.md` (§3a auth/SigV4, §3f `hivemind work` worker loop, §3d/§3e Bedrock router + MCP gateway).
- CodeAct spike: `[[cadenza-agent-harness-codeact-spike]]` (the in-process compile/execute/repair seams;
  the String-result-ABI open constraint).
- Cross-component interop: `implementation/design/DESIGN-cross-component-interop-rcdzc.md` (U1–U4:
  effect-bound-to-peer, the precedence ladder, compound-by-handle).
- Effects: `implementation/design/DESIGN-effects-rcdzc.md` (§2.4 `host` delegation, E2h).
- ABI reality: `implementation/seed/crates/rcdzc/src/backend/wasm/host.rs` (`abi_val_type`,
  `extern_abi_val_type`, `first_unrepresentable_host_op`); `.../cdz-run/src/lib.rs` (`bind_host_imports`,
  `RunOpts::host_responses`).
- Cedar surface: `implementation/seed/crates/cadenza-syntax/src/cedar.rs` (`(cedar-policyset …)` arena).
