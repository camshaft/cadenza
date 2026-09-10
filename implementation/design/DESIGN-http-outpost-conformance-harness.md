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

**Goal.** A growing **conformance corpus** of **declarative scenario files**, run by a **Rust driver**
against a **real, running HTTP gateway** wired to a real CAS store and a **mock control server**. Each
scenario is a linear sequence of steps:

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
   │  reads a scenario corpus + a nix build manifest; spins up 3 child processes; executes each          │
   │  scenario's steps against them; verdict = per-scenario pass/fail (report + exit code).              │
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

## 3. The test interface: declarative scenarios

### 3.1 What a test author writes

A scenario is a small declarative file (one file per scenario; the corpus is a directory of them). Format is
a human-authored, low-ceremony document — a linear list of `steps`, each either an **action** or an
**expectation**. Illustrative (final surface syntax TBD — TOML shown; a `.sexp`/cadenza-value form is an
option, §10):

```toml
# corpus/routing/route-to-handler.toml
name = "routes GET / to the hello handler, unmatched -> 404"

[[step]]  # arrange: point the gateway at a root router (by NAME; driver resolves to a ProgramHash)
control.set_root_router = "router-hello"

[[step]]  # act + assert
http.get = "/"
expect.status = 200
expect.body = "hello from a wasm handler"

[[step]]
http.get = "/nope"
expect.status = 404          # no-match -> deny(404) floor
```

```toml
# corpus/reconfig/live-swap.toml
name = "control live-swaps the root router without a restart"

[[step]]
control.set_root_router = "router-hello"
[[step]]
http.get = "/"
expect.body = "hello from a wasm handler"

[[step]]
control.push_root_router = "router-echo"     # live-swap
[[step]]
http.get = "/"
expect.body_contains = "method=GET"
expect.retry_until_match = true              # the ONE non-linear primitive: poll until the swap propagates
```

```toml
# corpus/control-plane/control-send.toml
name = "handler control.send reaches control with handler id + request context, reply folds back"

[[step]]
control.set_root_router = "router-emits-control-send"
control.prime_reply = { match = { path = "/emit" }, reply = "PONG" }   # correlation-routed reply

[[step]]
http.post = "/emit"
body = "ping"
expect.status = 200
expect.body_contains = "PONG"                # the primed reply folded back into the handler's response

[[step]]  # observe what control captured
expect.control_up = [
  { program = "handler:emitter", method = "POST", path = "/emit", payload = "ping" },
]
```

The author does **not**: build any Cadenza by hand, compute any hash, encode any binary-AST frame, or manage
sockets/processes. All of that is the driver + the nix build. Adding a scenario = dropping a file in the
corpus directory (the driver auto-discovers them).

### 3.2 The Rust driver (`cdz-http-conformance`)

A built-once binary (the `platformItest` shape). Given a corpus path + a build manifest (§6.3), for each
scenario it:

1. starts CAS, control, gateway as child processes on ephemeral loopback ports (control told the CAS url +
   cred; gateway told only the control address);
2. waits for readiness (§6.4);
3. resets control state (per-scenario isolation), then executes the scenario's steps in order — issuing
   control commands to the mock (§3.3), HTTP/WS requests to the gateway, and checking each `expect`;
