// SCENARIO: a CAS-BACKED response body (the http.response-cas / CasRef terminal, §6), composed e2e.
//
// http-casref is the root router: GET / makes it blobs.put(b"cas-backed body bytes") -> get the hash -> close
// with http.response-cas{200, [], body_hash=hash}. The gateway fetches the blob from its shared write-capable
// CAS by that hash and serves it. So the HTTP body the client sees is exactly the bytes the handler PUT (not
// an inline body) -> proves blobs.put persisted + the CAS-fetch response path (§6 CasRef half). Also the first
// harness scenario exercising blobs.put, the write path the /parse and /compile e2es will build on.
{
  config = {
    root-router = "http-casref",
    programs = [ { name = "http-casref", program = "http-casref" } ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"cas-backed body bytes" } },
  ],
}
