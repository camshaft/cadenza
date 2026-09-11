// SCENARIO (forward codec — headers field): http-echo-header as the DIRECT root router echoes the VALUE of a
// named request header, pinning that the gateway's encode_request `headers` field (each request header encoded
// as a Header record into List(Header)) is READABLE in the guest via Value.decode(_ : Request). Completes the
// forward-codec field coverage (method: echo-direct, query: request-query-decode, body: request-body-decode) —
// before this only that a header-BEARING request DECODES was gated (echo-direct), not that a guest reads a
// header's VALUE.
//
// The handler recurses List(Header) for x-probe (the gateway lowercases request header names, so it matches the
// lowercased name). Request 1 sends X-Probe: probe-value-42 → the echoed body is that value. Request 2 sends no
// x-probe → the (absent) sentinel, pinning the not-found terminal of the header scan too.
{
  config = {
    root-router = "http-echo-header",
    programs = [ { name = "http-echo-header", program = "http-echo-header" } ],
  },
  requests = [
    { http = { method = "GET", path = "/", headers = [ { name = "x-probe", value = "probe-value-42" } ] },
      expect = { status = 200, body = b"probe-value-42" } },
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"(absent)" } },
  ],
}
