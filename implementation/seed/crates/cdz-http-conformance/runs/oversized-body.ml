// SCENARIO (design §8 #5): a request body OVER the gateway's ceiling → 413 Payload Too Large, BEFORE routing.
// The edge must reject an oversized body up front (fast path on Content-Length; a lying/chunked length caught by
// a bounded read) so it never buffers it or drives a handler. Owned end-to-end with v-gateway-rewrite, who is
// landing the ceiling (~16 MiB, a baked safety limit) this cycle; this scenario is the e2e gate for that path.
//
// http-hello is the direct root router — it IGNORES the request and always answers 200. So an oversized POST
// that reaches routing would wrongly get 200; a 413 proves the ceiling fires FIRST, before http-hello is driven.
// A normal small POST still routes to http-hello (200), confirming the ceiling only rejects the oversized case.
//
// The oversized request declares `Content-Length: 20000000` (> the 16 MiB = 16777216 ceiling) with NO actual
// body (`declared-content-length`, sent over a raw socket). The gateway's fast path rejects on the declared
// length BEFORE reading a byte, so a CLIENT-VISIBLE 413 comes back with nothing uploaded — v-gateway-rewrite's
// recommended assertion. (A genuine multi-MiB upload instead races the gateway's close: the client sees a
// connection reset, not the 413 status, so we assert the declared-length fast path here.)
//
// The 413 ceiling is implemented + owned by v-gateway-rewrite (#8785, MAX_REQUEST_BODY = 16 << 20; #8787 drains
// the oversized body so the client's upload completes + it receives a CLIENT-VISIBLE 413 rather than a mid-upload
// reset). http-hello is the direct root router — it IGNORES the request and always 200s, so an oversized POST
// that reached routing would wrongly get 200; a 413 proves the ceiling rejects FIRST.
{
  config = {
    root-router = "http-hello",
    programs = [ { name = "http-hello", program = "http-hello" } ],
  },
  requests = [
    // A genuine 20 MiB upload (> the 16 MiB = 16777216 ceiling) → a client-visible 413: the gateway drains the
    // body (#8787) so the upload completes and the 413 status comes back (not a connection reset). body-fill
    // generates the body at send time (no huge literal in the run-spec).
    { http = { method = "POST", path = "/", body-fill = 20971520 },
      expect = { status = 413, body-contains = "exceeds the 16 MiB ceiling" } },
    // A normal small body still routes to http-hello → 200.
    { http = { method = "POST", path = "/", body = b"hi" },
      expect = { status = 200, body = b"hello from a wasm handler" } },
  ],
}
