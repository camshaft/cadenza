// SCENARIO: a LIVE root-router swap over the control plane — the http-outpost's hot-reconfigure surface.
//
// Composes the control-driven live-swap end to end (#8716 atomic drive-context rebuild + #8718 push-root-router
// control step + #8721 drive/spawn-closure + #8723 retry-until-match, on the boot SETTLE gate that removed the
// first-request 503 race):
//   1. The mock ships a ControlConfig naming http-hello as the root router; the gateway boots from it and
//      GET / drives http-hello -> 200 "hello from a wasm handler" (the baseline, exactly drive-root-router).
//   2. A `push-root-router http-echo` control injection at the mock pushes an unsolicited ControlDown to the
//      connected gateway, which rebuilds its drive context atomically to serve http-echo as the root router.
//   3. GET / (retry-until-match — the swap propagates async, so poll past the pre-swap "hello" response)
//      now drives http-echo, which Value.decodes the delivered http-request + branches on the method:
//      a GET folds back 200 "method=GET". Matching that body confirms the NEW router is live.
//
// STATUS (test-first acceptance spec): steps 1 + 2 GREEN — the settle gate, the control push, and the live-swap
// MECHANISM all work (the swap DOES apply: step 3 reaches http-echo, which answers). Step 3 is RED pending the
// FORWARD REQUEST-CODEC: http-echo's `Value.decode(msg.payload) : Option(Request)` returns None on the gateway's
// encoded http-request (→ its 400 undecodable-branch), because http-lib decodes with a hand-written "compatible
// copy" of Request/Method that does not structurally match the `http_request` CONTRACT the gateway encodes with
// (encode_request via reqc::request_request; nullary Method as `(Ctor unit)`). The fix is to decode with the
// contract's own Request type (reuse, don't redefine) — a follow-up unit; this scenario auto-greens once it lands.
//
// Both programs are registered (config.programs), so the mock resolves http-echo's ProgramHash for the push;
// retry-until-match is the one non-linear primitive, exactly for this async-propagation shape.
{
  config = {
    root-router = "http-hello",
    programs = [
      { name = "http-hello", program = "http-hello" },
      { name = "http-echo",  program = "http-echo" },
    ],
  },
  requests = [
    // 1. Baseline: the booted root router (http-hello) answers directly.
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"hello from a wasm handler" } },
    // 2. Live-swap the root router to http-echo over the control plane.
    { control = { push-root-router = "http-echo" } },
    // 3. The swapped router is now live: GET / drives http-echo (poll past the async propagation).
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"method=GET", retry-until-match = true } },
  ],
}
