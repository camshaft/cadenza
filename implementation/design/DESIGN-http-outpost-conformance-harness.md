# DESIGN — http-outpost gateway conformance harness

**Status:** proposed (v-gateway-conformance, 2026-09-10). Supersedes the test-*driver* model of
`DESIGN-http-outpost-integration-harness.md` (#8651) per a direct operator directive this session (verbatim
below). The earlier doc's black-box goal stands; its "thin Rust orchestrator inside a nix `runCommand`"
mechanism does **not** — the operator wants the tests in **Python, outside the flake**, spinning up the
processes themselves.

## Operator directive (verbatim, 2026-09-10)

> "as your first step can you write a design doc and open a PR outlining your plans. i'd like to see what you
> are thinking in terms of test interface and how it will tie into the other components. don't over index on
> the current implementation of the cdz-http-gateway. that will likely be rewritten from scratch. just focus
> on the interface. we'll also need you to build a mock control server in rust that the python script can
> inject messages to and change behavior for the tests. i would expect the test framework to spin up these
> processes and then the python script can drive them and make observations about all of their behavior and
> either pass fail. we'll also need to spin up the CAS http server so programs can resolve. it would be great
> if we could also automatically inject cadenza programs into the environment, similar to how we do for the
> platform conformance test suite. i don't want any hand-rolled cadenza builds. it should be as painless as
> possible to write one and have it properly compiled for the harness."

And the founding brief (concierge, same session): tests are **Python, outside the flake**; nix's job is to
**build + provide** the running gateway/control/CAS environment + the compiled guests, NOT to contain the
corpus; behavior assertions are **language-neutral black-box HTTP + control-plane outcomes**.

---

## 1. Goal & non-goals

**Goal.** A growing **conformance corpus** of black-box tests, written in **Python and living outside the
flake**, run against a **real, running HTTP gateway** wired to a real CAS store and a **mock control
server**. Each test:

1. **arranges** — declares the Cadenza programs it needs (by name), and injects control-plane state (points
   the gateway at a root-router program, live-swaps it, seeds config, primes `control.send` responses) via a
   **language-neutral admin API** on the mock control server;
2. **acts** — makes real HTTP (and WebSocket) requests at the gateway over a loopback socket;
3. **asserts** — on observable outcomes: the HTTP status/headers/body, that a live-swap took effect, that a
   `deny`/413/504 floor fired, that a `control.send` envelope was delivered to control with the right
   provenance, etc.

**The harness tests a CONTRACT, not an implementation.** The gateway is expected to be rewritten from
scratch. This doc therefore pins the *interfaces* the harness depends on (§4 the control-plane contract, §5
the CAS contract, §6 the process/boot contract) and treats today's `cdz-http-gateway` internals
(`RootDriver`, `GatewayResolver`, `DynamicRouter`, the module-level `#[test]`s) as **replaceable**. Any
gateway that honors the three contracts passes the same corpus.

