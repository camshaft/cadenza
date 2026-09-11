// SCENARIO (reverse codec — response status + multi-header): http-multi-response as the DIRECT root router
// answers a handler-CHOSEN status (201) + a TWO-element List(Header), pinning the reverse (response) codec: the
// gateway's decode_response reads the status the handler set (not a hardcoded 200) and forwards EVERY header in
// the list (not just the first). drive-root-router/route-to-handler pin a SINGLE response header on a 200 — a
// single-header 200 can't catch a first-header-only bug or a hardcoded status, so this closes those gaps.
{
  config = {
    root-router = "http-multi-response",
    programs = [ { name = "http-multi-response", program = "http-multi-response" } ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 201,
                 body = b"created",
                 headers = [ { name = "content-type",  value = "text/plain" },
                             { name = "x-conformance", value = "multi-header-ok" } ] } },
  ],
}
