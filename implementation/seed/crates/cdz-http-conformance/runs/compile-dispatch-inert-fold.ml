// SCENARIO (§8 "grow" — compile-dispatch RED-NEGATIVE, v-hivemind rig-verified): the BIDIRECTIONAL partner of
// compile-dispatch.ml. That scenario proves a compiled router with a real on-response fold DISPATCHES (GET / →
// 200); this proves the inverse — a compiled router with NO fold HANGS. Together they pin the exact bug class
// that cost the fleet a couple ticks (a router that dropped its on-response): compile ok is NOT enough; the fold
// is what makes dispatch return a response.
//
// The router SOURCE is fixtures/root-router-inert-fold.cdz = root-router-baked with EXACTLY its on-response fold
// DELETED (on-response then resolves to reducer-lib's INERT Continue). The flake templates it (same placeholder
// substitution as the positive) + stages it. It /compiles FINE (verified on v-hivemind's rig: component
// 05HxlX9n…) and SERVES the Close path — GET /_routes → 200 (the router answers the manifest ITSELF, no dispatch,
// no fold). But a DISPATCH (GET /) → the child http-hello answers → the inert on-response just Continues, never
// respond-encodes → the caller NEVER gets a response → the request HANGS (the `times-out` assertion; the driver
// waits a bounded budget and PASSES iff no response arrives).
//
// Flow mirrors compile-dispatch: parse the RED router + the SAME 7-lib closure, swap to the compile handler,
// /compile (entry = the RED router) → capture the component, push-root-router from-capture. Then: GET /_routes →
// 200 (retry-until-match, past the swap propagation — proves it compiled AND serves) ; GET / → times-out (the
// missing fold hangs). The /_routes 200 is what makes the hang SPECIFIC to the dispatch fold, not a dead router.
{
  config = {
    root-router = "reducer-guest-parse",
    programs = [
      { name = "reducer-guest-parse",   program = "reducer-guest-parse" },
      { name = "reducer-guest-ml",      program = "reducer-guest-ml" },
      { name = "reducer-guest-rcdzc",   program = "reducer-guest-rcdzc" },
      { name = "reducer-guest-compile", program = "reducer-guest-compile" },
      { name = "http-hello",            program = "http-hello" },
      { name = "http-echo",             program = "http-echo" },
    ],
  },
  requests = [
    // 1. Parse the RED router + its 7-lib closure → capture each ast-hash.
    { http = { method = "POST", path = "/", body-source = "root-router-inert-fold" },
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
    // 2. Swap to the compile handler.
    { control = { push-root-router = "reducer-guest-compile" } },
    // 3. Compile the RED router → 200 + a real wasm component (it DOES compile); capture the component hash.
    { http = { method = "POST", path = "/compile",
               compile-request = {
                 asts = [
                   { name = "root-router-inert-fold", from-capture = "router-ast" },
                   { name = "http-lib",      from-capture = "http-lib-ast" },
                   { name = "reducer-lib",   from-capture = "reducer-lib-ast" },
                   { name = "http-response", from-capture = "http-response-ast" },
                   { name = "http-deny",     from-capture = "http-deny-ast" },
                   { name = "http-dispatch", from-capture = "http-dispatch-ast" },
                   { name = "control-send",  from-capture = "control-send-ast" },
                   { name = "contract-id",   from-capture = "contract-id-ast" },
                 ],
                 entry = "root-router-inert-fold",
                 wit-world = { name = "reducer-world", from-capture = "reducer-world" },
               } },
      expect = { status = 200, resolves-in-cas = true, cas-body-starts-with = b"\x00asm",
                 capture-body-as = "compiled-red", retry-until-match = true } },
    // 4. Install the RED router as root.
    { control = { push-root-router = { from-capture = "compiled-red" } } },
    // 5. GET /_routes → 200: the router answers the manifest ITSELF (Close terminal, no fold), so it COMPILED and
    //    SERVES. retry-until-match polls past the root-swap propagation — and confirms the new root is live before
    //    the hang probe, so the hang below is SPECIFICALLY the dispatch fold, not an un-swapped / dead router.
    { http = { method = "GET", path = "/_routes" },
      expect = { status = 200, body-contains = "/echo", retry-until-match = true } },
    // 6. GET / → DISPATCH to http-hello → the inert on-response never folds the answer back → the request HANGS.
    //    `times-out` sends it and PASSES iff no response arrives within the probe budget (the RED-negative).
    { http = { method = "GET", path = "/" },
      expect = { times-out = true } },
  ],
}
