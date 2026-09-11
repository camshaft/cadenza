# Conformance run-specs (`DESIGN-http-outpost-conformance-harness.md` §3)

Each `*.ml` here is ONE black-box conformance scenario, written as a Cadenza value (ML surface — the same
style as `cdz-platform/harness-runs/*.ml`). The driver (`cdz-http-conformance`, the `cdz-platform-itest`
analogue) spins up the 3 SUT servers (the CAS store `cdz-cas-http`, the mock control server
`cdz-http-control-mock`, and the STOCK gateway `cdz-http-gateway`) on loopback sockets, executes the
scenario's `requests` against them, and asserts observable outcomes — verdict = exit code.

Adding a scenario = dropping a `*.ml` file here (auto-discovered as `checks.<sys>.http-conformance-<name>`; the
programs it names are compiled from `../programs/` and resolvable by name — see that tree). Everything is
binary-AST end to end: the run-spec is `cdz convert`-compiled (ML → binary-AST); the control plane + admin
channel are binary-AST; no JSON.

> **Gate note for gateway/contract changes.** The `http-conformance-<name>` checks are the real end-to-end gate
> for the guest request-DECODE path — a change to the gateway's request encoding (`encode_request_value`) that
> the crate/build checks (`cdz-http-gateway`, `cdz-http-conformance`) pass can still break the guest's compiled
> `Value.decode : Option(Request)` (see the #8770 → #8775 episode). The cheapest smoke is
> `http-conformance-{echo-direct,parse}` (no `rcdzc`); the `compile*` scenarios add the heavier compile path.

## The run value

A record with two fields (read by name; order-independent):

- `config` — the SUT setup:
  - `root-router` — the root-router program by NAME (the mock resolves it to a `ProgramHash` and ships it in
    the `ControlConfig` on connect).
  - `programs` — `[ { name = "…", program = "…" }, … ]`: the programs to make resolvable in the CAS (routers +
    handlers), each by name. In-tree guests compile from `../programs/…/reducer.cdz`; heavier reducer-target
    guests (`reducer-guest-{parse,ml,sexpr,rcdzc,compile}`) are seeded per-scenario via the flake's
    `httpConformanceExternalGuests.<scenario>` map so unrelated runs don't build them.
  - `cas-write-credential` — `"…"?`: OVERRIDE the CAS write credential the mock ships the gateway (default ⇒ the
    correct seed). Set WRONG to exercise the auth-failure path — the harness CAS always expects the fixed seed.
- `requests` — an ordered list of interactions, each an `http` request OR a `control` injection:
  - `{ http = { method, path, headers = [ { name, value } ]?, body = b"…"?, compile-request = {…}? },
       expect = { … }? }`
  - `{ control = { push-root-router = "<name>" } }` — live-swap the root router (no restart).
  - `{ control = { push-down = { session = b"…"?, payload = b"…" } } }` — an unsolicited `ControlDown`.
  - `{ control = { prime-reply = { match-path = "…"?, reply = b"…" } } }` — prime the mock to answer a
    handler's `control.send`: when a `ControlUp` whose path matches `match-path` (absent ⇒ any) arrives, the
    mock replies a correlation-matched `ControlDown` carrying `reply`. Set it before the request that sends.
  - `{ control = { drop-control = true } }` — close the gateway's control-ws server-side, forcing it to
    REDIAL (reconnect/resilience testing; the mock re-ships the config on reconnect).
  - `{ control = { push-garbage-frame = true } }` — send an UNDECODABLE control frame; the gateway must
    tolerate it (ignore/recover, not crash/hang).

### `http` request fields

- `method` / `path` — required.
- `headers` — `[ { name, value }, … ]?` request headers (e.g. `content-type`).
- `body` — `b"…"?` the request body bytes.
- `compile-request` — `{ asts = [ { name, from-capture }, … ], entry = "<name>", wit-world = { name,
  from-capture }? }?`: build the `/compile` route's artifact-list body AT SEND TIME by invoking the real
  `cdz-http-compile-request` deploy tool (the single source of truth for the `CompileRoute` value shape). Each
  `asts` entry names a module + the `from-capture` (an earlier step's `capture-body-as`) holding its raw 33-byte
  `/parse` ast-hash; `entry` is the entrypoint module. `wit-world` (optional) names a `kind="wit-world"` artifact
  by a `from-capture` holding its hash — required to `/compile` a reducer-world guest (a driver-seeded artifact,
  e.g. `reducer-world`). Building at send time keeps every hash a LIVE captured value — never a pinned constant.
  (Takes precedence over the other body fields.)
- `body-source` — `"<name>"?`: read the body AT SEND TIME from a staged in-tree module source
  (`<name>.cdz` under `CDZ_HARNESS_MODULE_SOURCES_DIR`, staged per-scenario by the flake) — POST a real
  lib/contract source to `/parse` without embedding + drifting its text here. (Precedence: after `compile-request`.)
- `body-fill` — `<n>?`: generate an `n`-byte filler body AT SEND TIME — POST a large body (e.g. over the 16 MiB
  ceiling → a client-visible 413, since the gateway drains the oversized body) without a huge literal here.
- `body-nonce` — `true?`: generate a per-run-UNIQUE body at send time (pid+nanos+counter) — for a handler
  that publishes it, the CAS hash is fresh every run (used by the auth-failure scenario: a swallowed denied write
  leaves a fresh hash unresolvable → a deterministic 502).
- `stalled-content-length` — `<n>?`: over a RAW socket, declare `Content-Length: n` in the request head then
  send NO body and hold the connection open — a stalled/slowloris client. Exercises the gateway's body-read IDLE
  timeout (→ 408) without a real upload. Bypasses the reqwest client (raw TCP); the driver waits past the idle
  timeout to observe a real 408 vs. a hang.
- `concurrency` — `<n>?`: fire this request `n` times CONCURRENTLY (all in flight) and require every
  response to pass the step's `expect` — the concurrency/isolation gate (bypasses the `body*`/capture machinery).

### `expect` fields (all optional; a `None` field asserts nothing)

- `status` — the exact status code.
- `body` — the exact response body bytes.
- `body-contains` — a substring the body must contain (robust to wording tweaks — prefer for diagnostics).
- `headers` — `[ { name, value }, … ]` response headers that must be present (name case-insensitive).
- `retry-until-match` — `true` ⇒ re-issue the request until the assertion holds or a timeout elapses. The one
  non-linear primitive, for async propagation (e.g. a live root-router swap applied on a later request).
- `resolves-in-cas` — `true` ⇒ treat the response body as a raw 33-byte content hash, base62-encode it, and
  assert the blob RESOLVES (a non-empty CAS GET). Proves a `blobs.put` publish persisted (a `/parse` ast-hash,
  a `/compile` component hash).
- `cas-body-starts-with` — `b"…"?`: after `resolves-in-cas`, assert the resolved blob STARTS WITH these bytes
  (e.g. the wasm magic `b"\x00asm"` for a `/compile` component — note the NUL is `\x00`, not `\0`).
- `capture-body-as` — `"<name>"?`: on pass, store the response body under this name for a later step.
- `body-equals-capture` — `"<name>"?`: assert the body equals a value captured by an earlier `capture-body-as`
  (a structural cross-step check that pins no machine-specific value — e.g. the cross-surface invariant).
- `times-out` — `true?`: assert the gateway does NOT respond within a bounded budget — i.e. the request HANGS.
  Mutually exclusive with the response-shape fields (there is no response to check). The RED-negative for the
  dispatch fold: a compiled router whose `on-response` never folds (the inert `Continue`) leaves the caller
  waiting forever (see `compile-dispatch-inert-fold`).

## The scenario corpus

Effect vocabulary + routing:
- `drive-root-router` — the booted root router answers directly (the baseline spine).
- `route-to-handler` — the baked router dispatches by (method, path); `/nope` → 404 deny.
- `routes-manifest` — the self-describing `/_routes` convention returns the baked route table.
- `echo-direct` — a handler `Value.decode`s the request + branches on method (the forward request-codec);
  drives GET/POST/DELETE so the method-tag decode is pinned across variants.
- `request-query-decode` — a handler echoes the decoded request's `query` string (`GET /?foo=bar&x=1` → body
  `foo=bar&x=1`), pinning the forward codec's `query` field round-trips into the guest (complements the method
  coverage above).
