// SCENARIO: the /compile route end to end — source → ast-hash → a compiled wasm COMPONENT, persisted in the CAS.
// The artifact-list /compile design (reducer-targets B10c, via v-hivemind): a client parses each module via
// /parse (→ its raw ast-hash), assembles a CompileRoute artifact list (one kind="ast" CasRef per module + a
// kind="entry" Inline naming the entrypoint), and POSTs it; the handler resolves each CasRef via blobs.get,
// hands rcdzc the bytes-only artifacts, and publishes the emitted component — answering 200 + its ProgramHash.
//
// A TWO-PHASE live-swap run (both route guests are single-purpose root-direct handlers, so each phase runs as
// root against the SAME gateway + CAS — the ast /parse publishes persists for /compile to resolve):
//   1. reducer-guest-parse (root): POST the ml source (default → ml parser) → 200 + the raw 33-byte ast-hash;
//      CAPTURE it as "main-ast" + assert it resolves in the CAS (the ast persisted).
//   2. push-root-router → reducer-guest-compile over the control plane (the live-swap; the mock resolves its
//      ProgramHash because it is registered in config.programs).
//   3. POST /compile with a body ASSEMBLED AT SEND TIME by the real cdz-http-compile-request deploy tool from the
//      captured "main-ast" (--ast main=@<hash> --entry main) — so nothing machine-specific is pinned in this
//      spec. retry-until-match polls past the async swap propagation. → 200 + the component's ProgramHash, which
//      RESOLVES in the CAS (a non-empty wasm component the compile actually published). The full parse→compile
//      →publish chain, black-box, against the stock gateway.
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
    // 1. Parse the ml source → capture the ast-hash (and confirm the ast persisted in the CAS).
    { http = { method = "POST", path = "/",
               body = b"def main() -> Int64 = 42\nexport { main }" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "main-ast" } },
    // 2. Live-swap the root router to the compile handler.
    { control = { push-root-router = "reducer-guest-compile" } },
    // 3. Compile: the body is built from the captured ast-hash by the real deploy tool. → the component's
    //    ProgramHash, which must resolve in the CAS to a real wasm component — asserted by the wasm magic
    //    (\x00asm = 00 61 73 6d) prefix of the resolved blob, not just that it is non-empty.
    { http = { method = "POST", path = "/compile",
               compile-request = { asts = [ { name = "main", from-capture = "main-ast" } ],
                                   entry = "main" } },
      expect = { status = 200, resolves-in-cas = true, cas-body-starts-with = b"\x00asm",
                 retry-until-match = true } },
  ],
}
