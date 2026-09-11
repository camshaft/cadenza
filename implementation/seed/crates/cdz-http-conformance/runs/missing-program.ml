// SCENARIO (design §8 #9): a named-but-ABSENT program → the gateway floors GRACEFULLY (no hang, no crash). The
// http-outpost's dumb gateway fetches + composes a program from the CAS by hash; when a dispatch names a hash
// that isn't in the store, spawn must fail cleanly (an injected Err answer), NOT wedge the request or panic.
//
// http-dispatch-absent is the direct root router: on GET / it emits an http.dispatch to a deliberately-absent
// 33-byte hash and awaits the answer. The gateway can't fetch that hash, injects Err, and the handler folds it
// into a 502 deny. Asserting a clean 502 (within the request timeout) proves the graceful-floor path — a hang
// would instead surface as the driver's request timeout (a RED this scenario would catch).
{
  config = {
    root-router = "http-dispatch-absent",
    programs = [ { name = "http-dispatch-absent", program = "http-dispatch-absent" } ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 502 } },
  ],
}
