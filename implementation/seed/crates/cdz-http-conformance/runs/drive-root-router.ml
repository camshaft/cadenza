// SCENARIO: the gateway DRIVES the control-shipped root-router program per request + folds its response.
//
// Proves the drive path landed in #8716: the mock ships a ControlConfig naming a root-router ProgramHash; the
// gateway fetches that program (+ its dependency-closure components — the value-heap runtime the guest imports)
// from the CAS, drives it over a fresh mailbox (drive.rs + GatewayResolver) per request, and maps its terminal
// Break to the HTTP response (an http.response terminal -> status/headers/body). The root router here is
// http-hello — a program that DIRECTLY answers 200 "hello from a wasm handler" in on-message (no dispatch to a
// separate handler). So this isolates the drive+fold+spawn-closure path from the dispatch path
// (route-to-handler.ml, which additionally needs the router to bake real handler hashes).
{
  config = {
    root-router = "http-hello",
    programs = [
      { name = "http-hello", program = "http-hello" },
    ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"hello from a wasm handler" } },
  ],
}