4. records a verdict; on the first failed assertion the scenario fails with a diagnostic (which step, expected
   vs actual, plus each process's captured stderr).

Run modes: on-demand (`cdz-http-conformance <corpus-dir>` against a nix-provided SUT) and wrappable as a nix
`runCommand` check later (`checks.<sys>.cdz-http-gateway-conformance`), exactly like `mkHarnessRun`. Verdict
is per-scenario (a report), aggregated to an exit code for the check.

The step vocabulary (grows with the corpus; initial set):
- `control.set_root_router <name|hash>` · `control.push_root_router <name|hash>` (live-swap) ·
  `control.config { cas_url?, cas_credential?, root_router }` · `control.prime_reply { match, reply }` ·
  `control.push_down { session?, payload }` · `control.disconnect { session? }`
- `http.<method> <path>` (+ `headers`, `body`) capturing the response ·
  `expect.{status, header, body, body_contains, retry_until_match}`
- `expect.control_up [ { program, session?, method?, path?, payload? } ]` ·
  `expect.connections`, `expect.events` (ordering)
- (later) `ws.connect`, `ws.send`, `expect.ws_recv`

### 3.3 How the driver commands the mock control server (internal admin channel)

Because both the driver and the mock are Rust, this is a driver-internal detail, not an author-facing API.
The mock exposes a small admin surface the driver uses to inject state + read observations; a plain
HTTP/JSON admin listener is the default (trivial to call, human-debuggable with `curl`, and keeps the mock
independently pokeable):

| Inject (driver → mock)        | Effect |
|-------------------------------|--------|
| set config / root-router      | the `ControlConfig` the mock ships on connect; a `root_router` name is resolved via the mock's `--program-manifest`. |
| push root-router (live-swap)  | push a new root-router hash to connected gateway sessions (the #8619 cell, now holding a hash). |
| prime `control.send` reply    | how the mock replies to an incoming `ControlUp` (echo / canned / drop); the reply echoes the `correlation` so it routes back to the exact handler invocation. |
| push `ControlDown`            | an unsolicited control→handler message (delivered as `on_notification`). |
| reset                         | clear injected config + captured observations (per-scenario isolation). |

| Observe (mock → driver)       | Returns |
|-------------------------------|---------|
| captured `ControlUp`s         | each `{program (handler id), session, correlation, payload, request:{method,path,headers}, seq, ts}`. |
| connections                   | gateway sessions `{session, connected_at, config_served}`. |
| event log                     | ordered connect / config-served / root-router-pushed / control-up / control-down / disconnect. |

(If we later decide to **embed** the control endpoint in the driver process to drop a child process, this
admin surface becomes an in-process API instead; §10.)

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

### 6.3 The build manifest + seeded store (what nix hands the driver)

```json
{
  "binaries": { "cas": "…/bin/cdz-cas-http", "control": "…/bin/cdz-http-control-mock",
                "gateway": "…/bin/cdz-http-gateway" },
  "cas_store": "…/cas-seeded",                     // pre-seeded: all programs + runtime + nfc
  "programs":  { "router-hello": "gWc…base62…",     // name -> ProgramHash
                 "handler:http-hello": "hZ2…" }
}
```

The driver discovers it via an env var / flag (`CDZ_OUTPOST_HARNESS=<manifest.json>`) that a devshell / `nix
build` sets. **Non-nix fallback:** the driver also works pointed at a locally-built manifest, so an author can
iterate without a full nix build.

### 6.4 Readiness

Each binary exposes a cheap readiness signal: CAS `HEAD /{any}` (or `/healthz`), control admin health, gateway
`/healthz` (or "first successful request"). The gateway is ready only once it has dialed control and applied a
`ControlConfig`; the driver blocks on that so a scenario never races an unconfigured gateway.

---

## 7. Where the new code lives

- **Conformance corpus** — a repo-versioned directory of declarative scenario files, e.g.
  `implementation/seed/crates/cdz-http-gateway/harness/corpus/**.toml`. Grows freely; adding a scenario is
  dropping a file. Not compiled by nix (it's data); the driver reads it.
- **The Rust driver** — `cdz-http-conformance`, a new bin (proposed: its own excluded `[workspace]` crate, or
  a bin in the mock-control crate). Reads a scenario, spins up the SUT, executes + asserts.
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
3. **Auto-enumerated program tree + deploy-templating + seeded CAS store + manifest** (§6) — the nix
   `http-outpost-harness` package. Verifiable standalone: build it, assert the manifest resolves + the store
   serves.
4. **The Rust driver `cdz-http-conformance`** — scenario parser + process orchestration + step executor +
   assertions + report. First scenarios (§8.1–2) land with it.
5. **Gateway boot-from-control** (the one gateway-side gap the harness needs; §6). Coordinate with the
   gateway-rewrite owner — the harness defines the contract (§4/§5/§6.4); the gateway implements it. If no
   active gateway owner, build the minimal boot wiring against the existing `HttpEdge::dumb` + `RootDriver` as
   a stopgap so the corpus runs end-to-end, flagged for replacement.
6. **Grow the corpus + migrate** the behaviors currently asserted by gateway `#[test]`s into scenarios, then
   **retire** the superseded `#[test]`s (don't keep both).

## 10. Open decisions / asks (non-blocking; proceeding on the defaults)

- **Scenario file format** (§3.1): a human-authored declarative doc. **Default: TOML** (trivial to author +
  parse, diff-friendly). A `.sexp`/cadenza-value form (→ binary-AST, closest to the platform-itest precedent)
  is the alternative if we want the *scenario* itself in the canonical exchange format; I lean TOML for author
  ergonomics since the Cadenza *programs* are already the binary-AST/wasm artifacts.
- **Mock control server: separate process vs embedded in the driver** (§3.3): **default separate** (matches
  "3 servers", most faithful — the gateway dials a real socket). Embedding drops a child process at the cost
  of realism; easy to switch since both are Rust.
- **Wire codec home** (§7): extract `ControlConfig`/`ControlUp`/`ControlDown` to a shared `cdz-http-protocol`
  crate (default) vs the mock depending on the gateway crate's `codec`. Coordinate with the gateway-rewrite
  owner when one exists.
- **Router deploy-templating** (§6.2): reuse `cdz rewrite` if it can target source markers, else scoped `sed`.

None of these block starting the mock control server (slice 2); I'll raise anything genuinely load-bearing to
the concierge as an `ask` and keep building on the defaults.
