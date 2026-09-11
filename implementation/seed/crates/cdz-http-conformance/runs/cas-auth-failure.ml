// SCENARIO (§8-"grow" — auth failure): a handler's blobs.put with a MISMATCHED CAS write credential surfaces as
// a graceful downstream floor (502), never a hang/crash. ROOT CAUSE (v-gateway-rewrite trace): the guest
// blobs.put WIT import is INFALLIBLE-shaped, so the host SWALLOWS the CAS 401 (wrong Bearer) and returns the
// locally-computed content hash as if the write succeeded. So the write's failure is invisible to the guest; it
// surfaces DOWNSTREAM: the handler closes http.response-cas{that hash}, the gateway CAS-fetches it, and — since
// the (denied) write never persisted the blob — the fetch MISSES → 502. (A latent platform gap: a fallible
// blobs.put WIT would let a guest handle its own write failure; flagged to concierge. This scenario pins the
// observable downstream 502.)
//
// To make the 502 DETERMINISTIC regardless of what's already in the CAS, the published body must be UNIQUE per
// run (a fresh hash never pre-present): http-casref-echo publishes msg.payload (the raw encoded request), and
// body-nonce makes that request body unique per run. config.cas-write-credential ships a WRONG credential (the
// harness CAS expects the fixed seed credential), so the blobs.put is denied → swallowed → fresh hash absent →
// gateway cas.get miss → 502. (The `casref` scenario is the happy-path twin with the correct credential → 200.)
{
  config = {
    root-router = "http-casref-echo",
    programs = [ { name = "http-casref-echo", program = "http-casref-echo" } ],
    cas-write-credential = "wrong-credential",
  },
  requests = [
    { http = { method = "POST", path = "/", body-nonce = true },
      expect = { status = 502, body-contains = "absent from CAS" } },
  ],
}
