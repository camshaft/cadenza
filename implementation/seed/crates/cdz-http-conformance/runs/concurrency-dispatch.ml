// SCENARIO (§8-"grow" — concurrency through DISPATCH): the concurrency.ml companion. That fires 25 concurrent
// requests at a DIRECT handler (http-hello as root); this fires 25 concurrent requests through the BAKED ROUTER,
// so each one DISPATCHES to a child handler — exercising concurrent child SPAWNS + the per-request dispatch
// session/mailbox handling under load, a distinct path from the direct case. A race in concurrent dispatch
// (shared spawn state, mailbox reuse, correlation mix-up across simultaneous children) would fail some responses;
// concurrency.ml (no dispatch) can't catch that.
//
// root-router-baked routes GET / → http-hello (its baked, deploy-templated handler hash). `concurrency = 25`
// fires 25 copies of GET / at once; EVERY response must be the dispatched handler's 200 "hello…", folded back
// through the router's on-response — proving 25 simultaneous route→spawn→fold round-trips stay isolated + correct.
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
    { http = { method = "GET", path = "/", concurrency = 25 },
      expect = { status = 200, body = b"hello from a wasm handler" } },
  ],
}
