// SCENARIO: the root router routes GET / to the hello handler; an unmatched path denies.
//
// The mock ships the baked root router (which bakes the handlers' real ProgramHashes — deploy-templated at
// build time); the gateway fetches it from the CAS, drives it per request, and the router dispatches GET /
// to the http-hello handler (whose 200 body folds back through the router's on-response to the socket). An
// unmatched path (GET /nope) hits the router's deny terminal — the gateway HONORS the deny's status + reason
// (#8745): the router denies with deny(404, b"not found"), so the response is 404 with body "not found" (a
// malformed/out-of-range deny would fall back to a plain 403).
{
  config = {
    root-router = "root-router-baked",
    programs = [
      { name = "root-router-baked", program = "root-router-baked" },
      { name = "http-hello",        program = "http-hello" },
      { name = "http-echo",         program = "http-echo" },
    ],
  },
  requests = [
    // The dispatched handler's content-type must SURVIVE the router's on-response fold back to the socket —
    // a distinct path from drive-root-router's DIRECT header forwarding (which pins the same text/plain on
    // http-hello answering directly). Pins that header forwarding through the router fold isn't dropped/mangled.
    { http = { method = "GET", path = "/" },
      expect = { status = 200,
                 body = b"hello from a wasm handler",
                 headers = [ { name = "content-type", value = "text/plain" } ] } },
    { http = { method = "GET", path = "/nope" },
      expect = { status = 404, body = b"not found" } },
  ],
}
