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
- `requests` — an ordered list of interactions, each an `http` request OR a `control` injection:
  - `{ http = { method, path, headers = [ { name, value } ]?, body = b"…"?, compile-request = {…}? },
       expect = { … }? }`
  - `{ control = { push-root-router = "<name>" } }` — live-swap the root router (no restart).
  - `{ control = { push-down = { session = b"…"?, payload = b"…" } } }` — an unsolicited `ControlDown`.
  - `{ control = { prime-reply = { match-path = "…"?, reply = b"…" } } }` — prime the mock to answer a
    handler's `control.send`: when a `ControlUp` whose path matches `match-path` (absent ⇒ any) arrives, the
    mock replies a correlation-matched `ControlDown` carrying `reply`. Set it before the request that sends.

### `http` request fields

- `method` / `path` — required.
- `headers` — `[ { name, value }, … ]?` request headers (e.g. `content-type`).
- `body` — `b"…"?` the request body bytes.
- `compile-request` — `{ asts = [ { name, from-capture }, … ], entry = "<name>" }?`: build the `/compile`
  route's artifact-list body AT SEND TIME by invoking the real `cdz-http-compile-request` deploy tool (the
  single source of truth for the `CompileRoute` value shape). Each `asts` entry names a module + the
  `from-capture` (an earlier step's `capture-body-as`) holding its raw 33-byte `/parse` ast-hash; `entry` is
  the entrypoint module. Building at send time keeps the ast-hash a LIVE captured value — never a pinned
  constant. (Takes precedence over `body`.)

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

## The scenario corpus

Effect vocabulary + routing:
- `drive-root-router` — the booted root router answers directly (the baseline spine).
- `route-to-handler` — the baked router dispatches by (method, path); `/nope` → 404 deny.
- `routes-manifest` — the self-describing `/_routes` convention returns the baked route table.
- `echo-direct` — a handler `Value.decode`s the request + branches on method (the forward request-codec).
- `live-swap` — a control `push-root-router` hot-swaps the root router (retry-until-match past propagation).
- `control-send` — a handler's `control.send` round-trips (ControlUp → primed reply → on-response).
- `deadline-timeout` — a per-request deadline fires `Err(Timeout)` (gateway-enforced).
- `timer` — the fire-after timer effect (`FireAfter` → `Fired`).
- `casref` — a handler publishes a body to the CAS + answers a `http.response-cas` CasRef terminal.

`/parse` + `/compile` (reducer-target guests):
- `parse` — POST ml source → 200 + the ast-hash, which resolves in the CAS.
- `cross-surface` — ml source and its sexpr form parse to the SAME ast-hash (surface-agnostic; capture+compare).
- `compile` — two-phase: parse → capture ast-hash → live-swap to the compile handler → POST `/compile`
  (tool-built body) → 200 + a component ProgramHash that resolves to a real wasm component (`\x00asm` magic).
- `parse-diagnostics` — a malformed source → 400 whose body carries the parse diagnostic ("expected").
- `compile-diagnostics` — programs that parse but fail to COMPILE → 422 with the CDZ code ("CDZ0203",
  "nothing is public").
