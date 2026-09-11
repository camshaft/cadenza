// SCENARIO (§8 "grow" — compile→root→DISPATCH, v-hivemind recipe): build a dispatching ROUTER via the /compile
// endpoint AT RUNTIME, root it, and DISPATCH a request through it — closing the compile-ok-but-dispatch-hangs gap.
// reducer-world-compile.ml proves compile→install→serve for a DIRECT handler (http-hello answers itself); this
// proves the harder path: a compiled ROUTER whose on-response FOLD (Ok => respond-encoded, Err => deny 502) routes
// to SEPARATELY-seeded handlers and folds their answer back to the socket. That fold is the exact thing the bug
// class (compile ok, but dispatch hangs / drops the fold) would break; a 200 with the handler's body proves it.
//
// The router SOURCE is root-router-baked's — the flake stages a DEPLOY-TEMPLATED copy (its placeholder handler
// markers substituted with the REAL escaped ProgramHashes of the seeded http-hello / http-echo / http-page, exactly
// like the baked component's build-time templating), so the compiled router dispatches by hash to the seeded
// handlers. Its 8-module closure = the SAME modules reducer-world-compile parses, with the router replacing
// http-hello as the entry: router + http-lib, reducer-lib, http-response, http-deny, http-dispatch, control-send,
// contract-id. http-hello (GET /) and http-echo (POST /echo) are SEEDED as programs so the router can fetch them.
//
// Flow (two-phase like reducer-world-compile): while reducer-guest-parse is root, /parse the router + its 7-lib
// closure (capture each ast-hash); the driver has seeded reducer-world.bin + exposed its hash as "reducer-world".
// Live-swap to reducer-guest-compile, /compile the artifact list (entry = the router) → a real wasm component;
// CAPTURE its ProgramHash (never pinned — /compile is deterministic per v-hivemind, but the hash depends on the
// staged source + toolchain, so capture-don't-hardcode). push-root-router FROM that capture (#8811), then dispatch:
// GET / → 200 "hello…" (router folds http-hello's response), GET /nope → 404 "not found" (router deny terminal).
{
  config = {
    root-router = "reducer-guest-parse",
    programs = [
      { name = "reducer-guest-parse",   program = "reducer-guest-parse" },
      { name = "reducer-guest-ml",      program = "reducer-guest-ml" },
      { name = "reducer-guest-rcdzc",   program = "reducer-guest-rcdzc" },
      { name = "reducer-guest-compile", program = "reducer-guest-compile" },
      // The seeded DISPATCH TARGETS the templated router routes to (by their real ProgramHashes).
      { name = "http-hello",            program = "http-hello" },
      { name = "http-echo",             program = "http-echo" },
    ],
  },
  requests = [
    // 1. Parse the ROUTER + its full lib/contract closure (each a staged in-tree source) → capture each ast-hash.
    { http = { method = "POST", path = "/", body-source = "root-router-baked" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "router-ast" } },
    { http = { method = "POST", path = "/", body-source = "http-lib" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "http-lib-ast" } },
    { http = { method = "POST", path = "/", body-source = "reducer-lib" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "reducer-lib-ast" } },
    { http = { method = "POST", path = "/", body-source = "http-response" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "http-response-ast" } },
    { http = { method = "POST", path = "/", body-source = "http-deny" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "http-deny-ast" } },
    { http = { method = "POST", path = "/", body-source = "http-dispatch" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "http-dispatch-ast" } },
    { http = { method = "POST", path = "/", body-source = "control-send" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "control-send-ast" } },
    { http = { method = "POST", path = "/", body-source = "contract-id" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "contract-id-ast" } },
    // 2. Live-swap the root router to the compile handler.
    { control = { push-root-router = "reducer-guest-compile" } },
    // 3. Compile the router + its closure + the reducer-world wit-world → 200 + a real wasm component; CAPTURE the
    //    component ProgramHash for the install step.
    { http = { method = "POST", path = "/compile",
               compile-request = {
                 asts = [
                   { name = "root-router-baked", from-capture = "router-ast" },
                   { name = "http-lib",      from-capture = "http-lib-ast" },
                   { name = "reducer-lib",   from-capture = "reducer-lib-ast" },
                   { name = "http-response", from-capture = "http-response-ast" },
                   { name = "http-deny",     from-capture = "http-deny-ast" },
                   { name = "http-dispatch", from-capture = "http-dispatch-ast" },
                   { name = "control-send",  from-capture = "control-send-ast" },
                   { name = "contract-id",   from-capture = "contract-id-ast" },
                 ],
                 entry = "root-router-baked",
                 wit-world = { name = "reducer-world", from-capture = "reducer-world" },
               } },
      expect = { status = 200, resolves-in-cas = true, cas-body-starts-with = b"\x00asm",
                 capture-body-as = "compiled-router", retry-until-match = true } },
    // 4. INSTALL the freshly-compiled ROUTER as root (rooting a RUNTIME-CAPTURED ProgramHash), then DISPATCH through
    //    it: GET / → the router routes to http-hello and folds its 200 back (compile-ok-AND-dispatch-works).
    { control = { push-root-router = { from-capture = "compiled-router" } } },
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body-contains = "hello from a wasm handler", retry-until-match = true } },
    // 5. An unmatched path exercises the router's DENY terminal through the compiled router (not a gateway floor).
    { http = { method = "GET", path = "/nope" },
      expect = { status = 404, body-contains = "not found" } },
  ],
}
