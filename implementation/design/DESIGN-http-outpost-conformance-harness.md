# DESIGN — http-outpost gateway conformance harness

**Status:** proposed (v-gateway-conformance, 2026-09-10). Supersedes the test-*driver* model of
`DESIGN-http-outpost-integration-harness.md` (#8651). The black-box conformance goal stands; the *mechanism*
is a **declarative scenario corpus + a Rust driver**, modeled directly on the platform integration tests
(`cdz-platform-itest`), NOT a Python test suite.

## Operator directives (verbatim, 2026-09-10)

First:
> "we need to get another agent to build a control server test harness. and then set up a testing environment
> in nix where we can have test programs that make requests to the control server to do things and then make
> http requests against the gateway and assert they return the right thing … we should just have a big corpus
> of tests that we can run against a real http gateway and assert it conforms to the expected behavior."

On the gateway's essence:
> "essentially what the gateway is is a dynamic http server where the control server pushes wasm programs to
> it and it hot-reconfigures on the fly. so the external interface for it is just http requests. and the the
> control surface is simply just pushing router changes and the gateway reconfigures and starts routing any
> new requests to those handlers. the handlers also need to be able to send messages to the control server and
> that would need to handle those and respond. the control server needs to know which handler id is making the
> request and have as much info about the request so it can correctly route it."

On the test mechanism (this doc's core pivot):
> "i'm actually wondering if we even need this to be in python, right? what if we did the same as the platform
> integration tests and just had a declarative interface and then wrote a driver in rust that executed those
> commands against the (currently) 3 servers. … like how much branching do we really need? it's just 'make a
> http request here', 'make one over here', 'assert the response was this'."

Plus: build a **mock control server in Rust** the driver can inject messages into + change behavior on;
**automatically inject Cadenza programs** like the platform conformance suite — painless, no hand-rolled
Cadenza builds.

---

## 1. Goal & non-goals

**Goal.** A growing **conformance corpus** of **ML-surface run specs** (`*.ml`, like the platform
`harness-runs`), run by a **Rust driver** against a **real, STOCK, running HTTP gateway** wired to a real CAS
store and a real (behavior-mocked) control server — nothing internal to the gateway is mocked, so a pass means
genuine end-to-end conformance. Each run is a near-linear sequence of steps:

1. **arrange** — inject control-plane state (point the gateway at a root-router program, live-swap it, seed
   config, prime `control.send` responses);
2. **act** — make real HTTP (and later WebSocket) requests at the gateway;
3. **assert** — on observable outcomes: HTTP status/headers/body, that a live-swap took effect, that a
   `deny`/413/504 floor fired, that a `control.send` was delivered to control with the right handler id +
   request context, etc.

**Why declarative + a Rust driver (not Python).**
- **Most language-neutral.** The standing directive is "e2e → conformance, not `#[test]`; don't lock tests
  into a language." A *declarative* corpus is data, not code — it locks into *no* language. (Python would
  just swap a Rust lock-in for a Python one, plus a separate out-of-flake toolchain.)
- **Matches the precedent the operator keeps pointing at.** `cdz-platform-itest` = a declarative spec
  resolved + encoded, run by a built-once Rust binary, verdict = exit code. We mirror it.
- **Scenarios barely branch.** "make a request here, make one there, assert the response" is a linear
  step-list. The only non-linear need is *wait-until-reconfigured* (a retrying expect for async live-swap
  propagation) — one primitive, not control flow.
- **Reuses the harness machinery** (auto-inject Cadenza, seeded CAS, name→hash manifest) with no second
  toolchain.

**The harness tests a CONTRACT, not an implementation.** The gateway is expected to be rewritten from
scratch. This doc pins the *interfaces* it depends on — §4 control-plane wire contract, §5 CAS contract, §6
program-injection, §7 boot — and treats today's `cdz-http-gateway` internals as **replaceable**. Any gateway
that honors the contracts passes the same corpus.

**Non-goals.** Not a Python suite. Not Rust `#[test]`s of gateway internals (the existing pile is migrated
into scenarios then retired, §9). Not a production control server (ours is a **mock**: it speaks the real
gateway-facing protocol, but its behavior is driven by the scenario, not by real control logic).

---

## 2. The gateway's essence (operator framing)

The gateway is a **dynamic HTTP server that the control server reconfigures on the fly.** Its whole external
surface is **just HTTP requests** — clients see nothing else. Its **control surface** is equally simple:
control **pushes router changes** (a new root-router program, resolved from CAS by hash), and the gateway
**hot-reconfigures with no restart**, routing every subsequent request through the new handlers. Two
consequences the harness leans on:

- **Reconfiguration is live and observable purely through HTTP.** A scenario pushes a router change via
  control, then observes it by making an HTTP request — no restart, no other configuration surface.
- **The control link is bidirectional.** Handlers can **send messages up to control**, which handles them and
  **responds**; the response routes back to the exact handler invocation (§4). A request/response leg, not
  fire-and-forget.

The **control server is the sole configurator**: the gateway boots knowing only where control is; everything
else (routes, CAS location, credentials) arrives over the control link and can be live-swapped.

### 2.1 Topology

```
   ┌──────────────────────── Rust conformance driver (cdz-http-conformance) ───────────────────────────┐
   │  consumes ONE run's binary-AST spec (rewritten *.ml); spins up 3 child processes; executes its       │
   │  requests + inline asserts, runs the optional checker over the log; verdict = exit code.             │
   │                                                                                                     │
   │      command control        ┌────────────────────┐    control-plane ws     ┌──────────────────┐    │
   │      (inject + observe) ────▶│ mock control server │◀───────────────────────│  HTTP gateway    │    │
   │                             └────────────────────┘  ControlConfig/root-      │ (boot-from-       │    │
   │                              (cdz-http-control-mock)  router push down;       │  control)         │    │
   │                                                       ControlUp up            └────────┬─────────┘    │
   │      real HTTP/WS requests ──────────────────────────────────────────────────────────▶│ :gw_port     │
   │                                                                                        │              │
   │                                                       resolve program by hash  ┌───────▼────────┐     │
   │                                                                    GET /{hash} ▶│ CAS HTTP server│     │
   │                                                                                 │ (cdz-cas-http) │     │
   │                                                                                 └────────────────┘     │
   │   Cadenza programs (routers, handlers) are compiled by NIX ahead of time and pre-seeded into the CAS  │
   │   store; scenarios name them; a build-produced manifest maps name → ProgramHash.                      │
   └───────────────────────────────────────────────────────────────────────────────────────────────────┘
```

Three SUT processes, all loopback on driver-chosen ephemeral ports:

- **CAS HTTP server** — `cdz-cas-http` (EXISTS, blessed). Serves compiled programs by hash. Started
  disk-backed at the nix-produced, pre-seeded store dir (§5).
- **Mock control server** — `cdz-http-control-mock` (**NEW, the main Rust artifact I build**). Speaks the
  real control-plane wire protocol toward the gateway (§4); its behavior is set + observed by the driver via
  a small admin channel (§3.3). Holds scenario-driven state.
- **HTTP gateway** — `cdz-http-gateway` in **boot-from-control** mode: told only the control address, it
  dials control, applies its `ControlConfig` (CAS url + credential + root-router hash), resolves programs
  from CAS by hash, and serves. (Unbuilt gap today — §9. The harness pins the contract; the gateway rewrite
  implements it.)

---

## 3. The test interface: ML-surface run specs

Modeled directly on the platform integration tests (`implementation/seed/crates/cdz-platform/harness-runs/
*.ml` + `mkHarnessRun`). **Each conformance run is one `*.ml` file** — a Cadenza value (ML surface)
describing the whole run: a `config` that sets the SUT up, a `requests` list that drives it with inline
`expect` assertions, and an optional `checker` program that judges the recorded observation log. Nix resolves
program/contract **names** to hashes/paths via `cdz rewrite` (an AST-validated structural transform, not text
substitution), injects the content-addressed deps, encodes to binary-AST, and feeds it to a **built-once
driver** (the `cdz-platform-itest` analogue). Verdict = exit code. **Adding a run = dropping a `*.ml` file**
(auto-discovered as `checks.<sys>.http-conformance-<name>`, mirroring harness-runs).

### 3.1 The run value

A run is a Cadenza record (every field read by name, order-independent — like `HarnessSpec`):

- `config` — **required**; the SUT setup:
  - `root-router = "<program name>"` — the root router the control server ships to the gateway on connect
    (name → `ProgramHash` via `cdz rewrite`).
  - `programs = [ { name = "…", program = "…" }, … ]` — the Cadenza programs to make resolvable in the CAS
    (routers + handlers), each by name; nix rewrites `program` → the built wasm path and injects its
    content-addressed deps (runtime + nfc), so the run is **self-contained** (mirrors harness-runs `blobs` +
    `deps`). An inline placeholder uses `bytes = b"…"`.
  - `cas-credential = b"…"?` / `control-config = { … }?` — optional overrides; the driver fills defaults
    (harness CAS url + a generated credential).
  - `prime-replies = [ { match = { path = "…"?, program = "…"? }, reply = b"…" }, … ]?` — how the mock
    replies to an incoming `control.send` (correlation-routed).
- `requests` — an ordered list of interactions, each an action plus an optional inline `expect`:
  - `{ http = { method = "GET", path = "/", headers = [ { name = "…", value = "…" } ]?, body = b"…"? },
       expect = { status = 200, body = b"…"?, body-contains = "…"?, header = [ … ]?,
                  retry-until-match = true? } }` — make an HTTP request, assert the response inline.
    `retry-until-match` is the sole non-linear primitive: poll the request until it matches (async live-swap
    propagation).
  - `{ control = { push-root-router = "<program name>" } }` — live-swap the root router (no restart).
  - `{ control = { push-down = { session = b"…"?, payload = b"…" } } }` — push an unsolicited `ControlDown`.
- `checker` — **optional**; the blob name of a Cadenza reducer run over the completed **observation log** to
  judge pass/fail (same shape as platform §9). The log carries the full run: each request + response, and the
  control-plane observations (each captured `ControlUp` with its handler id + request context + correlation,
  config served, root-router pushes, connections, disconnects). The checker `Value.decode`s the full-fidelity
  log and emits a verdict. A run with **no** `checker` passes iff every inline `expect` held and the run
  completed. (Operator: *inline assertions OR an optional checker that looks at what happened and judges.*)
- `run-for` — optional; the time horizon before declaring quiescence (like harness-runs).

### 3.1.1 Examples

Route-to-handler (inline asserts, no checker):

```
{
  config = {
    root-router = "router-hello",
    programs = [
      { name = "router-hello",       program = "router-hello" },
      { name = "handler:http-hello", program = "http-hello" }
    ]
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"hello from a wasm handler" } },
    { http = { method = "GET", path = "/nope" },
      expect = { status = 404 } }                 # no-match -> deny(404) floor
  ]
}
```

Live-swap (control pushes a new router mid-run):

```
{
  config = {
    root-router = "router-hello",
    programs = [ { name = "router-hello", program = "router-hello" },
                 { name = "router-echo",  program = "router-echo" },
                 { name = "handler:http-hello", program = "http-hello" },
                 { name = "handler:http-echo",  program = "http-echo" } ]
  },
  requests = [
    { http = { method = "GET", path = "/" }, expect = { status = 200, body = b"hello from a wasm handler" } },
    { control = { push-root-router = "router-echo" } },
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body-contains = "method=GET", retry-until-match = true } }
  ]
}
```

control.send request/response, judged by a checker program:

```
{
  config = {
    root-router = "router-emitter",
    programs = [ { name = "router-emitter",   program = "router-emitter" },
                 { name = "handler:emitter",  program = "emitter" },
                 { name = "control-send-check", program = "control-send-check" } ],
    prime-replies = [ { match = { path = "/emit" }, reply = b"PONG" } ]   # correlation-routed
  },
  requests = [
    { http = { method = "POST", path = "/emit", body = b"ping" },
      expect = { status = 200, body-contains = "PONG" } }                 # primed reply folded back
  ],
  checker = "control-send-check"   # decodes the log; asserts a ControlUp{program=handler:emitter,
                                    # method=POST, path=/emit, payload=b"ping"} was captured
}
```

The author never builds Cadenza by hand, computes a hash, encodes a frame, or manages sockets/processes —
nix + the driver do all of it. Adding a run = dropping a `*.ml`.

### 3.2 The Rust driver (`cdz-http-conformance`)

A built-once binary (the `cdz-platform-itest` shape) that consumes ONE run's binary-AST spec and:

1. starts CAS, control, gateway as child processes on ephemeral loopback ports (seeding the spec's programs +
   injected deps into the CAS; control told the CAS url + cred; gateway told only the control address);
2. waits for readiness (§6.4), then applies `config` (prime `control.set_root_router`, replies);
3. executes `requests` in order — issuing control commands to the mock (§3.3) and HTTP/WS requests to the
   gateway, recording an **observation log** and checking each inline `expect`;
4. if a `checker` is named, runs it as an ordinary wasm reducer over the completed log for the verdict;
5. exits 0 iff every inline `expect` held and the checker (if any) passed; on failure, a diagnostic (which
   request, expected vs actual, plus each process's captured stderr).

Nix wraps each `*.ml` as a `runCommand` (like `mkHarnessRun`) → `checks.<sys>.http-conformance-<name>`;
fine-grained caching (a run reruns only when its spec, a program it names, or the shared binary changes). The
same binary also runs on-demand against a locally-built spec for author iteration.

### 3.3 How the driver commands the mock control server (internal admin channel)

Because both the driver and the mock are Rust, this is a driver-internal detail, not an author-facing API.
The mock exposes a small admin surface the driver uses to inject state + read observations. **Its payloads are
binary-AST Cadenza values — NOT JSON** (operator directive: *"i want cadenza ast everywhere"* / the standing
"binary-AST is THE data-exchange format, no exceptions" rule). One request/response endpoint over a loopback
socket carries an `AdminCommand` value up and an `AdminReply` value back, each encoded with the same
value-form codec as the control-plane frames (§4). Both the mock and the driver share the frame types + codec
(a library module of the mock crate, which the driver deps), so no ad-hoc text format enters the harness.

The `AdminCommand` sum (driver → mock):

| Command                    | Effect |
|----------------------------|--------|
| set config / root-router   | the `ControlConfig` the mock ships on connect; a `root_router` name is resolved via the mock's program manifest. |
| push root-router (live-swap) | push a new root-router hash to connected gateway sessions (the #8619 cell, now holding a hash). |
| prime `control.send` reply | how the mock replies to an incoming `ControlUp` (canned bytes / drop); the reply echoes the `correlation` so it routes back to the exact handler invocation. |
| push `ControlDown`         | an unsolicited control→handler message (delivered as `on_notification`). |
| reset                      | clear injected config + captured observations (per-scenario isolation). |

The `AdminReply` values (mock → driver), each a binary-AST value:

| Observation             | Carries |
|-------------------------|---------|
| captured `ControlUp`s   | the `ControlUp` list (each: `program` handler id, `session`, `correlation`, `payload`, `request` {method, path, headers}). |
| connections             | gateway sessions (`session`, `config_served`). |
| event log               | ordered connect / config-served / root-router-pushed / control-up / control-down / disconnect. |

**The gateway under test is the STOCK gateway binary — nothing internal is mocked** (operator: *"i want a
stock gateway to be tested end-to-end instead of mocking anything internal. that way i definitely know things
are working as intended"*). Only the *control server* is a mock, and even it is a real **separate process**
speaking the real control-plane wire protocol (§4) — "mock" solely in that its behavior is test-driven via
this admin channel, not that it is fake or embedded. The driver never reaches inside the gateway; every
assertion is on genuine end-to-end behavior over real sockets.

---

## 4. The control-plane contract (mock ⇄ gateway)

The wire interface the mock speaks toward the gateway, and that a rewritten gateway must implement. It is the
existing binary-AST protocol (frames in `cdz-http-gateway/src/codec.rs` today; on where they should live, §7):

- **Transport:** one persistent bidirectional WebSocket, gateway dials control (so the gateway boots with only
  an address).
- **On connect, control → gateway:** a **`ControlConfig`** — `{cas_url: String, cas_credential: Bytes,
  root_router: ProgramHash(33 bytes)}`. The gateway builds a CAS client at `cas_url`, resolves + drives the
  `root_router`.
- **Control → gateway push (live-swap):** a new `root_router` hash at any time; applied to subsequent requests
  with no restart.
- **Gateway → control (up):** a **`ControlUp`** carrying a handler's `control.send` message plus **enough
  context for control to route + respond** (operator: *control must know which handler id is making the
  request and have as much info about the request as possible*):
  - `program: ProgramHash` — **which handler** is sending (the handler id / provenance);
  - `session` — the connection/session the handler is serving;
  - `correlation` — a token unique to *this* `control.send`, echoed on the response so it reaches the exact
    awaiting handler call (the request/response leg);
  - `payload` — the handler's message bytes (opaque to the gateway; meaningful to control);
  - **request context** — as much of the originating HTTP request as the gateway can attach (≥ method + path;
    ideally selected headers) so control can route without re-parsing the payload.

  The gateway inspects none of the *payload*, but **does** stamp the routing context above.
- **Control → gateway (down, addressed):** a **`ControlDown`** `{session, correlation, payload}`. A matching
  `correlation` is the **response** to a pending `ControlUp` (folded back into the awaiting handler call); an
  un-correlated `ControlDown` is an unsolicited push delivered as an `on_notification`.
- All frames are **binary-AST** encoded (the standing data-exchange directive). The mock owns encode/decode;
  the scenario author never sees them.

Because the harness only depends on *these frames*, a gateway rewrite is free to change everything else.

---

## 5. The CAS contract (gateway → CAS) & program resolution

`cdz-cas-http` (unchanged, blessed):

- `GET /{hash}` → blob bytes (`200` content-verified / `404` / `401`), immutable/cacheable. `HEAD /{hash}` →
  existence. `PUT /{hash}` → publish (needs `CDZ_CAS_WRITE_CREDENTIAL`; validates `Hash::of(body)==hash`).
- Auth `Authorization: Bearer <credential>`. Key = base62 `Hash`; resolution is tag-agnostic on the 32-byte
  digest, so a `Program`-tagged fetch resolves a `Blob`-stored body.

**How programs get into CAS (painless, §6).** Nix compiles every program in the harness `programs/` tree and
produces a **pre-seeded CAS store directory** with every compiled component blob **plus the runtime + NFC
components** the guests import (a guest imports `cadenza:runtime/heap@…` by hash; the host composes it from
CAS). The CAS server starts `CDZ_CAS_STORE_DIR=<seeded dir>` (copied to a temp dir per driver run so
scenarios can't mutate the nix store). Result: **every program a scenario names is already resolvable** — no
PUT dance, no per-scenario seeding. (A scenario testing a *missing* program just names one not in the store.)

---

## 6. Automatic Cadenza program injection (the "painless" build)

Hard requirement: adding a Cadenza program is **just dropping a `.cdz`** — the build compiles it, deploys it,
and makes it resolvable by name. Modeled on the platform conformance suite's name-resolution
(`mkHarnessAst` resolves program/contract *names* to store paths/hashes via `cdz rewrite`; guests are
auto-enumerated).

### 6.1 The `programs/` tree + auto-enumeration

```
implementation/seed/crates/cdz-http-gateway/harness/programs/
    handlers/
        http-hello/reducer.cdz       # a handler: folds a request -> http-response
        http-echo/reducer.cdz
        emitter/reducer.cdz          # emits a control.send
    routers/
        router-hello.cdz             # a root router; table references handlers BY NAME
        router-echo.cdz
```

Nix **auto-enumerates** this tree (a `builtins.readDir` walk, like the platform guest enumeration) and calls
`mkCadenzaGuest` per program — no per-program flake edits; dropping a directory is enough. Each yields a
`.wasm` component; a `ProgramHash` is computed with the existing `cdz-http-programhash` tool.

### 6.2 Deploy-templating: bake handler hashes into routers automatically

A root router bakes its handlers' **real** `ProgramHash`es into its source (the operator "ship a compiled
program, don't template at runtime" decision, #8649). The harness automates the baking so authors never touch
hashes:

- A router `.cdz` references handlers by a **name marker**, e.g. `@@handler:http-hello@@`, instead of a raw
  `b"\x..."` literal.
- A nix deploy step per router: compile referenced handlers → get their hashes via `cdz-http-programhash
  --escaped` → substitute each marker with the escaped 33-byte literal → `cdz compile` the router →
  `cdz-http-programhash` the result. (This is `cdz rewrite`-style name→hash resolution; reuse `cdz rewrite`
  directly if it can target these markers, else a scoped `sed`.)
- Handler-before-router order is a natural nix derivation dependency.

### 6.3 The rewritten, self-contained spec (what nix hands the driver)

There is **no separate manifest file** — the rewritten `*.ml` spec is self-contained (binary-AST), exactly
like harness-runs. `mkHarnessAst` (reused/adapted) rewrites each `config.programs[*].program` name → its built
wasm `path`, rewrites `root-router`/`push-root-router`/`checker` names → the matching program, and injects a
`deps` list of the content-addressed runtime + nfc components the guests import. The driver seeds every
program blob + dep into the CAS store it boots (by content hash), so a guest's content-addressed imports
resolve and the run is self-contained (mirrors the itest executable seeding `deps`). The three SUT **binary**
paths are supplied to the driver by the `runCommand` env (built once, shared across runs). **Non-nix
fallback:** the driver also accepts locally-built wasm paths for author iteration without a full nix build.

### 6.4 Readiness

Each binary exposes a cheap readiness signal: CAS `HEAD /{any}` (or `/healthz`), control admin health, gateway
`/healthz` (or "first successful request"). The gateway is ready only once it has dialed control and applied a
`ControlConfig`; the driver blocks on that so a scenario never races an unconfigured gateway.

---

## 7. Where the new code lives

- **Conformance corpus** — a repo-versioned directory of `*.ml` run specs, e.g.
  `implementation/seed/crates/cdz-http-gateway/harness/runs/**.ml` (mirroring `cdz-platform/harness-runs/`).
  Grows freely; adding a run is dropping a `*.ml` file (auto-discovered → `checks.<sys>.http-conformance-<name>`).
  **Checker programs** (Cadenza reducers that judge the observation log) live in the `programs/` tree (§6.1)
  and compile like any other guest.
- **The Rust driver** — `cdz-http-conformance`, a new bin (proposed: its own excluded `[workspace]` crate, or
  a bin in the mock-control crate). Consumes ONE run's binary-AST spec, spins up the SUT, executes + asserts +
  runs the checker (the `cdz-platform-itest` analogue).
- **Mock control server** — `cdz-http-control-mock`, a new bin. **Recommendation: a new excluded crate**, so
  it survives the gateway rewrite independently. (Driver + mock may share one crate with two bins.)
- **The control-plane wire codec.** The mock and the gateway must agree on
  `ControlConfig`/`ControlUp`/`ControlDown`. Today they live in `cdz-http-gateway/src/codec.rs`. Since the
  gateway will be rewritten, **recommendation: extract the wire frames into a small shared crate
  `cdz-http-protocol`** both the mock and the rewritten gateway depend on — one source of truth for the
  interface the harness pins (§10).
- **Harness nix plumbing** (auto-enumeration, deploy-templating, seeded store, manifest, the driver build)
  goes in `flake.nix` as `packages.<sys>.http-outpost-harness`, optionally exposed as a check later.

---

## 8. What the corpus covers (initial scenarios → grows)

Each a declarative scenario, mapping behaviors currently pinned by Rust `#[test]`s:

1. **route-to-handler** — `GET /` → hello handler → 200; unmatched → 404 deny.
2. **live-swap** — push a new root-router hash; same running gateway serves new behavior, no restart.
3. **dispatch + fold** — router emits a dispatch effect, handler folds, response returned (dumb path).
4. **deny terminal** — router closes with `deny(status,reason)` → that status floor.
5. **413** — body over the ceiling → 413 before routing.
6. **504** — a handler that never resolves → wall-clock floor.
7. **fresh session** — per-request isolation (a counting handler answers "1" every time).
8. **control.send request/response** — handler emits `control.send`; assert the `ControlUp` carried the right
   handler id, request context (method/path), correlation + payload; a primed correlation-matched
   `ControlDown` reply folds back and shapes the HTTP response; an unsolicited `ControlDown` arrives as an
   `on_notification`.
9. **CAS resolution** — a named-but-absent program → gateway floors gracefully (no hang/crash).
10. **content-addressed routing** — router table carries a real hash; handler resolves from CAS by hash.

Then grow: WebSocket sessions, malformed frames, auth failures, reconnect/resilience, concurrency.

---

## 9. Build sequence (landable slices)

1. **This doc + PR** (interface). ← current step.
2. **Mock control server (Rust)** — `cdz-http-control-mock`: control-plane ws face (ship `ControlConfig`,
   push root-router, receive `ControlUp`, send correlation-matched `ControlDown`) + admin channel (§3.3) +
   program-manifest name resolution + a nix build. Gated by its own crate check + a few Rust tests of the
   mock's *own* correctness (admin/wire translation), not gateway behavior.
3. **Auto-enumerated program tree + deploy-templating + `mkHarnessAst`-style name-rewrite + injected deps**
   (§6) — the nix plumbing that turns a `*.ml` run into a self-contained binary-AST spec. Verifiable
   standalone: rewrite a spec, assert names resolved + deps injected.
4. **The Rust driver `cdz-http-conformance`** — consumes a run's binary-AST spec: process orchestration +
   CAS-seed + step executor + inline-`expect` asserts + checker execution over the observation log + report.
   First runs (§8.1–2) land with it, wrapped as `checks.<sys>.http-conformance-<name>` (mkHarnessRun-style).
5. **Gateway boot-from-control** (the one gateway-side gap the harness needs; §6). Coordinate with the
   gateway-rewrite owner — the harness defines the contract (§4/§5/§6.4); the gateway implements it. If no
   active gateway owner, build the minimal boot wiring against the existing `HttpEdge::dumb` + `RootDriver` as
   a stopgap so the corpus runs end-to-end, flagged for replacement.
6. **Grow the corpus + migrate** the behaviors currently asserted by gateway `#[test]`s into scenarios, then
   **retire** the superseded `#[test]`s (don't keep both).

## 10. Decisions (settled by operator) + remaining open items

**Settled by the operator (2026-09-10):**
- **Run-spec format = the ML surface** (`*.ml` Cadenza values, like `cdz-platform/harness-runs/`), rewritten
  by `cdz rewrite` + encoded to binary-AST — NOT TOML/Python. Config + requests + inline assertions + an
  optional checker program (§3).
- **Mock control server = a separate process; the gateway under test = the STOCK binary, nothing internal
  mocked** — full end-to-end (§3.3, §2).

**Remaining open (non-blocking; proceeding on the defaults):**
- **Wire codec home** (§7): extract `ControlConfig`/`ControlUp`/`ControlDown` to a shared `cdz-http-protocol`
  crate (default) vs the mock depending on the gateway crate's `codec`. Coordinate with the gateway-rewrite
  owner when one exists.
- **Router deploy-templating** (§6.2): reuse `cdz rewrite` if it can target source markers, else scoped `sed`.

Neither blocks starting the mock control server (slice 2); I'll raise anything genuinely load-bearing to the
concierge as an `ask` and keep building on the defaults.
