// SCENARIO: /compile a real reducer-world GUEST (a handler) into a wasm component — the ADVANCED /compile path
// (the flake notes the reducer-world compile path was UNEXERCISED e2e). Unlike compile.ml (a PLAIN program, one
// --ast, no world), a reducer-world guest that does anything real needs (a) its lib/contract CLOSURE as extra
// --ast modules and (b) a kind="wit-world" artifact (the reducer-world binary that TYPES its on-message
// boundary + links its run/blobs imports). v-hivemind's deploy-router proved this recipe live.
//
// The guest is conformance http-hello (a trivial respond("hello") handler — a plain respond needs no dispatch,
// ideal for a compile-produces-a-component assertion). Its 8-module closure (v-hivemind): the guest + http-lib,
// reducer-lib, http-response, http-deny, http-dispatch, control-send, contract-id. Each is a real in-tree
// source the nix rig STAGES; we POST each to /parse (body-source, so no source text is embedded/drifted here)
// to get its ast-hash, then /compile the artifact list + the seeded reducer-world wit-world.
//
// Flow (two-phase like compile.ml): while reducer-guest-parse is root, /parse all 8 modules (capture each
// ast-hash); the driver has already seeded reducer-world.bin into the CAS + exposed its hash as the capture
// "reducer-world". Then live-swap to reducer-guest-compile and POST /compile the full CompileRoute (8 kind="ast"
// CasRefs + kind="wit-world" CasRef + kind="entry" http-hello) → 200 + the component ProgramHash, which resolves
// in the CAS to a real wasm component (\x00asm magic). Nothing machine-specific is pinned: every hash is a LIVE
// capture (parsed or seeded at runtime).
{
  config = {
    root-router = "reducer-guest-parse",
    programs = [
      { name = "reducer-guest-parse",   program = "reducer-guest-parse" },
      { name = "reducer-guest-ml",      program = "reducer-guest-ml" },
      { name = "reducer-guest-rcdzc",   program = "reducer-guest-rcdzc" },
      { name = "reducer-guest-compile", program = "reducer-guest-compile" },
    ],
  },
  requests = [
    // 1. Parse the guest + its full lib/contract closure (each a staged in-tree source) → capture each ast-hash.
    { http = { method = "POST", path = "/", body-source = "http-hello" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "http-hello-ast" } },
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
    // 3. Compile the guest + its closure + the reducer-world wit-world → 200 + a real wasm component.
    { http = { method = "POST", path = "/compile",
               compile-request = {
                 asts = [
                   { name = "http-hello",    from-capture = "http-hello-ast" },
                   { name = "http-lib",      from-capture = "http-lib-ast" },
                   { name = "reducer-lib",   from-capture = "reducer-lib-ast" },
                   { name = "http-response", from-capture = "http-response-ast" },
                   { name = "http-deny",     from-capture = "http-deny-ast" },
                   { name = "http-dispatch", from-capture = "http-dispatch-ast" },
                   { name = "control-send",  from-capture = "control-send-ast" },
                   { name = "contract-id",   from-capture = "contract-id-ast" },
                 ],
                 entry = "http-hello",
                 wit-world = { name = "reducer-world", from-capture = "reducer-world" },
               } },
      expect = { status = 200, resolves-in-cas = true, cas-body-starts-with = b"\x00asm",
                 retry-until-match = true } },
  ],
}
