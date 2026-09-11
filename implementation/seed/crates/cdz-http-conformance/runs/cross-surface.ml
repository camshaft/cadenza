// SCENARIO: /parse is SURFACE-AGNOSTIC — an ml program and its sexpr form parse to the SAME canonical binary-AST
// (v-hivemind's cross-surface golden, validated live). Binary-AST is the canonical form, so both surfaces of the
// same program yield the same content hash.
//
// reducer-guest-parse is the root. Step 1: POST the ml source (default Content-Type -> ml parser) -> 200 + the
// ast-hash; CAPTURE that body as "ast". Step 2: POST the SAME program's sexpr form (Content-Type
// application/sexpr -> sexpr parser) -> 200 + its ast-hash; assert it EQUALS the captured "ast". A robust
// structural check that pins NO machine-specific hash value (the two surfaces must agree, whatever the hash is).
// Also proves Content-Type routing (ml vs sexpr parser) + both publish a resolvable AST.
{
  config = {
    root-router = "reducer-guest-parse",
    programs = [
      { name = "reducer-guest-parse",  program = "reducer-guest-parse" },
      { name = "reducer-guest-ml",     program = "reducer-guest-ml" },
      { name = "reducer-guest-sexpr",  program = "reducer-guest-sexpr" },
    ],
  },
  requests = [
    // ml surface (default) -> capture the ast-hash.
    { http = { method = "POST", path = "/",
               body = b"def main() -> Int64 = 42\nexport { main }" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "ast" } },
    // sexpr surface of the SAME program -> must yield the SAME ast-hash.
    { http = { method = "POST", path = "/",
               headers = [ { name = "content-type", value = "application/sexpr" } ],
               body = b"(do\n  (def (main) (: 42 Int64))\n\n  (export main))" },
      expect = { status = 200, resolves-in-cas = true, body-equals-capture = "ast" } },
  ],
}
