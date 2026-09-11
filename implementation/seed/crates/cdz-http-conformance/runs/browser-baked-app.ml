// SCENARIO (browser-outpost S0.1): the SHARED production baked root router serves an HTML page on a route.
// v-hivemind (baked-router owner) blessed adding GET /app -> http-page to root-router-baked. This proves the
// real deployed router path — not just an isolated standalone router — returns text/html on a route
// (operator: "a router that returns HTML and JavaScript on certain routes").
//
// root-router-baked bakes the http-page handler's real ProgramHash (deploy-templated at build time, like
// http-hello/http-echo); the gateway drives the router per request and it dispatches GET /app to http-page,
// whose 200 text/html response folds back through the router's on-response to the socket. The gateway forwards
// the content-type header verbatim (dispatch fold-back preserves headers, cf. drive-root-router.ml).
{
  config = {
    root-router = "root-router-baked",
    programs = [
      { name = "root-router-baked", program = "root-router-baked" },
      { name = "http-page",         program = "http-page" },
    ],
  },
  requests = [
    { http = { method = "GET", path = "/app" },
      expect = { status = 200,
                 body-contains = "browser outpost",
                 headers = [ { name = "content-type", value = "text/html" } ] } },
  ],
}
