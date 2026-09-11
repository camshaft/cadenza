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
    // The AOT bundle-file routes serve each file's bytes via blobs.get(ProgramHash). In THIS harness the files
    // are not seeded and the hashes are unsubstituted placeholder markers, so blobs.get misses -> 404 (pinning
    // the route exists + the blobs-miss path). The LIVE deploy seeds the 8 bundle files + substitutes the 7
    // markers, so this serves 200 text/javascript there (and the .core.wasm routes serve application/wasm).
    { http = { method = "GET", path = "/browserOutpostApp/browserOutpostApp.js" },
      expect = { status = 404, body-contains = "not found" } },
    // An unmatched route denies 404 (the router's inline no-route branch).
    { http = { method = "GET", path = "/nope" },
      expect = { status = 404, body-contains = "not found" } },
    // A page/asset router serves GET only: a non-GET to a known route is 405, not the page.
    { http = { method = "POST", path = "/" },
      expect = { status = 405, body-contains = "method not allowed" } },
  ],
}
