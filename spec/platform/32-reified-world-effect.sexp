; Platform-conformance suite — the SCHEMA-HASH REIFY boundary: a reducer that PERFORMS a target-free async
; world-effect has that perform REIFIED by rcdzc into its returned effect-list as a 3-field effect-request
; record {correlation, kind, payload} (NO target column), which crosses the A1 bytes fold boundary and is
; observed by the platform runner. This is the phase-1a shape of the schema-hash effect-identity workstream:
; a reducer no longer hand-builds an effect-request record; it DECLARES an effect and PERFORMS a typed op
; ((host (Beat) (list (Beat.beat p)))), and the compiler reifies the perform to the wire record — rcdzc bakes
; the effect's schema identity, the guest writes no hash. Because Beat is NOT a world import (kv is the sole
; import), is_world_import_op(Beat.beat)=false, so the fork REIFIES it (vs a synchronous kv HostCall consumed
; inline). The runner surfaces the reified effect on its observed-effect stream under family effect/Beat (the
; userspace_effect_family_kind naming; phase-2 re-keys this string family to a schema-hash). Pins that the
; reify producer + the guest->kernel parse_effect_request boundary + the runner's observation agree end-to-end.
;
; NOTE this is the TARGET-FREE reify (Beat.beat: the Bytes payload IS the one arg, no @resource, no dest). A
; target-HAVING emit (Emit.send(@resource dest, body)) rides the schema-hash phase-2 window (kernel @resource
; extraction); this case is landable now, independent of that window, and pins the reify mechanism the whole
; effect-list flip is built on. A status-only assertion would be near-vacuous (the reify could silently drop
; and the session would still be quiescent); asserting the OBSERVED effect via (expect-effects ..) is what
; pins the reify boundary non-vacuously.

(platform-case "a reducer performs a target-free async world-effect; rcdzc REIFIES the perform into the observed effect-list"
  (doc "The reify boundary end-to-end: the reducer declares (effect Beat (op beat (-> Bytes Unit))) — Beat is
        NOT a world import (kv is), so a performed Beat.beat is an async world-effect. On its message kickoff
        the reducer host-delegates the perform ((host (Beat) (list (Beat.beat \"tick\")))); rcdzc reifies it
        into the returned effect-list as the 3-field effect-request record (correlation None, kind effect/Beat,
        payload the tick bytes) — NO target column, since a target-free effect has no destination. The runner
        observes it on the effect stream. Asserts the observed effect (from s, family effect/Beat) AND the
        session quiescent. The apply RESULT-TYPE annotation is the 3-field (correlation, kind, payload) record
        the reify emits (NOT the pre-reify 4-field-with-target shape) — the annotation flip is part of the
        reify transform. Pins reify producer + parse_effect_request ingest + runner observation agree.")
  (session "s" (reducer
    (do
      (effect kv (op put (-> Bytes Bytes Unit)))
      (effect Beat (op beat (-> Bytes Unit)))
      (type ReplyPayload (Inline Bytes) (Blob Bytes))
      (type Outcome (Ok ReplyPayload) (Err (Record (: message Bytes) (: retryable Bool))) (TimedOut))
      (type ChildOutcome (Success ReplyPayload) (Failure Bytes))
      (def (apply (: e (Record (: content-type (Record (: family String) (: version (UInt 32)))) (: payload (Option Bytes)) (: resumes (Option Bytes)) (: outcome (Option Outcome)) (: child-completed (Option (Record (: child Bytes) (: outcome ChildOutcome)))))))
        (: (if (= (. (. e content-type) family) "message")
             (host (Beat) (list (Beat.beat ((. String to-bytes) "tick"))))
             (list))
           (List (Record (: correlation (Option Bytes)) (: kind String) (: payload (Option Bytes))))))
      (export apply))))
  (kickoff "s" (inbound "message" (: unit Unit)))
  (expect-effects (effect (from "s") (family "effect/Beat")))
  (end-state "s" (status quiescent)))
