// SCENARIO: a handler's control.send ROUND-TRIP — the control-plane egress surface (#8720 ControlUp/ControlDown
// correlation), composed end to end:
//   1. prime-reply at the mock: when a ControlUp arrives on path /emit, answer a correlation-matched
//      ControlDown carrying b"PONG" (the RESPONSE to the handler's control.send).
//   2. GET /emit drives http-ctl (the root router): it emits a control.send (opaque payload) + CONTINUES; the
//      gateway wraps it in a ControlUp (stamping program/session/request-context) and forwards it UP; the mock
//      matches /emit and replies the correlated ControlDown b"PONG"; the gateway folds that back into the
//      handler's on-response, which closes with a 200 whose body is the control server's reply.
//   -> the HTTP response body is exactly what the control server sent DOWN, proving the round-trip + correlation.
//
// STATUS (test-first acceptance spec): RED — this pins a gateway bug the harness caught. The gateway does NOT
// forward the handler's control.send: with this scenario driven end to end, the mock captures ZERO ControlUps
// and the drive HANGS (the effect falls into the resolver's await-branch, not the control.send-forward branch).
// dispatch (route-to-handler) + response/deny (drive-root-router) all work, so the gateway's classification of
// THIS effect is the gap: the guest emits on control-send-descriptor().id (the canonical way, identical to the
// working dispatch), so the guest's control-send contract-id and the gateway's cdz_platform control_send id
// DISAGREE (a control-send-specific Cadenza-vs-Rust contract-id mismatch). Handed to v-gateway-rewrite; this
// scenario auto-greens when the ids agree (their session-registry + classify path is otherwise ready, #8720).
{
  config = {
    root-router = "http-ctl",
    programs = [ { name = "http-ctl", program = "http-ctl" } ],
  },
  requests = [
    { control = { prime-reply = { match-path = "/emit", reply = b"PONG" } } },
    { http = { method = "GET", path = "/emit" },
      expect = { status = 200, body = b"PONG" } },
  ],
}
