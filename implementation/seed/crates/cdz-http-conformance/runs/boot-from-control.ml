// SCENARIO: the gateway boots from the control server + serves.
//
// The mock control server ships a ControlConfig (pointing at the seeded CAS + the baked root router) on
// connect; the gateway dials control, applies the config, and serves. This is the first end-to-end proof of
// the boot-from-control loop — achievable against v-gateway-rewrite's bootable stub (#8666), which serves 200
// for every request once it has applied the ControlConfig. A 200 here means: gateway dialed control, read +
// applied the ControlConfig, and is serving. (Route-to-handler bodies are asserted once routing is wired —
// see route-to-handler.ml.)
{
  config = {
    root-router = "root-router-baked",
    programs = [
      { name = "root-router-baked", program = "root-router-baked" },
    ],
  },
  requests = [
    { http = { method = "GET", path = "/" },
      expect = { status = 200 } },
  ],
}
