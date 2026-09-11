// SCENARIO: a handler's FIRE-AFTER timer wakes it (§2/§6; gateway timer wired in #8748), composed e2e.
//
// http-timer is the root router: GET / makes it arm a fire-after timer (Envelope.FireAfter(100ms)) on the
// kernel timer contract + Continue. The gateway arms a detached timer under the request's continuation token
// and, ~100ms later, wakes the handler with the Fired event AS the response (Ok) → the handler folds it → 200
// "FIRED". A REAL gateway-armed timer (LOCAL-inject, no control-link), pairing with deadline-timeout.ml. The
// drive BLOCKS until the wake fires (~100ms), well within the request budget; the settle-gate robustness
// (#8737) makes an arming-then-waking root router driveable.
{
  config = {
    root-router = "http-timer",
    programs = [ { name = "http-timer", program = "http-timer" } ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body-contains = "FIRED" } },
  ],
}