**Non-goals.**
- Not a nix `runCommand` that boots the whole SUT and returns an exit code (the #8651 model). Nix builds the
  binaries + compiles the guests; **Python** owns process lifecycle and assertions.
- Not Rust `#[test]`s of gateway internals. The existing pile (`src/*.rs` `#[cfg(test)]` blocks) is migrated
  into corpus scenarios and then retired (§9).
- Not a production control server. The control server here is a **mock**: it speaks the real gateway-facing
  protocol, but its behavior is *driven by the test* through the admin API, not by real control logic.

---

## 2. Topology

```
   ┌─────────────────────────── Python test process (pytest, OUTSIDE the flake) ───────────────────────────┐
   │                                                                                                        │
   │   fixtures spin up 3 child processes, then drive + observe them:                                       │
   │                                                                                                        │
   │        (a) admin HTTP/JSON  ┌───────────────────┐        control-plane ws        ┌─────────────────┐   │
   │        inject + observe ───▶│ mock control server│◀──────────────────────────────│  HTTP gateway   │   │
   │                            └───────────────────┘   ControlConfig / root-router    │ (boot-from-      │   │
   │                              (cdz-http-control-mock)  push down; ControlUp up      │  control)        │   │
   │                                                                                    └────────┬────────┘   │
   │        (b) real HTTP/WS requests ──────────────────────────────────────────────────────────▶│ :gw_port  │
   │                                                                                              │           │
   │                                                            resolve program by hash  ┌────────▼────────┐  │
   │                                                                       GET /{hash} ─▶│  CAS HTTP server │  │
   │                                                                                     │  (cdz-cas-http)  │  │
   │                                                                                     └─────────────────┘  │
   │                                                                                                          │
   │   Cadenza programs (root routers, handlers) are compiled by NIX ahead of time and are already resolvable│
   │   in the CAS store (seeded store dir). Python refers to them by NAME; a build-produced manifest maps     │
   │   name → ProgramHash.                                                                                    │
   └──────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

Three SUT processes, all bound to loopback on test-chosen (ephemeral) ports:

- **CAS HTTP server** — `cdz-cas-http` (EXISTS, blessed). Serves compiled programs by hash so the gateway can
  resolve them. Started disk-backed at a nix-produced, pre-seeded store dir (§5).
- **Mock control server** — `cdz-http-control-mock` (**NEW, the main Rust artifact I build**). Two faces: a
  **control-plane** face toward the gateway (speaks the real wire protocol, §4) and an **admin** face toward
  Python (HTTP/JSON, §3). Holds test-driven state.
- **HTTP gateway** — `cdz-http-gateway` in **boot-from-control** mode: told only the control address, it
  dials control, receives its `ControlConfig` (CAS url + credential + root-router program hash), resolves
  programs from CAS by hash, and serves. (This boot mode is an unbuilt gap today — see §6 / §9. The harness
  defines the *contract* it must satisfy; the gateway rewrite implements it.)

### 2.1 What the gateway *essentially is* (operator framing)

The gateway is a **dynamic HTTP server that the control server reconfigures on the fly.** Its whole external
surface is **just HTTP requests** — clients see nothing else. Its **control surface** is equally simple:
control **pushes router changes** (a new root-router program, resolved from CAS by hash), and the gateway
**hot-reconfigures with no restart**, routing every subsequent request through the new handlers. Two
consequences the harness leans on:

- **Reconfiguration is live and observable purely through HTTP.** A test pushes a router change via control,
  then observes the change by making an HTTP request — the gateway needs no restart and exposes no other
  configuration surface.
- **The control link is bidirectional.** Handlers running in the gateway can **send messages up to the
  control server**, which handles them and **responds**; the response is routed back to the exact handler
  invocation that sent it (§4). This is a request/response leg, not fire-and-forget.

The **control server is the sole configurator** of the gateway: the gateway boots knowing only where control
is; everything else (which program routes, the CAS location, credentials) arrives over the control link and
can be live-swapped.

---

## 3. The test interface (Python)

### 3.1 What a test author writes

A test is a Python function using a small fixture library. The intended ergonomics:

```python
def test_routes_to_the_matched_handler(outpost):
    # `outpost` fixture already booted CAS + control + gateway and pointed the
    # gateway at a root router that routes "/" -> the http-hello handler.
    outpost.control.set_root_router("router-hello")          # by NAME; resolved to a ProgramHash

    resp = outpost.http.get("/")
    assert resp.status == 200
    assert resp.text == "hello from a wasm handler"

    resp = outpost.http.get("/nope")
    assert resp.status == 404                                 # no-match -> deny(404) floor


def test_live_swap_reroutes_without_restart(outpost):
    outpost.control.set_root_router("router-hello")
    assert outpost.http.get("/").text == "hello from a wasm handler"

    outpost.control.set_root_router("router-echo")            # push a new root-router hash (live-swap)
    outpost.wait_until(lambda: outpost.http.get("/").status == 200)
    assert "method=GET" in outpost.http.get("/").text         # same running gateway, new behavior


def test_control_send_is_delivered_with_provenance(outpost):
    outpost.control.set_root_router("router-emits-control-send")
    outpost.http.post("/emit", body=b"ping")

    ups = outpost.control.received_control_up()               # observe what control captured
    assert len(ups) == 1
    assert ups[0].program == outpost.hash("handler-emitter")  # provenance = emitting program
    assert ups[0].payload == b"ping"
```

The corpus author does **not**: build any Cadenza by hand, compute any hash, encode any binary-AST frame, or
manage sockets/processes. All of that is the fixture library + the nix build.

### 3.2 The fixture object (`outpost`)

Provided by a session/function-scoped pytest fixture (`conftest.py`) that:

1. reads a **build manifest** (nix output; §6.3) locating the three binaries + the seeded CAS store dir + the
   `name → ProgramHash` program map;
2. starts CAS, control, gateway as child processes on ephemeral loopback ports (control told the CAS url +
   cred; gateway told only the control address);
3. waits for readiness (health probes, §6.2);
4. yields an `outpost` handle;
5. on teardown, tears down control state (`reset`) and kills the processes; dumps each process's captured
   stdout/stderr into the test report on failure.

`outpost` surfaces three sub-APIs:

- `outpost.http` — a plain HTTP/WS client bound to the gateway port (`get`/`post`/`request`/`ws_connect`).
- `outpost.control` — a client for the mock control server's **admin API** (§3.3).
- `outpost.hash(name)` / `outpost.wait_until(pred, timeout)` — helpers (name→hash lookup, polling).

Per-test isolation: each test (or a fresh `outpost`) begins with the control server reset to empty state, so
tests never leak routing/config into each other.

### 3.3 The mock control server admin API (the injection + observation surface)

This is the **language-neutral interface** the operator asked for — a plain HTTP/JSON admin listener on the
mock control server that Python pokes to *inject messages and change behavior*, and to *observe* what the
gateway did on the control plane. (JSON here is the ADMIN transport between Python and the mock; the
gateway-facing control plane stays binary-AST, §4. The mock translates.)

Injection (arrange / act):

| Method & path              | Body (JSON)                                  | Effect |
|----------------------------|----------------------------------------------|--------|
| `PUT /config`              | `{cas_url?, cas_credential?, root_router}`   | Set the `ControlConfig` the mock ships to a gateway on connect. `root_router` is a program **name or hex/base62 hash**; `cas_url`/`cas_credential` default to the harness CAS (fixture-filled). |
| `POST /root-router`        | `{program}`                                  | **Live-swap**: push a new root-router hash to every connected gateway session (the #8619 cell, now holding a hash). |
| `POST /control-down`       | `{session, payload_b64}` or `{payload_b64}`  | Push a `ControlDown` to a session (or broadcast) — a control→handler message, delivered as an `on_notification`. |
| `PUT /control-send-reply`  | `{match?, reply_b64}`                        | Prime how the mock replies to an incoming `control.send` (`ControlUp`) — echo, canned bytes, or drop. `match?` can select on handler `program`, request method/path, or payload prefix. The mock echoes the `correlation` so the reply routes back to the exact handler invocation — this is how a test exercises the request/response leg. |
| `POST /disconnect`         | `{session?}`                                 | Force-close a control link (test reconnect/resilience). |
| `POST /reset`              | —                                            | Clear all injected config + captured observations (per-test isolation). |

Observation (assert):

| Method & path              | Returns |
|----------------------------|---------|
| `GET /control-up`          | The list of `ControlUp` envelopes the mock received, each `{program (hash), session, correlation, payload_b64, request: {method, path, headers}, seq, ts}` — assert `control.send` delivery, handler-id provenance, and the request context the gateway attached. |
| `GET /connections`         | Current gateway sessions `{session, connected_at, config_served}` — assert the gateway dialed + what config it got. |
| `GET /events`              | An ordered event log (connect / config-served / root-router-pushed / control-up-received / control-down-sent / disconnect) for ordering assertions. |
| `GET /health`              | Readiness for the boot fixture. |

Program **names** are accepted wherever a hash is expected; the mock resolves them via the build manifest it
is started with (`--program-manifest <path>`), so tests read `set_root_router("router-hello")` rather than a
45-char base62 string. A raw hash is also accepted for tests that specifically exercise unknown/garbage
hashes.

---

## 4. The control-plane contract (mock ⇄ gateway)

This is the wire interface the mock speaks toward the gateway and that a rewritten gateway must implement.
It is the existing binary-AST protocol, restated here as the pinned contract (frames defined today in
`cdz-http-gateway/src/codec.rs`; see §7 on where they should live):

- **Transport:** one persistent bidirectional WebSocket, gateway dials control. (Direction chosen so the
  gateway boots with only an address.)
- **On connect, control → gateway:** a **`ControlConfig`** frame — `{cas_url: String, cas_credential: Bytes,
  root_router: ProgramHash(33 bytes)}`. The gateway applies it: build a CAS client at `cas_url` with the
  credential, resolve+drive the `root_router` program.
- **Control → gateway push (live-swap):** a new `root_router` hash at any time; the gateway applies it to
  subsequent requests with no restart (the live cell).
- **Gateway → control (up):** a **`ControlUp`** envelope carrying a handler's `control.send` message plus
  **enough context for control to route + respond**. The operator requirement: *control must know which
  handler id is making the request and have as much info about the request as possible.* So the envelope
  carries:
  - `program: ProgramHash` — **which handler** is sending (the handler id / provenance);
  - `session` — the connection/session the handler is serving, so responses and later frames correlate;
  - `correlation` — a token unique to *this* `control.send` invocation, echoed back on the response so it
    reaches the exact awaiting handler call (this is the request/response leg — not fire-and-forget);
  - `payload` — the handler's message bytes (opaque to the gateway; meaningful to control);
  - **request context** — as much of the originating HTTP request as the gateway can attach (at minimum the
    method + path; ideally selected headers) so control can route correctly without re-parsing the payload.

  The gateway inspects none of the *payload* (pure opaque router for the message body), but it **does** stamp
  the routing context above — that context is exactly what lets control decide how to handle + respond.
- **Control → gateway (down, addressed):** a **`ControlDown`** envelope `{session, correlation, payload}`.
  When `correlation` matches a pending `ControlUp`, it is the **response** to that `control.send`, folded
  back into the awaiting handler call; an un-correlated `ControlDown` is an unsolicited push delivered to the
  session as an `on_notification`.
- Everything above is **binary-AST** encoded (the standing "binary-AST is THE data-exchange format"
  directive). The mock owns encode/decode of these frames; Python never sees them (it speaks the JSON admin
  API, §3.3).

Because the harness only depends on *these frames*, a gateway rewrite is free to change everything else.

---

## 5. The CAS contract (gateway → CAS) & program resolution

`cdz-cas-http` (unchanged, blessed):

- `GET /{hash}` → blob bytes (`200` content-verified / `404` / `401`), immutable/cacheable. `HEAD /{hash}` →
  existence. `PUT /{hash}` → publish (requires `CDZ_CAS_WRITE_CREDENTIAL`; validates `Hash::of(body)==hash`).
- Auth: `Authorization: Bearer <credential>`. Key is the base62 `Hash` string; resolution is tag-agnostic on
  the 32-byte digest, so a `Program`-tagged fetch resolves a `Blob`-stored body.

**How programs get into CAS (painless, no hand-rolled builds — §6).** Nix compiles every program in the
harness `programs/` tree and produces a **pre-seeded CAS store directory** containing every compiled
component blob **plus the runtime + NFC components** the guests import (a guest imports
`cadenza:runtime/heap@…` by hash; the host composes it from CAS, so those must be present too). The CAS
server is started `CDZ_CAS_STORE_DIR=<seeded dir>` (copied to a temp dir per session so tests can't mutate
the nix store). Result: **every program a test names is already resolvable** — no PUT dance, no per-test
seeding. (A test that specifically exercises a *missing* program just names one not in the store.)

---

## 6. Automatic Cadenza program injection (the "painless" build)

The operator's hard requirement: adding a Cadenza program to the harness must be **just dropping a `.cdz`
file** — the build compiles it, deploys it, and makes it resolvable by name. Modeled on the platform
conformance suite's name-resolution (`cdz-platform-itest`'s `mkHarnessAst` resolves program/contract *names*
to store paths/hashes via `cdz rewrite`; guests are auto-enumerated).

### 6.1 The `programs/` tree + auto-enumeration

```
implementation/seed/crates/cdz-http-gateway/harness/programs/
    handlers/
        http-hello/reducer.cdz            # a handler: folds a request -> http-response
        http-echo/reducer.cdz
        emitter/reducer.cdz               # emits a control.send
    routers/
        router-hello.cdz                  # a root router; table references handlers BY NAME
        router-echo.cdz
```

Nix **auto-enumerates** this tree (a `builtins.readDir` walk, like the platform guest enumeration) and calls
`mkCadenzaGuest` per program. No per-program flake edits: dropping a directory is enough. Each produces a
`.wasm` component; a `ProgramHash` is computed with the existing `cdz-http-programhash` tool.

### 6.2 Deploy-templating: bake handler hashes into routers automatically

A root router bakes its handlers' **real** `ProgramHash`es into its source (the operator's "ship a compiled
program, don't template at runtime" decision, #8649). Today that baking is manual (`cdz-http-programhash
--escaped` + `sed`). The harness **automates** it so authors never touch hashes:

- A router `.cdz` references handlers by a **name marker** in its baked table, e.g.
  `@@handler:http-hello@@` (a distinctive placeholder) instead of a raw `b"\x..."` literal.
- A nix deploy step, for each router: compile all referenced handlers → get their hashes via
  `cdz-http-programhash --escaped` → substitute each `@@handler:NAME@@` with the escaped 33-byte literal →
  `cdz compile` the router → `cdz-http-programhash` the result. (This is `cdz rewrite`-style name→hash
  resolution; if `cdz rewrite` can be pointed at these markers we reuse it directly rather than `sed`.)
- Dependency order (handlers before routers) is expressed naturally as nix derivation deps.

### 6.3 The build manifest + seeded store (what nix hands Python)

The harness build produces one output consumed by the Python fixtures:

```json
{
  "binaries":  { "cas": "/nix/store/…/bin/cdz-cas-http",
                 "control": "/nix/store/…/bin/cdz-http-control-mock",
                 "gateway": "/nix/store/…/bin/cdz-http-gateway" },
  "cas_store": "/nix/store/…/cas-seeded",                     // pre-seeded, all programs + runtime + nfc
  "programs":  { "router-hello":   "gWc…base62…",             // name -> ProgramHash
                 "handler:http-hello": "hZ2…",
                 "handler:http-echo":  "kP9…" }
}
```

Python discovers it via an env var (`CDZ_OUTPOST_HARNESS=<manifest.json>`) that a devshell / `nix build`
sets, or a documented `nix build .#http-outpost-harness && ./run path`. The corpus never hard-codes store
paths. **Non-nix fallback:** the runner also works if pointed at a locally-built manifest, so an author can
iterate without a full nix build.

### 6.4 Readiness

Each binary must expose a cheap readiness signal for the boot fixture: CAS `HEAD /{any}` (or a `/healthz`),
control `GET /health` (admin API), gateway a `/healthz` (or "first successful request"). The gateway is
ready only once it has dialed control and applied a `ControlConfig`; the fixture blocks on that so a test
never races an unconfigured gateway.

---

## 7. Where the new code lives

- **Python corpus + runner + fixtures — OUTSIDE the flake**, at repo top-level `http-outpost-conformance/`:
  ```
  http-outpost-conformance/
      conftest.py           # the outpost fixture: boot/teardown, manifest discovery
      harness/              # the fixture library: process mgmt, http client, control admin client
      tests/                # the GROWING corpus (test_routing.py, test_live_swap.py, test_control_send.py …)
      README.md             # how to run: nix build the harness, point pytest at the manifest
      pyproject.toml        # pytest + httpx + websockets; no nix
  ```
  It is repo-versioned but **not a flake input** — it is not built or gated by nix; it is run on demand
  against a nix-provided SUT. (Matches "tests live outside the flake"; this directly serves the standing
  "e2e → conformance, not `#[test]`; don't lock tests into a language" directive.)

- **Mock control server — a new Rust bin.** Proposed: a new crate `cdz-http-control-mock` (excluded
  `[workspace]`, like the other http-outpost crates), or a `[[bin]]` in the gateway crate. Recommendation:
  **new crate**, so it survives the gateway rewrite independently.

- **The control-plane wire codec.** The mock and the gateway must agree on the `ControlConfig`/`ControlUp`/
  `ControlDown` frames. Today those live in `cdz-http-gateway/src/codec.rs`. Since the gateway will be
  rewritten and the mock must not depend on doomed internals, **recommendation: extract the wire frames into
  a small shared crate `cdz-http-protocol`** that both the mock and the (rewritten) gateway depend on — one
  source of truth for the interface the harness pins. (Open decision, §10.)

- **The harness nix plumbing** (auto-enumeration, deploy-templating, seeded store, manifest) goes in
  `flake.nix` as a `packages.<sys>.http-outpost-harness` (NOT a check — it is built on demand, not gated).

---

## 8. What the corpus covers (initial scenarios → grows)

Seed scenarios (each a Python test), mapping the behaviors currently pinned by Rust `#[test]`s:

1. **route-to-handler** — root router routes `GET /` → hello handler → 200 body; unmatched → 404 deny.
2. **live-swap** — push a new root-router hash; same running gateway serves new behavior, no restart.
3. **dispatch + fold** — router emits a dispatch effect, handler folds, response returned (the dumb path).
4. **deny terminal** — router closes with `deny(status,reason)` → that status floor.
5. **413** — body over the ceiling → 413 before routing.
6. **504** — a handler that never resolves → wall-clock floor.
7. **fresh session** — per-request isolation (a counting handler answers "1" every time).
8. **control.send request/response** — a handler emits `control.send`; assert the `ControlUp` reached
   control with the right handler id, request context (method/path), correlation, and payload; then a primed
   `ControlDown` reply (same correlation) is folded back into the awaiting handler call and observably shapes
   the HTTP response. Also: an unsolicited `ControlDown` is delivered as an `on_notification`.
9. **CAS resolution** — a named-but-absent program → gateway floors gracefully (not a hang/crash).
10. **content-addressed routing** — router table carries a real hash; handler resolves from CAS by hash.

Then grow: WebSocket sessions, malformed frames, auth failures, reconnect/resilience, concurrency.

---

## 9. Build sequence (landable slices)

1. **This doc + PR** (interface). ← current step.
2. **Mock control server (Rust)** — `cdz-http-control-mock`: control-plane ws face (ship `ControlConfig`,
   push root-router, receive `ControlUp`, send `ControlDown`) + admin HTTP/JSON face (§3.3) + program
   manifest name-resolution + a nix build. Gated by its own crate check + a couple of Rust tests of the
   admin/wire translation (the mock's *own* correctness, not gateway behavior).
3. **Auto-enumerated program tree + deploy-templating + seeded CAS store + manifest** (§6) — the nix
   `http-outpost-harness` package. Verifiable standalone: build it, assert the manifest resolves + the store
   serves.
4. **Gateway boot-from-control** (the one gateway-side gap the harness needs; §6). Coordinate with whoever
   owns the gateway rewrite — the harness defines the contract (§4/§5/§6.4); the gateway implements it. If no
   active gateway owner, I build the minimal boot wiring against the existing `HttpEdge::dumb` + `RootDriver`
   as a stopgap so the corpus can run end-to-end, flagged for replacement.
5. **Python fixtures + runner + first scenarios** (§3, §8.1–2).
6. **Grow the corpus + migrate** the behaviors currently asserted by gateway `#[test]`s into scenarios, then
   **retire** the superseded `#[test]`s (don't keep both).

## 10. Open decisions / asks (non-blocking; proceeding on the defaults)

- **Wire codec home** (§7): extract `ControlConfig`/`ControlUp`/`ControlDown` to a shared `cdz-http-protocol`
  crate, or keep the mock depending on the gateway crate's `codec`? **Default:** extract, so the mock is
  decoupled from the gateway rewrite. (Coordinate with the gateway-rewrite owner when one exists.)
- **Router deploy-templating mechanism** (§6.2): reuse `cdz rewrite` for name→hash marker substitution if it
  can target arbitrary source markers, else a scoped `sed` in the nix deploy step. **Default:** whichever
  `cdz rewrite` supports cleanly; `sed` fallback otherwise.
- **Admin transport** (§3.3): HTTP/JSON (chosen — trivial from Python, human-debuggable). Not blocking.
- **Mock crate vs bin** (§7): new excluded crate (chosen, survives the rewrite).

None of these block starting the mock control server (slice 2); I'll raise anything genuinely load-bearing
to the concierge as an `ask` and keep building on the defaults.
