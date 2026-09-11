// SCENARIO (§8-"grow" — malformed frames): the gateway must TOLERATE a malformed/undecodable control frame on
// its control link — ignore it (or drop+redial) and keep serving, never crash/hang. A buggy control server or a
// corrupt wire could deliver garbage; the dumb gateway's FrameCodec::decode returns None for it, and that must
// not take down the data plane.
//
// http-hello is the direct root router (ignores the request, always 200s) — a request-independent responder, so
// this isolates control-frame robustness. Flow: baseline GET / → 200; a `push-garbage-frame` step (new mock admin
// PushGarbageFrame → the mock sends undecodable bytes, NOT a valid tagged ControlFrame, to the gateway session);
// then GET / (retry-until-match, polling past any brief reconnect if the gateway chose to drop+redial on the bad
// frame) → 200 again. A gateway that crashed / wedged on the garbage would fail step 3; a 200 proves it stayed
// healthy + serving.
{
  config = {
    root-router = "http-hello",
    programs = [ { name = "http-hello", program = "http-hello" } ],
  },
  requests = [
    // 1. Baseline: the booted gateway serves.
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"hello from a wasm handler" } },
    // 2. Deliver an undecodable control frame to the gateway.
    { control = { push-garbage-frame = true } },
    // 3. Still serving after the malformed frame (poll past any drop+redial the gateway may do).
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"hello from a wasm handler", retry-until-match = true } },
  ],
}
