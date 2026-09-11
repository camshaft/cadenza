// SCENARIO (browser-outpost S0): a cadenza reducer serves an HTML PAGE through the real gateway — the SERVER
// side of the browser outpost (operator 2026-09-11: "a router that returns HTML and JavaScript on certain
// routes"). This is the foundational delivery slice: prove that a content-addressed wasm handler can answer
// `200 text/html` with a full HTML document + minimal inline JS bootstrap, and that the dumb gateway forwards
// the `content-type: text/html` header verbatim (ZERO gateway change) so a real browser would render it.
//
// http-page is the DIRECT root router (like http-echo in echo-direct.ml) — it serves the page for any request
// and does not inspect it, so this stands alone in the browser-outpost territory without touching the shared
// baked root router. Later slices ship a cadenza reducer to the browser (S1, jco) and wire a browser-API-as-
// effect into the reducer fold (S2). Routing an HTML page on a specific baked-router route is a follow-on
// coordinated with the router owner.
{
  config = {
    root-router = "http-page",
    programs = [ { name = "http-page", program = "http-page" } ],
  },
  requests = [
    // 200 text/html, and the body carries the page (the JS bootstrap's marker string proves the whole
    // document round-tripped from the handler through the gateway to the socket).
    { http = { method = "GET", path = "/" },
      expect = { status = 200,
                 body-contains = "browser outpost",
                 headers = [ { name = "content-type", value = "text/html" } ] } },
  ],
}