- `request-body-decode` — a handler echoes the decoded request's `body` bytes verbatim (`POST /` with a body →
  the same bytes back), pinning the forward codec's `body` field round-trips into the guest.
- `request-header-decode` — a handler recurses `List(Header)` for a named header (`x-probe`) and echoes its
  value (`X-Probe: v` → body `v`; absent → `(absent)`), pinning the forward codec's `headers` field is READABLE
  in the guest (names arrive lowercased). Completes the forward-codec field coverage (method/path/query/body/headers).
- `response-multi-header` — a handler answers a chosen status (`201`) + a TWO-element `List(Header)`, pinning the
  REVERSE (response) codec: the gateway forwards the handler's status (not a hardcoded 200) and EVERY header (not
  just the first) — invariants a single-header 200 can't catch.
- `response-cas-headers` — the CasRef analogue: a `blobs.put` handler answers `http.response-cas` with a non-200
  status (`201`) + a header, pinning the gateway's `decode_response_cas` status/header decode (a SEPARATE path
  from `decode_response`; the plain `casref` only exercises `200` + no headers).
- `live-swap` — a control `push-root-router` hot-swaps the root router (retry-until-match past propagation).
- `control-send` — a handler's `control.send` round-trips (ControlUp → primed reply → on-response).
- `deadline-timeout` — a per-request deadline fires `Err(Timeout)` (gateway-enforced).
- `timer` — the fire-after timer effect (`FireAfter` → `Fired`).
- `casref` — a handler publishes a body to the CAS + answers a `http.response-cas` CasRef terminal.
- `missing-program` — a dispatch to a deliberately-absent program hash → the gateway floors GRACEFULLY (a 502,
  no hang/crash): spawn can't fetch the hash, injects `Err`, the handler folds it into a deny (design §8 #9).
