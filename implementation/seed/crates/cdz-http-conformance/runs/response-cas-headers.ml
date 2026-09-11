// SCENARIO (reverse codec — CasRef path status + header): http-casref-headers as the DIRECT root router
// publishes a body via blobs.put and closes the http.response-cas terminal with a NON-200 status (201) + a
// response header, pinning the gateway's decode_response_cas status/header decode. That is a SEPARATE code path
// from decode_response (the CasRef path FETCHES the body from the CAS by hash), and the plain casref scenario
// only exercises 200 + [] headers — so a status/header bug specific to the CasRef decoder would slip through.
// This is the CasRef analogue of response-multi-header (inline path).
//
// GET / → the handler publishes b"cas-backed body bytes", answers response-cas{201, [x-conformance], hash};
// the gateway fetches the blob + serves it with the handler's status + header. Assert status 201, the fetched
// body, and the x-conformance header all forward.
{
  config = {
    root-router = "http-casref-headers",
    programs = [ { name = "http-casref-headers", program = "http-casref-headers" } ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 201,
                 body = b"cas-backed body bytes",
                 headers = [ { name = "x-conformance", value = "cas-header-ok" } ] } },
  ],
}
