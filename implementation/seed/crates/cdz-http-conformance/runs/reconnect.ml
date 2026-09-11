// SCENARIO (§8-"grow" — reconnect/resilience): a control-link drop must NOT break the data plane. The gateway
// dials the mock's control ws + boots from the ControlConfig; if that connection drops, it must REDIAL
// (exponential backoff, #8741) + recover, so a control blip doesn't take down request serving.
//
// http-hello is the direct root router (ignores the request, always 200s) — a stable request-independent
// responder, so this isolates control-link recovery from any request-codec concern. Flow: baseline GET / → 200;
// a `drop-control` step (new mock admin DropControl → the mock closes the gateway's control ws server-side) forces
// the gateway to see the link drop + redial; then GET / (retry-until-match, polling past any brief redial gap) →
// 200 again. A gateway that crashed / wedged / never recovered on the control drop would fail step 3; a 200
// proves it stayed healthy + serving across the drop (it redials + the mock re-ships the config on reconnect).
{
  config = {
    root-router = "http-hello",
    programs = [ { name = "http-hello", program = "http-hello" } ],
  },
  requests = [
    // 1. Baseline: the booted gateway serves.
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"hello from a wasm handler" } },
    // 2. Drop the gateway's control-ws connection server-side → it must redial.
    { control = { drop-control = true } },
    // 3. Still serving after the drop+redial (poll past the brief reconnect gap).
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body = b"hello from a wasm handler", retry-until-match = true } },
  ],
}
