// SCENARIO (§8-"grow" — auth failure): a handler's blobs.put with a MISMATCHED CAS write credential surfaces as
// a clean floor, never a hang/crash. BEHAVIOR: the guest blobs.put is DENIED by the CAS (401, wrong Bearer). As
// of #8880 the state/blobs host imports are FALLIBLE and TRAP the guest reducer on that backend error (closing
// the previously-flagged infallible-swallow gap), so the reducer never reaches a terminal Break. As of the
// ReducerFault-classification refinement (on #8882), the gateway maps a TOP-LEVEL HostBackend fault (a denied/
// failed upstream state|blobs op) to 502 "upstream backend error" — a dependency/upstream failure — while
// keeping genuine guest faults (trap/panic/malformed step) at 500. This handler's blobs.put is TOP-LEVEL (not
// dispatched), so it hits the top-level 502 mapping. (Supersedes the interim trap→500 pin (#8881) and, before
// #8880, the infallible-swallow → downstream-502 "absent from CAS".)
//
// body-nonce keeps the (attempted) published body unique per run so the outcome is deterministic regardless of
// CAS contents. config.cas-write-credential ships a WRONG credential (the harness CAS expects the fixed seed),
// so the blobs.put is denied → traps → the gateway floors 502 (a top-level HostBackend fault). (The `casref`
// scenario is the happy-path twin with the correct credential → 200; it stays green — only the DENIED path.)
{
  config = {
    root-router = "http-casref-echo",
    programs = [ { name = "http-casref-echo", program = "http-casref-echo" } ],
    cas-write-credential = "wrong-credential",
  },
  requests = [
    { http = { method = "POST", path = "/", body-nonce = true },
      expect = { status = 502, body-contains = "upstream backend error" } },
  ],
}
