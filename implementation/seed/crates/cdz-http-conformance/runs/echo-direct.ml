// SCENARIO (isolation): http-echo as the DIRECT root router — proves the FORWARD request-codec end to end
// (the gateway's encoded http-request decodes in a guest via Value.decode(_ : Request)). Unlike live-swap
// this needs no swap, so it separates "does a guest decode a real request" from the live-swap delivery path.
{
  config = {
    root-router = "http-echo",
    programs = [ { name = "http-echo", program = "http-echo" } ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body-contains = "method=GET" } },
  ],
}
