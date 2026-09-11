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
// GREEN. This exercised the harness's first control.send and caught + fixed a real bug: the gateway speaks
// TAGGED `ControlFrame` envelopes (FrameCodec, operator tagged-frame directive), and the mock originally read
// BARE — so it captured 0 ControlUps and the handler hung. The mock now speaks the tagged FrameCodec on both
// faces (it builds FrameCodec::new(config,up,down) from the 3 canonical frame ids the rig sources from the
// contract-declarations manifest), so the ControlUp reaches the mock, its correlation-matched ControlDown
// folds back into on-response, and the body is the primed reply. (The control_send effect ids always agreed.)
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
