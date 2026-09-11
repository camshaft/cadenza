// SCENARIO (forward codec — query field): http-echo-query as the DIRECT root router echoes the decoded
// request's QUERY string, pinning that the gateway's encode_request query field (parts.uri.query(), boot.rs)
// round-trips into the guest's Value.decode(_ : Request). Complements echo-direct (which pins the method
// tag) — before this only method + path were exercised e2e, leaving the codec's `query` field untested.
//
// GET /?foo=bar&x=1 → the gateway parses the query as "foo=bar&x=1" (no leading '?'), so the echoed body is
// exactly those bytes. A codec that dropped or mangled the query field would fail this exact-body assertion.
{
  config = {
    root-router = "http-echo-query",
    programs = [ { name = "http-echo-query", program = "http-echo-query" } ],
  },
  requests = [
    { http = { method = "GET", path = "/?foo=bar&x=1" },
      expect = { status = 200, body = b"foo=bar&x=1" } },
  ],
}
