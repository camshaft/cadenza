/-
`oracle-selftest` — the L0.1 gate witness: a smoke request round-trips through the frame codec and
the declining handler.

It (1) builds a request with one module and one trial, (2) encodes it and decodes it back, asserting
the request survives the wire round-trip, (3) runs `handle`, (4) encodes the response and decodes it
back, and (5) asserts the result is exactly one `Unsupported` verdict with the skeleton reason and no
host calls. Exits non-zero on any mismatch so Lake's test driver (and the Nix smoke check) fail loudly.

This also pins the byte layout: any accidental change to the frame codec that breaks the round-trip
is caught here rather than at the far end of the differential.
-/
import Oracle

open Oracle Oracle.Frame

/-- Fail with a message + non-zero exit. -/
def bail (msg : String) : IO UInt32 := do
  (← IO.getStderr).putStrLn s!"oracle-selftest: FAIL — {msg}"
  return 1

def main : IO UInt32 := do
  let req : Request := {
    modules := #[ByteArray.mk #[0x01, 0x02, 0x03]]
    trials := #[{ entry := "main", args := #[ByteArray.mk #[0x2a]], hostResponses := #[] }]
  }
  -- (2) request wire round-trip
  match decodeRequest (encodeRequest req) with
  | .error e => bail s!"request round-trip decode: {e}"
  | .ok back =>
    if back.modules.size != 1 || back.trials.size != 1 then
      bail s!"request round-trip shape: {back.modules.size} modules, {back.trials.size} trials"
    else if back.trials[0]!.entry != "main" then
      bail s!"request round-trip entry: {back.trials[0]!.entry}"
    else
      -- (3)+(4) run + response wire round-trip
      match decodeResponse (encodeResponse (handle back)) with
      | .error e => bail s!"response round-trip decode: {e}"
      | .ok resp =>
        -- (5) exactly one Unsupported verdict, skeleton reason, no host calls
        if resp.size != 1 then
          bail s!"expected 1 verdict, got {resp.size}"
        else
          let v := resp[0]!
          match v.outcome with
          | .unsupported reason =>
            if reason != skeletonReason then
              bail s!"unexpected reason: {reason}"
            else if v.hostCalls.size != 0 then
              bail s!"expected 0 host calls, got {v.hostCalls.size}"
            else do
              IO.println "oracle-selftest: ok — smoke request round-trips, verdict = Unsupported"
              return 0
          | _ => bail "expected Unsupported outcome"
