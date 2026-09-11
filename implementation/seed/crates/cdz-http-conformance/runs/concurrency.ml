// SCENARIO (§8-"grow" — concurrency/isolation): the gateway must handle many IN-FLIGHT requests at once, driving
// a FRESH mailbox per request with no cross-request race, deadlock, or state corruption. Every other scenario
// drives steps sequentially; this fires a burst concurrently.
//
// http-hello is the direct root router — it ignores the request + always answers 200 "hello…", so it's the ideal
// request-independent responder for a pure concurrency probe (isolate concurrency from any request-codec /
// per-request state). `concurrency = 25` fires 25 copies of GET / at once; EVERY response must be 200 with the
// expected body. A gateway that serialized, deadlocked, or mixed up per-request state under load would fail some.
{
  config = {
    root-router = "http-hello",
    programs = [ { name = "http-hello", program = "http-hello" } ],
  },
  requests = [
    { http = { method = "GET", path = "/", concurrency = 25 },
      expect = { status = 200, body = b"hello from a wasm handler" } },
  ],
}
