// SCENARIO (design §8 #5): a request body OVER the gateway's ceiling → 413 Payload Too Large, BEFORE routing.
// The edge must reject an oversized body up front (fast path on Content-Length; a lying/chunked length caught by
// a bounded read) so it never buffers it or drives a handler. Owned end-to-end with v-gateway-rewrite, who is
// landing the ceiling (~16 MiB, a baked safety limit) this cycle; this scenario is the e2e gate for that path.
//
// http-hello is the direct root router — it IGNORES the request and always answers 200. So an oversized POST
// that reaches routing would wrongly get 200; a 413 proves the ceiling fires FIRST, before http-hello is driven.
// A normal small POST still routes to http-hello (200), confirming the ceiling only rejects the oversized case.
//
// The oversized body is GENERATED at send time via `body-fill` (20 MiB — comfortably over a ~16 MiB ceiling; no
// giant literal in the run-spec). If v-gateway-rewrite's final ceiling differs, bump this to stay clearly over.
//
// Test-first: RED until the gateway ceiling lands (until then the oversized POST returns http-hello's 200), then
// auto-greens. v-gateway-rewrite will ping at land with the exact ceiling value.
{
  config = {
    root-router = "http-hello",
    programs = [ { name = "http-hello", program = "http-hello" } ],
  },
  requests = [
    // Oversized body (20 MiB) → 413 before routing (http-hello never sees it).
    { http = { method = "POST", path = "/", body-fill = 20971520 },
      expect = { status = 413 } },
    // A normal small body still routes to http-hello → 200.
    { http = { method = "POST", path = "/", body = b"hi" },
      expect = { status = 200, body = b"hello from a wasm handler" } },
  ],
}
