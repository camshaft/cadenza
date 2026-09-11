// SCENARIO: a handler's effect DEADLINE fires (§1/§2) — the gateway-enforced per-request timeout, composed e2e.
//
// http-deadline is the root router: GET / makes it emit ONE effect with deadlineNanos = 100ms + Continue,
// awaiting the answer. NO prime-reply is set, so the control server never answers → the gateway's resolver
// (which arms the deadline BEFORE classifying the effect — a REAL gateway timeout, not a harness injection)
// injects Err(Error.Timeout) at the deadline → the handler folds it → 504 body "TIMEOUT".
//
// Immune to the control-link tagged-vs-bare frame bug (control-send.ml): the deadline is LOCAL-inject, so it
// fires whether or not the ControlUp reaches the mock. retry-until-match is unnecessary — the drive BLOCKS
// until the deadline fires (~100ms), well within the driver's request budget.
{
  config = {
    root-router = "http-deadline",
    programs = [ { name = "http-deadline", program = "http-deadline" } ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 504, body-contains = "TIMEOUT" } },
  ],
}
