// SCENARIO: the /PARSE HTTP route (reducer-targets B10) — POST ml source, parse it to an AST, publish the AST
// to the CAS, answer 200 with the AST's ProgramHash — verified end to end (source -> ast-hash -> RESOLVES).
//
// reducer-guest-parse is the root program (root-direct, no router — v-hivemind's rig shape). GET/POST / drives
// it with the http-request; it decodes the request, `run`s the ml parser (delegated by baked ProgramHash — a
// host effect via the gateway's in-process ReducerGraph), `blobs.put`s the resulting AST (a host effect backed
// by the gateway's shared write-capable CAS — needs the mock's shipped write credential), and answers 200 with
// the AST's RAW 33-byte ProgramHash. The `resolves-in-cas` assertion base62-encodes that body-hash and GETs it
// from the CAS — proving the parse published (blobs.put persisted) + the content-addressed round-trip.
//
// Seeds the external reducer-guest-{parse,ml} (SAME-eval build so parse's baked ml hash matches the seeded ml).
// The first e2e exercising host `run` + `blobs.put` (the CAS write path validated by casref.ml) end to end.
{
  config = {
    root-router = "reducer-guest-parse",
    programs = [
      { name = "reducer-guest-parse", program = "reducer-guest-parse" },
      { name = "reducer-guest-ml",    program = "reducer-guest-ml" },
    ],
  },
  requests = [
    { http = { method = "POST", path = "/",
               headers = [ { name = "content-type", value = "text/plain" } ],
               body = b"def main() -> Int64 = 42\nexport { main }" },
      expect = { status = 200, resolves-in-cas = true } },
  ],
}
