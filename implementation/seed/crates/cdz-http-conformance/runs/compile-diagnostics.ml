// SCENARIO: /compile returns ACTIONABLE compile diagnostics (operator directive: "return actionable
// diagnostics") — a program that PARSES cleanly but fails to COMPILE surfaces at /compile as 422 with the CDZ
// diagnostic code / message in the body, NOT a bare 4xx. v-hivemind's demo-compile asserts these same two cases
// live, so the conformance gate and the demo stay aligned.
//
// Two compile-error programs, both syntactically valid (so /parse succeeds + yields an ast-hash), each failing at
// the COMPILE step for a different reason:
//   - `def main() -> Int = 42 / export { main }`  → 422, body contains "CDZ0203" (Int is a width constructor;
//     the actionable fix is Int64). A TYPE error, not a parse error.
//   - `def main() -> Int64 = 42` (no export)      → 422, body contains "nothing is public" (no public surface).
// Both share the two-phase live-swap of runs/compile.ml: parse each (while reducer-guest-parse is root) to capture
// its ast-hash, then push-root-router → reducer-guest-compile and POST /compile the tool-built bodies. Assert
// SUBSTRINGS (robust to diagnostic wording tweaks), and NO resolves-in-cas (a 422 publishes no component).
//
// ⚠️ Test-first acceptance: every step reads the delivered http-request, so this is RED under the #8770
// forward-codec regression (ascription-free http-request → guest `Value.decode : Option(Request)` = None → 400
// "undecodable http-request"). It auto-greens once the forward codec is fixed.
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
    // Parse both compile-erroring programs (they parse fine) → capture each ast-hash while parse is root.
    { http = { method = "POST", path = "/",
               body = b"def main() -> Int = 42\nexport { main }" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "int-ast" } },
    { http = { method = "POST", path = "/",
               body = b"def main() -> Int64 = 42" },
      expect = { status = 200, resolves-in-cas = true, capture-body-as = "noexport-ast" } },
    // Live-swap the root router to the compile handler.
    { control = { push-root-router = "reducer-guest-compile" } },
    // Compile the Int-width-ctor program → 422 with the CDZ0203 code (retry past the async swap).
    { http = { method = "POST", path = "/compile",
               compile-request = { asts = [ { name = "main", from-capture = "int-ast" } ],
                                   entry = "main" } },
      expect = { status = 422, body-contains = "CDZ0203", retry-until-match = true } },
    // Compile the no-export program → 422 "nothing is public".
    { http = { method = "POST", path = "/compile",
               compile-request = { asts = [ { name = "main", from-capture = "noexport-ast" } ],
                                   entry = "main" } },
      expect = { status = 422, body-contains = "nothing is public" } },
  ],
}
