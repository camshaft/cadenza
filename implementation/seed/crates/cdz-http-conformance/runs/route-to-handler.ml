// SCENARIO: the root router routes GET / to the hello handler; an unmatched path denies.
//
// The mock ships the baked root router (which bakes the handlers' real ProgramHashes — deploy-templated at
// build time); the gateway fetches it from the CAS, drives it per request, and the router dispatches GET /
// to the http-hello handler (whose 200 body folds back through the router's on-response to the socket). An
// unmatched path (GET /nope) hits the router's deny terminal — the gateway folds ANY `http.deny` to a fixed
// `403` (it does not read the deny's status field), so an unmatched route surfaces as 403, not 404.
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
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"hello from a wasm handler" } },
    { http = { method = "GET", path = "/nope" },
      expect = { status = 403 } },
  ],
}
