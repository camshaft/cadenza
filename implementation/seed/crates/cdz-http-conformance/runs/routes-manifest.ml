// SCENARIO: the SELF-DESCRIBING routes manifest (the `/_routes` convention — operator's self-describing-router
// idea, blessed by v-gateway-rewrite as a ZERO-gateway-change userspace ROUTER convention).
//
// The baked root router answers a reserved `GET /_routes` ITSELF (not dispatched) with a 200 whose body is
// `Value.encode(routes())` — its baked `List(Route)` in binary-AST (THE data-exchange format). A deploy tool
// GETs `/_routes`, `Value.decode`s the body as `List(Route)`, edits the table, recompiles + reinstalls the
// router. This pins the convention: GET /_routes -> 200, and the manifest body carries the baked route paths
// (here "/" and "/echo" appear as Str leaves in the encoded value, so body-contains proves the round-trip).
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
    { http = { method = "GET", path = "/_routes" },
      expect = { status = 200, body-contains = "/echo" } },
  ],
}
