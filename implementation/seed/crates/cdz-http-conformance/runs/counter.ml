// SCENARIO (counter): the COUNTER browser-app root-router (operator note 746: "make it a simple counter.
// Add and subtract. And then it displays the current value ... make it look decent with some css"). The
// counter reducer IS the root router: it decodes each request, branches on the path, holds the count in
// durable `state`, and answers INLINE with a fully-rendered, CSS-styled HTML page:
//   GET /         -> 200 text/html   the styled page showing the current value + ADD / SUBTRACT link-buttons
//   GET /add      -> 200 text/html   n := n + 1, re-render
//   GET /subtract -> 200 text/html   n := n - 1, re-render
//   GET /nope     -> 404             no route
//   POST /        -> 405             GET-only
// Driven as the DIRECT root router (stands alone, no shared baked router / dispatch / flake templating). The
// dumb gateway forwards the content-type verbatim so a browser renders the page and the links navigate. This
// also exercises the request forward codec (Value.decode : Request) since the router branches on the path.
//
// This first slice pins ROUTING + RENDERING + METHOD handling (certainly true regardless of how the harness
// scopes root-router state across requests). The value-INCREMENT assertion (that /add actually bumps the
// displayed number) is a follow-up once the harness's state-persistence semantics are verified empirically.
{
  config = {
    root-router = "counter",
    programs = [ { name = "counter", program = "counter" } ],
  },
  requests = [
    // The page: 200 text/html, the styled counter shell with both link-buttons wired to /add and /subtract.
    { http = { method = "GET", path = "/" },
      expect = { status = 200,
                 body-contains = "Cadenza Counter",
                 headers = [ { name = "content-type", value = "text/html; charset=utf-8" } ] } },
    // The add-button target: folds the count and re-renders the page (still 200 text/html).
    { http = { method = "GET", path = "/add" },
      expect = { status = 200,
                 body-contains = "Cadenza Counter",
                 headers = [ { name = "content-type", value = "text/html; charset=utf-8" } ] } },
    // The subtract-button target: folds the count and re-renders the page.
    { http = { method = "GET", path = "/subtract" },
      expect = { status = 200, body-contains = "Cadenza Counter" } },
    // An unmatched route denies 404 (the router's inline no-route branch).
    { http = { method = "GET", path = "/nope" },
      expect = { status = 404, body-contains = "not found" } },
    // GET-only: a non-GET to a known route is 405, not the page.
    { http = { method = "POST", path = "/" },
      expect = { status = 405, body-contains = "method not allowed" } },
  ],
}
