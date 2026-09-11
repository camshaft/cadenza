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
    // The HTML route: 200 text/html — the page references the out-of-line JS module + carries the mount/buttons.
    { http = { method = "GET", path = "/" },
      expect = { status = 200,
                 body-contains = "src='/app.js'",
                 headers = [ { name = "content-type", value = "text/html" } ] } },
    // The JavaScript route: 200 text/javascript. Pin the durable bootstrap contract — it mounts into #app via
    // getElementById('app') (the DOM handle the reducer's render effect targets) and instantiates the reducer.
    { http = { method = "GET", path = "/app.js" },
      expect = { status = 200,
                 body-contains = "getElementById('app')",
                 headers = [ { name = "content-type", value = "text/javascript" } ] } },
    // The wasm asset routes serve the reducer/runtime COMPONENT bytes via blobs.get(ProgramHash). In THIS
    // harness the two components are not seeded and the hashes are unsubstituted placeholder markers, so
    // blobs.get misses -> 404 (pinning the route exists + the blobs-miss path). The LIVE deploy substitutes the
    // real ProgramHashes + seeds the components into the CAS, so /reducer.wasm serves 200 application/wasm there.
    { http = { method = "GET", path = "/reducer.wasm" },
      expect = { status = 404, body-contains = "not found" } },
    // An unmatched route denies 404 (the router's inline no-route branch).
    { http = { method = "GET", path = "/nope" },
      expect = { status = 404, body-contains = "not found" } },
    // A page/asset router serves GET only: a non-GET to a known route is 405, not the page.
    { http = { method = "POST", path = "/" },
      expect = { status = 405, body-contains = "method not allowed" } },
  ],
}