- `oversized-body` — a request body over the 16 MiB ceiling → 413 BEFORE routing (design §8 #5): a genuine large
  upload (`body-fill`) is drained by the gateway so the client sees a real 413, not a mid-upload reset; a small
  body still routes → 200.
- `slow-upload` — the too-SLOW counterpart to oversized-body's too-BIG: a `stalled-content-length` client declares
  a (sub-ceiling) body then STALLS → the gateway's body-read idle timeout floors it 408 (#8801, slowloris hardening);
  a normal small body still routes → 200 (idle-based, so only a genuine stall trips it).
- `cas-auth-failure` — a handler `blobs.put` with a WRONG control-shipped CAS write credential (config
  `cas-write-credential`): the CAS rejects the write (401) but the infallible-shaped `blobs.put` WIT SWALLOWS it, so
  it surfaces DOWNSTREAM — the published (unique) CasRef hash is absent → the gateway floors 502 (§8-grow auth failure).
- `reconnect` — a `drop-control` closes the gateway's control link → it must REDIAL (#8741) + recover: a
  baseline GET / → 200, then after the drop a retried GET / → 200 (a control blip doesn't break the data plane).
- `concurrency` — 25 concurrent GET / to http-hello → all 200: the gateway drives a fresh mailbox per
  request with no cross-request race/deadlock/corruption under load (§8-grow concurrency).
- `malformed-frame` — a `push-garbage-frame` delivers undecodable bytes on the control link → the gateway
  tolerates it (baseline GET /→200, garbage frame, retried GET /→200); a crash/hang would fail (§8-grow malformed frames).

`/parse` + `/compile` (reducer-target guests):
- `parse` — POST ml source → 200 + the ast-hash, which resolves in the CAS.
- `cross-surface` — ml source and its sexpr form parse to the SAME ast-hash (surface-agnostic; capture+compare).
- `compile` — two-phase: parse → capture ast-hash → live-swap to the compile handler → POST `/compile`
  (tool-built body) → 200 + a component ProgramHash that resolves to a real wasm component (`\x00asm` magic).
- `parse-diagnostics` — a malformed source → 400 whose body carries the parse diagnostic ("expected").
- `compile-diagnostics` — programs that parse but fail to COMPILE → 422 with the CDZ code ("CDZ0203",
  "nothing is public").
- `reducer-world-compile` — `/compile` a real reducer-world GUEST: parse the guest (http-hello) + its full
  lib/contract closure (8 modules, via `body-source`), then `/compile` the artifact list + a `kind="wit-world"`
  reducer-world artifact → a real wasm component (`\x00asm`), then INSTALL it as root + SERVE (compile→install→serve).
- `compile-dispatch` — the compile→root→DISPATCH path: `/compile` a dispatching ROUTER (the deploy-templated
  root-router-baked source, its placeholder handler hashes substituted with the seeded http-hello/http-echo
  ProgramHashes) from its 8-module closure, root the freshly-built component (`push-root-router` from-capture,
  #8811), then dispatch: `GET /` → 200 (router folds http-hello's response through its on-response), `GET /nope`
  → 404 deny. Closes the compile-ok-but-dispatch-hangs gap (the router's fold is the exact bug-class guard).
- `compile-dispatch-inert-fold` — the RED-negative partner of `compile-dispatch` (bidirectional guard): the SAME
  router source with its `on-response` fold DELETED (resolves to the inert `Continue`). It `/compile`s fine and
  serves the Close path (`GET /_routes` → 200), but a dispatch (`GET /`) HANGS — the missing fold never returns
  the child's answer. Asserts `times-out` on `GET /`. Real fold → 200 (positive) ; inert fold → hang (this).

Browser outpost (owned by the `v-browser-outpost` vertical — it drops `browser-*.ml` + their handlers/routers
here; they auto-discover as `http-conformance-browser-*` checks):
- `browser-page` / `browser-outpost` / `browser-baked-app` — a reducer serves HTML + JavaScript through the
  gateway with the `content-type` forwarded verbatim (S0: a direct handler; S1a: a method-aware router — `GET /`→html,
  `GET /app.js`→js, unmatched→404, non-GET→405; and a baked-app variant serving a bundled page). This vertical only
  guarantees the harness runs them; their scenario semantics + additions are that vertical's.
