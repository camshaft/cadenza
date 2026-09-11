// SCENARIO (isolation): http-echo as the DIRECT root router — proves the FORWARD request-codec end to end
// (the gateway's encoded http-request decodes in a guest via Value.decode(_ : Request)). Unlike live-swap
// this needs no swap, so it separates "does a guest decode a real request" from the live-swap delivery path.
//
// The handler branches on r.method and echoes the decoded variant (method=GET/POST/DELETE/…). Driving MORE
// than one method pins that the codec reads the method TAG field correctly across variants — the exact
// request-decode path that regressed in the #8770/#8794 ascription episodes (a codec that mangled the method
// discriminant would echo the wrong variant or fail to decode). POST also carries a body, so it additionally
// pins that a non-empty request body doesn't perturb the method decode.
{
  config = {
    root-router = "http-echo",
    programs = [ { name = "http-echo", program = "http-echo" } ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body-contains = "method=GET" } },
    { http = { method = "POST", path = "/", body = b"a request body" },
      expect = { status = 200, body-contains = "method=POST" } },
    { http = { method = "DELETE", path = "/" },
      expect = { status = 200, body-contains = "method=DELETE" } },
  ],
}
