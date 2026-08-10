; Platform-conformance suite — I2: effect-handler SESSIONS (seq359).
;
; A case declares a CALLER reducer that performs an effect + a HANDLER session bound to serve that
; effect family via (serves <family>). The effect is DEFERRED, forwarded to the handler as an
; effect-request/<family> inbound, the handler folds it and emits effect/reply, and the reply settles
; the caller's open effect — the real in-process round-trip (the SAME machinery the OUTPOST federates).
;
; STATUS: these cases grade TODO until binary-AST B2 (design-binary-ast-abi) lands. The current fold
; boundary carries only the closed 6-variant EffectKind enum, so a Cadenza reducer cannot emit a
; REGISTER-BY-STRING effect family (weather/effect/reply) — and UserspaceEffectExecutor refuses every
; built-in family, so NO effect a Cadenza reducer can emit today reaches a handler session. B2 flips
; fold.apply to apply(list<u8>)->list<u8> where an effect-request carries its family as an arbitrary
; STRING, which is the unblock. The xtask grader Todo-gates any case with a (serves ..) binding until
; then (reported to v-agent-harness B2 owner + concierge; concierge ruling: Todo-witness meanwhile).
; When B2 (+ B3 for real-rcdzc reducers) lands, delete the grader's serves→Todo gate and these run for real.
;
; The reducer bodies below are the INTENDED shape (they compile fine); they just can't be DRIVEN to the
; handler until B2. Authoring them now pins the I2 grammar (serves/expect-effects) end-to-end through the
; reader + grade path, and the cases auto-flip Todo→green on B2 with zero grammar/grader change.

(platform-case "a worker performs a `weather` effect served by a handler session (round-trip)"
  (doc "TODO-witness (blocked on binary-AST B2). The worker performs a userspace `weather` effect; the
        runner defers it and forwards it to the `sky` handler session (bound via (serves \"weather\")),
        which folds the request and replies; the reply settles the worker's open effect. This documents
        the intended I2 round-trip: an order-verified (expect-effects) sequence + the handler's end-state.
        Grades Todo until a Cadenza reducer can emit the register-by-string `weather`/`effect/reply`
        families (B2); flips to a real graded run when B2 lands.")
  (session "worker" (reducer
    (do
      (type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))
      (type EffectRequest
        (Mk (Record
          (: kind EffectKind)
          (: target String)
          (: payload (Option Bytes))
          (: correlation (Option Bytes)))))
      (def (apply
             (: ct (Record (: family String) (: version (UInt 32))))
             (: payload (Option Bytes))
             (: resumes (Option Bytes)))
        (: ("list") (List EffectRequest)))
      (export apply))))
  (session "sky" (reducer
    (do
      (type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))
      (type EffectRequest
        (Mk (Record
          (: kind EffectKind)
          (: target String)
          (: payload (Option Bytes))
          (: correlation (Option Bytes)))))
      (effect Kv (op get (-> Bytes (Option Bytes))) (op put (-> Bytes Bytes Unit)))
      (bind Kv "cadenza:agent-kernel/kv")
      (def (apply
             (: ct (Record (: family String) (: version (UInt 32))))
             (: payload (Option Bytes))
             (: resumes (Option Bytes)))
        (: (host (Kv)
             (do
               ((. Kv put) ((. String to-bytes) "served") ((. Bytes of) ("list" ((. UInt8 wrap) 1))))
               ("list")))
           (List EffectRequest)))
      (export apply)))
    (serves "weather"))
  (kickoff "worker" (inbound "message" (: unit Unit)))
  (expect-effects
    (effect (from "worker") (family "weather")))
  (end-state "sky" (kv "served" (: 1 Int64)) (status quiescent)))
