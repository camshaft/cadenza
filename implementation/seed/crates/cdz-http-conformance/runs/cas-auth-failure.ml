// SCENARIO (§8-"grow" — auth failure): a handler's blobs.put with a MISMATCHED CAS write credential surfaces as
// a clean floor, never a hang/crash. BEHAVIOR (as of #8880, which made the state/blobs host imports FALLIBLE +
// TRAP on a backend error — closing the previously-flagged infallible-swallow gap): the guest blobs.put is
// DENIED by the CAS (401, wrong Bearer); the now-fallible blobs host import TRAPS the guest reducer on that
// backend error, so the reducer never reaches a terminal Break. The gateway drives a program that produces NO
// response → it floors 500 "no response from program". (BEFORE #8880 the infallible blobs.put SWALLOWED the 401
// and returned a bogus hash, surfacing DOWNSTREAM as an unresolvable CasRef → 502 "absent from CAS"; #8880
// replaced that silent-swallow with an explicit trap. NOTE: the trap→500 floor semantic — vs a more specific
// 502/bad-gateway for a backend-caused trap — is confirmed-pending with v-gateway-rewrite; this pins the current
// observable behavior.)
//
// body-nonce keeps the (attempted) published body unique per run so the outcome is deterministic regardless of
// CAS contents. config.cas-write-credential ships a WRONG credential (the harness CAS expects the fixed seed),
// so the blobs.put is denied → traps → no program response → 500. (The `casref` scenario is the happy-path twin
// with the correct credential → 200; it stays green — only the DENIED path changed under #8880.)
{
  config = {
    root-router = "http-casref-echo",
    programs = [ { name = "http-casref-echo", program = "http-casref-echo" } ],
    cas-write-credential = "wrong-credential",
  },
  requests = [
    { http = { method = "POST", path = "/", body-nonce = true },
      expect = { status = 500, body-contains = "no response from program" } },
  ],
}
