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
// STATUS (test-first acceptance spec): RED — pins a bug the harness caught, and the fix is in THIS vertical's
// lane (the mock control server). ROOT CAUSE (root-caused with v-gateway-rewrite): NOT an id mismatch — the
// control_send ids AGREE (the drive HANGS rather than 502-ing, which proves forward_control_send fired). The
// bug is a control-link FRAME-ENCODING mismatch: the gateway writes/reads TAGGED `ControlFrame` envelopes
// (FrameCodec, operator tagged-frame directive), but the mock reads BARE (`decode_control_up` on the raw
// frame) → fails → 0 ControlUps captured → never replies → the handler hangs awaiting the ControlDown. FIX
// (next unit, mine): make the mock speak the tagged FrameCodec on both faces — the driver/rig sources the 3
// canonical frame ids (control_config/up/down, from the contract-declarations manifest) and the mock builds
// FrameCodec::new(config,up,down), decoding inbound + sending Config/Down tagged. Auto-greens once landed.
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
