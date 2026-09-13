// SCENARIO (counter): the COUNTER browser-app root-router (operator note 746: "returning a browser
// application ... make it a simple counter. Add and subtract. And then it displays the current value ... make
// it look decent with some css"). The counter reducer IS the root router and serves ONE self-contained,
// CSS-styled HTML document whose inline JS runs the counter client-side (add/subtract update the display and
// persist to localStorage). It answers a browser GET / with the page and denies everything else.
//   GET /     -> 200 text/html   the styled single-page counter app (value display + subtract/add buttons + JS)
//   GET /nope -> 404             no other route
//   POST /    -> 405             GET-only
// State is client-side (localStorage), not server-side: v-hivemind verified the live daemon runs the
// root-router as a per-request EPHEMERAL session (fresh in-memory scratch KV per spawn, writes discarded), so a
// server-side counter would never accumulate. Running the count in the tab gives a working, refresh-persistent
// counter with no durable session. This also exercises the request forward codec (Value.decode : Request)
// since the router branches on the decoded method + path.
{
  config = {
    root-router = "counter",
    programs = [ { name = "counter", program = "counter" } ],
  },
  requests = [
    // The app document: 200 text/html, the styled counter shell (title + value display).
    { http = { method = "GET", path = "/" },
      expect = { status = 200,
                 body-contains = "Cadenza Counter",
                 headers = [ { name = "content-type", value = "text/html; charset=utf-8" } ] } },
    // The same document carries the interactive counter JS (localStorage-backed add/subtract) inline — pin
    // that the shipped page is the working app, not a static shell.
    { http = { method = "GET", path = "/" },
      expect = { status = 200, body-contains = "cadenza.counter" } },
    // An unmatched route denies 404 (the router's inline no-route branch).
    { http = { method = "GET", path = "/nope" },
      expect = { status = 404, body-contains = "not found" } },
    // GET-only: a non-GET to a known route is 405, not the page.
    { http = { method = "POST", path = "/" },
      expect = { status = 405, body-contains = "method not allowed" } },
  ],
}
