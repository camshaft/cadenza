// SCENARIO (browser-outpost S1a): a router returns HTML and JavaScript on certain routes — the operator's
// exact ask (2026-09-11: "build a router that returns HTML and JavaScript on certain routes"). The
// browser-outpost router decodes each request, branches on the path, and answers INLINE:
//   GET /        -> 200 text/html        the page (mount point + <script type=module src=/app.js>)
//   GET /app.js  -> 200 text/javascript  the minimal JS bootstrap
//   GET /nope    -> 404                   no route
// Driven as the DIRECT root router, so this stands alone in the browser-outpost territory (no shared baked
// router, no dispatch, no flake templating). The dumb gateway forwards each content-type header verbatim
// (ZERO gateway change), so a browser would fetch the HTML document and load the ES module. This also
// exercises the request forward codec (Value.decode : Request) — the path the #8770 regression broke and
// #8775 fixed — since the router branches on the decoded path.
{
  config = {
    root-router = "browser-outpost",
    programs = [ { name = "browser-outpost", program = "browser-outpost" } ],
  },
  requests = [
    // The HTML route: 200 text/html, and the page references the out-of-line JS module (keeping the HTML tiny).
    { http = { method = "GET", path = "/" },
      expect = { status = 200,
                 body-contains = "src='/app.js'",
                 headers = [ { name = "content-type", value = "text/html" } ] } },
    // The JavaScript route: 200 text/javascript, and the bootstrap module body round-trips to the socket.
    { http = { method = "GET", path = "/app.js" },
      expect = { status = 200,
                 body-contains = "bootstrap module loaded",
                 headers = [ { name = "content-type", value = "text/javascript" } ] } },
    // An unmatched route denies 404 (the router's inline no-route branch).
    { http = { method = "GET", path = "/nope" },
      expect = { status = 404, body-contains = "not found" } },
    // A page/asset router serves GET only: a non-GET to a known route is 405, not the page.
    { http = { method = "POST", path = "/" },
      expect = { status = 405, body-contains = "method not allowed" } },
  ],
}
