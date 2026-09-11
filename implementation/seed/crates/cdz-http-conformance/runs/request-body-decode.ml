// SCENARIO (forward codec — body field): http-echo-body as the DIRECT root router echoes the decoded
// request's BODY bytes verbatim, pinning that the gateway's encode_request body field round-trips into the
// guest's Value.decode(_ : Request). Complements echo-direct (method) + request-query-decode (query): the
// request body carries the arbitrary payload a handler processes, and before this nothing proved a guest
// reads the exact request-body bytes back (echo-direct's POST sends a body but only inspects the method).
//
// POST / with a body → the echoed 200 body must equal the request body byte-for-byte. A codec that dropped
// or truncated the body field would fail this exact-body assertion.
{
  config = {
    root-router = "http-echo-body",
    programs = [ { name = "http-echo-body", program = "http-echo-body" } ],
  },
  requests = [
    { http = { method = "POST", path = "/", body = b"request body payload 42" },
      expect = { status = 200, body = b"request body payload 42" } },
  ],
}
