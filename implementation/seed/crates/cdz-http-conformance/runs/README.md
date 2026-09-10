# Conformance run-specs (`DESIGN-http-outpost-conformance-harness.md` §3)

Each `*.ml` here is ONE black-box conformance scenario, written as a Cadenza value (ML surface — the same
style as `cdz-platform/harness-runs/*.ml`). The driver (`cdz-http-conformance`, the `cdz-platform-itest`
analogue) spins up the 3 SUT servers (the CAS store `cdz-cas-http`, the mock control server
`cdz-http-control-mock`, and the STOCK gateway `cdz-http-gateway`) on loopback sockets, executes the
scenario's `requests` against them, and asserts observable outcomes — verdict = exit code.

Adding a scenario = dropping a `*.ml` file here (auto-discovered; the programs it names are compiled from
`../programs/` and resolvable by name — see that tree). Everything is binary-AST end to end: the run-spec is
`cdz rewrite`-resolved (names → hashes) + encoded to binary-AST; the control plane + admin channel are
binary-AST; no JSON.

## The run value

A record with these fields (read by name; order-independent):

- `config` — the SUT setup:
  - `root-router` — the root-router program (by NAME; the mock resolves it to a `ProgramHash` and ships it in
    the `ControlConfig` on connect).
  - `programs` — `[ { name = "…", program = "…" }, … ]`: the Cadenza programs to make resolvable in the CAS
    (routers + handlers), each by name (nix compiles `../programs/…/reducer.cdz` + seeds the CAS by hash).
  - `prime-replies` — `[ { match = { path = "…"? }, reply = b"…" }, … ]?`: how the mock replies to an
    incoming `control.send` (correlation-routed).
- `requests` — an ordered list of interactions, each an action + an optional inline `expect`:
  - `{ http = { method = "GET", path = "/", headers = [ { name, value } ]?, body = b"…"? },
       expect = { status = 200, body = b"…"?, body-contains = "…"?, retry-until-match = true? } }`
  - `{ control = { push-root-router = "<name>" } }` — live-swap the root router (no restart).
  - `{ control = { push-down = { session = b"…"?, payload = b"…" } } }` — an unsolicited `ControlDown`.
- `checker` — optional; the blob name of a Cadenza reducer run over the observation log to judge pass/fail
  (like the platform §9 checkers). No checker ⇒ the run passes iff every inline `expect` held.

`retry-until-match` on an `expect` is the one non-linear primitive: poll the request until it matches (async
live-swap propagation).
