// SCENARIO (§8 "grow" — slowloris hardening, co-designed with v-gateway-rewrite): a client that DECLARES a
// request body but then STALLS (sends no bytes, holds the connection) must be floored by the gateway's body-read
// IDLE timeout → 408 Request Timeout, rather than tying up the edge forever. This complements oversized-body
// (413, a too-BIG body) with the too-SLOW body case.
//
// http-hello is the direct root router (ignores the request, always 200s if reached). The request declares
// Content-Length 1000 (UNDER the 16 MiB ceiling, so this is the IDLE path, not a 413) but sends NO body — a
// stalled/no-send client (the `stalled-content-length` cap: a raw socket that writes the head then holds open).
// The gateway's idle timeout floors it 408 (the driver waits well past the timeout to observe a real 408, not a
// hang). An active-but-slow upload would keep sending bytes and survive (idle-based); only a stall trips it.
//
// Test-first: RED until v-gateway-rewrite lands the body-read idle timeout (until then the no-send client hangs
// and the driver's stalled-read budget elapses → a clear read-timeout step error); auto-greens on land.
{
  config = {
    root-router = "http-hello",
    programs = [ { name = "http-hello", program = "http-hello" } ],
  },
  requests = [
    // Declare a (sub-ceiling) body, send nothing, hold open → the gateway's idle timeout floors it 408.
    { http = { method = "POST", path = "/", stalled-content-length = 1000 },
      expect = { status = 408 } },
    // A normal small body still routes to http-hello → 200 (the idle timeout only floors a stalled client).
    { http = { method = "POST", path = "/", body = b"hi" },
      expect = { status = 200, body = b"hello from a wasm handler" } },
  ],
}
