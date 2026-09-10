# HTTP-outpost — the gateway ↔ looping-program DRIVE CONTRACT (redirect increments 3–5)

Status: DESIGN (v-http-outpost, 2026-09-10). Pins how the *dumb* gateway drives the control-server-provided
programs as **looping reducers that emit effects/timers and exchange messages with the control server**
(operator redirect + the looping/effects + bidirectional-enveloped-messaging directives). Companion to
`DESIGN-http-outpost.md`; supersedes that doc's one-shot handler + in-gateway routing model.

## 0. The key finding — no platform gap
The reducer world (`cdz-platform/src/reducer.rs`) **already exposes the entire surface** a looping,
effect-emitting, timer-setting, message-exchanging program needs. Nothing is missing in cdz-platform; the
work is entirely gateway-side (the driver) plus a small **effect-contract vocabulary** the outpost defines.

| Need | Existing surface |
|---|---|
| loop across turns | `Outcome::Continue` keeps the reducer alive; `Break{schema,reason}` ends it |
| emit effects | `on_message`/`on_response`/`on_notification` all return `(Vec<Request>, Outcome)` |
| an effect | `Request { id: ContractId, payload: Bytes, continuation_token: Bytes, deadline: Option<Duration> }` |
| a timer | a `Request` with `deadline: Some(d)` — no answer in `d` folds `Err(Timeout)` |
| fold an effect's answer | `on_response(Response { id, continuation_token, payload: Result<Bytes,Error> })` |
| receive an unsolicited push | `on_notification(Notification { id, payload })` |
| provenance | `Message.from: Origin { reducer, host }` (unforgeable envelope metadata) |

The gateway's current `HandlerRunner` (one `on_message` → expect `Break`) ignores `Continue` + the emitted
`Vec<Request>`; the driver below uses them.

## 1. The drive loop (gateway-side, inc-3)
A program (the root router, or a subprogram) is a persistent reducer instance the gateway DRIVES:

```
spawn(program)                                  # from CAS by hash (HttpCas, #8623)
inputs = queue[ on_message(the triggering event) ]
loop:
  (requests, outcome) = reducer.<fold>(next input)   # on_message / on_response / on_notification
  for req in requests: carry_out(req)                # §2 — route by req.id, opaque payload
  if outcome == Break{schema, reason}: finish(schema, reason); stop
  if no inputs pending and the connection is open: await the next event (socket frame / control push /
                                                    timer fire / a carried-out request's response)
  else if connection closed: deliver on_notification(closed) then stop
```

- **Persistent**: one instance per connection/session (the root router may be one per gateway, or per
  connection — v0: per connection, matching the ws-session model). State survives across folds.
- **Effects out, answers in**: `carry_out(req)` performs the effect (§2) and, when it produces an answer,
  feeds it back via `on_response` correlated by `req.continuation_token`. A `deadline` arms a timer that
  folds `Err(Timeout)` if no answer lands in time.
- **Break** is one possible outcome, not the immediate expectation.

## 2. The effect-contract vocabulary (outpost-defined; the gateway routes by `req.id`, payload OPAQUE)
The gateway is a pure router of effects: it switches on the contract-id and never inspects the payload
beyond the fields a given effect's envelope needs. v0 contracts (33-byte markers now; real contract-ids when
the userspace contracts land — schema-id fix):

- **`cdz.http.dispatch`** — "spawn this subprogram and hand it this input." Payload names a subprogram
  `ProgramHash` + the input bytes. The gateway fetches the subprogram from CAS, spawns it, drives ITS loop
  (§1, recursively), and folds its terminal `Break` reason back as the emitter's `on_response`. This is how
  the ROOT ROUTER dispatches to a handler.
- **`cdz.http.response`** — "answer the current HTTP request with this http-response." The gateway writes it
  to the socket. (A handler that directly answers; or the router answering 404/deny.)
- **`cdz.ws.send`** — "push this frame to the connection" (the existing ws-send effect, §6).
- **`cdz.http.deny`** — "reject this request / ws-upgrade" (status + reason). The gateway floors/closes.
- **`cdz.control.send`** — "send this OPAQUE payload to the control server." The gateway wraps it in an
  ENVELOPE with provenance (§3) and forwards up the control link. An answer (if the effect expects one)
  folds back via `on_response`.
- a **timer** is any `Request` with `deadline: Some(d)` — no dedicated contract; the gateway arms it and
  folds `Err(Timeout)`/a timer notification when it fires.

Adding an effect class = adding a contract-id arm to the gateway's `carry_out`; the payload stays the
program's business.

## 3. Bidirectional enveloped handler ↔ control messaging (4th directive)
The control link (`control_link.rs`) carries, beyond `ControlConfig` (#8622) + root-router-hash pushes, an
**enveloped-message frame both ways** — the gateway is a pure opaque router (provenance/addressing only,
never payload):

- **UP** (`cdz.control.send` effect → control): `Envelope { provenance: { program: ProgramHash, session:
  Bytes }, payload: Bytes }`. The gateway fills `provenance` (which program instance / connection emitted
  it); `payload` is opaque.
- **DOWN** (control push → handler): `Envelope { addressing: { session: Bytes }, payload: Bytes }`. The
  gateway routes by `addressing` to the target reducer instance and folds it as an `on_notification`
  (contract = `cdz.control.message`), `payload` opaque.

The envelope is binary-AST (self-describing). The gateway never decodes `payload`.

## 4. Sequencing
- **inc-3** (gateway): the drive loop (§1) + `carry_out` for the effect vocabulary (§2) + the enveloped-
  message up/down plumbing (§3), driving programs fetched from CAS. Replaces `HandlerRunner`'s one-shot fold.
  Add the `Envelope` codec (up/down) alongside `ControlConfig`.
- **inc-4** (guest): the ROOT ROUTER program — a looping Cadenza reducer that folds requests and emits
  `cdz.http.dispatch`/`cdz.http.deny`/`cdz.control.send` effects. The routing table is its own concern
  (supersedes the in-gateway `DynamicRouter`).
- **inc-5** (boot): dial control → `ControlConfig` → fetch + drive the root router; root-router-hash pushes
  swap it live (#8619 cell, now holding a hash).

## 5. Open question routed to the concierge
None blocking. The drive contract is a gateway-side design (this doc); no cdz-platform reducer/effect/timer
piece is missing, so no platform vertical is needed. The effect-contract-id set (§2) firms up as inc-4's
router guest is authored; the schema-id fix (descriptor-derived ids) applies to these contracts too.
