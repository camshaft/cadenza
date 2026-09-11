// SCENARIO: /parse returns ACTIONABLE parse diagnostics (operator directive: "return actionable diagnostics",
// not a bare 4xx). A syntactically BROKEN source → 400 whose body carries the parser's diagnostic. v-hivemind's
// demo-compile asserts this same case live, so the conformance gate and the demo stay aligned.
//
// reducer-guest-parse is the root; POST a malformed program (default Content-Type → ml parser). The parse guest
// runs the ml parser, gets a non-empty diagnostics list, and answers 400 + render-parse-diags(...). We assert the
// status + a SUBSTRING of the diagnostic ("expected"), not the exact wording — robust to diagnostic-text tweaks.
//
// ⚠️ Test-first acceptance: this reads the delivered http-request, so it is RED under the #8770 forward-codec
// regression (the gateway now emits the http-request ascription-free via finish_value, and the guest's
// `Value.decode : Option(Request)` returns None → 400 "undecodable http-request", which passes the 400 status
// check but fails the "expected" body-contains). It auto-greens once the forward codec is fixed.
{
  config = {
    root-router = "reducer-guest-parse",
    programs = [
      { name = "reducer-guest-parse", program = "reducer-guest-parse" },
      { name = "reducer-guest-ml",    program = "reducer-guest-ml" },
    ],
  },
  requests = [
    // A malformed ml source → 400 with the parse diagnostic (the parser says what it "expected").
    { http = { method = "POST", path = "/",
               body = b"def main( ->" },
      expect = { status = 400, body-contains = "expected" } },
  ],
}
